"""Re-implementation of the MediaTek modem file system server (ccci_fsd).

The stock server is the ``fsd_main`` thread of Android's ``ccci_mdinit``.
The modem keeps its NVRAM (IMEI, calibration, ...) on the AP and accesses
it through the CCCI FS channel with DOS-like file operations.

Wire format (handled by the caller, not by this module)
-------------------------------------------------------
A request is ``u32 op, u32 count, count * {u32 len, data, pad to 4}``.
The reply is ``u32 op | 0xFFFF0000, u32 count, count * {u32 len, data, pad}``.
:meth:`FsServer.handle` takes the request's parameters as raw bytes and
returns the reply parameters. Integers are little-endian 32-bit, path names
are NUL-terminated UTF-16LE DOS paths such as ``Z:\\NVRAM\\NVD_IMEI\\LD0B_001``.
``ret`` is a signed 32-bit result: >= 0 on success (a handle, a count or 0),
otherwise a negative FS error code (FS_FILE_NOT_FOUND = -9, ...).

Operations (request params -> reply params)
-------------------------------------------
======  ==============  =====================================  ==============================
op      name            request                                reply
======  ==============  =====================================  ==============================
0x1001  Open            path, flags                            ret (handle)
0x1002  Seek            handle, offset (s32), whence           ret (new position)
0x1003  Read            handle, length                         ret, nread, data[nread]
0x1004  Write           handle, data, length                   ret, nwritten
0x1005  Close           handle                                 ret
0x1006  CloseAll        -                                      ret
0x1007  CreateDir       path                                   ret
0x1008  RemoveDir       path                                   ret
0x1009  GetFileSize     handle                                 ret, size
0x100A  GetFolderSize   path, flag                             ret (bytes, or clusters if flag==2)
0x100B  Rename          old path, new path                     ret
0x100C  Move            src path, dst path, flag               ret
0x100D  Count           path, flag                             ret (number of entries)
0x100E  GetDiskInfo     (ignored)                              ret, FS_DiskInfo[0x54]
0x100F  Delete          path                                   ret
0x1010  GetAttributes   path                                   ret (attribute bits)
0x1011  OpenHint        path, flags, hint[8]                   ret (handle), hint[8] (echo)
0x1012  FindFirst       pattern path, attr(u8), mask(u8), max  ret (handle), DOSDirEntry[0x34], name
0x1013  FindNext        handle, max                            ret, DOSDirEntry[0x34], name
0x1014  FindClose       handle                                 ret
0x1015  LockFAT         (ignored)                              0
0x1016  (unknown "UK")  (ignored)                              1
0x1017  Shutdown        -                                      (no parameters)
0x1018  XDelete         path, flag                             ret (number of deleted entries)
0x1019  (CDF)           (ignored)                              0
0x101A  GetDrive        type, serial, alt_mask                 ret ('Z' = 0x5A or -2)
0x101B  GetClusterSize  drive letter                           ret
0x101C  (SDF)           (ignored)                              0
0x101D  OTPWrite        dev, offset, data, length              ret  (no OTP here: -12)
0x101E  OTPRead         dev, offset, length                    ret, data[length]
0x101F  OTPQueryLength  dev                                    ret, length
0x1020  OTPLock         dev                                    ret
0x1021  Restore         path                                   ret  (no bin region here: -18)
0x1022  CMPT_Read       path, CmptReadReq                      (done, err), size, nread, data
0x1023  BinRegionAccess (ignored)                              -2
0x1024  CMPT_Write      path, CmptWriteReq, data               (done, err), handle, nwritten
0x1025  GetFileDetail   path                                   ret, {u64 mtime, atime, ctime}
0x1026  CMPT_Read_EX    path, CmptReadReq                      (done, err), size, nread, data,
                                                               handle
other   -               -                                      -2
======  ==============  =====================================  ==============================

"name" in the Find replies is the UTF-16LE file name with '/' turned into
'\\', NUL-terminated, at most ``max`` - 1 characters; it is an empty
parameter when nothing was found.

Drives: Z nvdata/md, X protect_f/md, Y protect_s/md, W firmware,
V nvdata/md_cmn, U mdlpm, T mdota, S nvcfg, R etc_md, Q data_de_md, all
relative to the server root (stock: /mnt/vendor/nvdata/md, ...).
"""

from __future__ import annotations

import errno
import logging
import os
import stat
import struct
import time
from collections.abc import Iterator
from dataclasses import dataclass

log = logging.getLogger("ccci_fs")

# --- FS error codes (MediaTek fs_errcode.h) ---------------------------------
FS_NO_ERROR = 0
FS_ERROR_RESERVED = -1
FS_PARAM_ERROR = -2
FS_DRIVE_NOT_FOUND = -4
FS_TOO_MANY_FILES = -5
FS_NO_MORE_FILES = -6
FS_FILE_NOT_FOUND = -9
FS_INVALID_FILE_HANDLE = -10
FS_UNSUPPORTED_DRIVER_FUNCTION = -12
FS_ACCESS_DENIED = -16
FS_GENERAL_FAILURE = -18
FS_PATH_NOT_FOUND = -19
FS_READ_ONLY_ERROR = -45
FS_PATH_OVER_LEN_ERROR = -49

# --- Open flags ---------------------------------------------------------------
FS_READ_ONLY = 0x100
FS_CREATE = 0x10000
FS_CREATE_ALWAYS = 0x20000
FS_COMMITTED = 0x1000000
FS_NONBLOCK_MODE = 0x10000000
_FSYNC_ON_CLOSE = FS_CREATE | FS_CREATE_ALWAYS | FS_COMMITTED

# --- Attribute bits / search flags / move flags ------------------------------
FS_ATTR_READ_ONLY = 0x01
FS_ATTR_DIR = 0x10
FS_FILE_TYPE = 0x04
FS_DIR_TYPE = 0x08
FS_RECURSIVE_TYPE = 0x10
FS_MOVE_COPY = 0x1
FS_MOVE_KILL = 0x2
FS_MOVE_OVERWRITE = 0x10000

# --- CMPT op bits (opid_map) ----------------------------------------------------
CMPT_OPEN = 0x01
CMPT_GETSIZE = 0x02
CMPT_SEEK = 0x04
CMPT_READ = 0x08
CMPT_CLOSE = 0x10
CMPT_WRITE = 0x20

FS_MAX_HANDLE = 0x80        # handles are table indexes 1..128
PATH_MAX = 0x1000
DOS_DIR_ENTRY_SIZE = 0x34
DISK_INFO_SIZE = 0x54

# Drive letters in stock table order, with the subdirectory used here.
DRIVES = (
    ("Z", "nvdata/md"),        # /mnt/vendor/nvdata/md
    ("X", "protect_f/md"),     # /mnt/vendor/protect_f/md
    ("Y", "protect_s/md"),     # /mnt/vendor/protect_s/md
    ("W", "firmware"),         # /vendor/firmware
    ("V", "nvdata/md_cmn"),    # /mnt/vendor/nvdata/md_cmn
    ("U", "mdlpm"),            # /data/vendor/mdlpm
    ("T", "mdota"),            # /vendor/etc/mdota
    ("S", "nvcfg"),            # /mnt/vendor/nvcfg
    ("R", "etc_md"),           # /vendor/etc/md
    ("Q", "data_de_md"),       # /data/vendor_de/md
)
# Directories FS_Init creates (mode 0770).
INIT_DIRS = ("nvdata", "nvdata/md_cmn", "nvdata/md", "protect_f/md", "protect_s/md")


class FsError(Exception):
    """Internal: carries an FS error code back to the op handler."""

    def __init__(self, code: int):
        super().__init__(code)
        self.code = code


def errno_to_fs(err: int | None) -> int:
    """FS_ErrorConv: map a Linux errno to an FS error code."""
    if err == errno.EMFILE:
        return FS_TOO_MANY_FILES
    if err == errno.EACCES:
        return FS_ACCESS_DENIED
    if err == errno.ENOENT:
        return FS_FILE_NOT_FOUND
    return FS_GENERAL_FAILURE


def pat_match(pattern: bytes, name: bytes) -> bool:
    """PatMatch: case-sensitive '*' / '?' wildcard match."""
    if not pattern:
        return not name
    c = pattern[0]
    if c == ord("*"):
        if pat_match(pattern[1:], name):
            return True
        return bool(name) and pat_match(pattern, name[1:])
    if c == ord("?"):
        return bool(name) and pat_match(pattern[1:], name[1:])
    return bool(name) and name[0] == c and pat_match(pattern[1:], name[1:])


def dos_datetime(t: float) -> int:
    """DOS date/time as packed by FS_EntryLinuxToDos.

    Stock quirks kept: the month field holds tm_mday & 0xF and seconds are
    not halved.
    """
    tm = time.localtime(t)
    year, mday = tm.tm_year - 1900, tm.tm_mday
    v = (tm.tm_sec & 0x1F) | ((tm.tm_min & 0x3F) << 5) | ((tm.tm_hour & 0x1F) << 11)
    v |= year << 25
    v = (v & ~(0xF << 21)) | ((mday & 0xF) << 21)
    v = (v & ~(0x1F << 16)) | ((mday & 0x1F) << 16)
    return (v + 0x60000000) & 0xFFFFFFFF


def _i32(v: int) -> bytes:
    return struct.pack("<I", v & 0xFFFFFFFF)


def _s32(v: int) -> int:
    v &= 0xFFFFFFFF
    return v - (1 << 32) if v & 0x80000000 else v


@dataclass
class _Handle:
    path: bytes                     # host path (for search handles: last match)
    flags: int = 0
    fd: int | None = None           # regular file
    search: Iterator[os.DirEntry] | None = None   # FindFirst/FindNext state
    search_dir: bytes = b""
    pattern: bytes = b""
    attr: int = 0
    attr_mask: int = 0
    dirty: bool = False


class FsServer:
    """CCCI FS server; drive letters map to subdirectories of ``root``.

    ``dsp_name``: stock opens ``W:`` paths as ``<firmware>/dsp_<name>.bin``
    with the name from a CCCI ioctl; give it here to get that behaviour,
    otherwise W: paths map like any other drive.
    ``extra_sync``: beyond stock behaviour, fsync files that were written on
    close and fsync parent directories after namespace changes.
    """

    def __init__(self, root: str, dsp_name: str | None = None, extra_sync: bool = True):
        self.root = os.fsencode(os.path.abspath(root))
        self.dsp_name = dsp_name
        self.extra_sync = extra_sync
        self.bases = {letter: os.path.join(self.root, os.fsencode(sub))
                      for letter, sub in DRIVES}
        self.handles: dict[int, _Handle] = {}
        for d in INIT_DIRS:
            p = os.path.join(self.root, os.fsencode(d))
            try:
                os.makedirs(p, 0o770, exist_ok=True)
            except OSError as e:
                log.error("FS_Init: mkdir %s failed: %s", p, e)
        self._ops = {
            0x1001: self._op_open, 0x1002: self._op_seek, 0x1003: self._op_read,
            0x1004: self._op_write, 0x1005: self._op_close, 0x1006: self._op_close_all,
            0x1007: self._op_create_dir, 0x1008: self._op_remove_dir,
            0x1009: self._op_get_file_size, 0x100A: self._op_get_folder_size,
            0x100B: self._op_rename, 0x100C: self._op_move, 0x100D: self._op_count,
            0x100E: self._op_get_disk_info, 0x100F: self._op_delete,
            0x1010: self._op_get_attributes, 0x1011: self._op_open_hint,
            0x1012: self._op_find_first, 0x1013: self._op_find_next,
            0x1014: self._op_find_close, 0x1015: lambda a: [_i32(0)],
            0x1016: lambda a: [_i32(1)], 0x1017: self._op_shutdown,
            0x1018: self._op_xdelete, 0x1019: lambda a: [_i32(0)],
            0x101A: self._op_get_drive, 0x101B: self._op_get_cluster_size,
            0x101C: lambda a: [_i32(0)], 0x101D: self._op_otp_simple,
            0x101E: self._op_otp_read, 0x101F: self._op_otp_query_length,
            0x1020: self._op_otp_simple, 0x1021: self._op_restore,
            0x1022: self._op_cmpt_read, 0x1023: lambda a: [_i32(FS_PARAM_ERROR)],
            0x1024: self._op_cmpt_write, 0x1025: self._op_get_file_detail,
            0x1026: self._op_cmpt_read_ex,
        }

    # ------------------------------------------------------------------ API
    def handle(self, op: int, args: list[bytes]) -> list[bytes]:
        fn = self._ops.get(op)
        if fn is None:
            log.error("unknown file op id 0x%x", op)
            return [_i32(FS_PARAM_ERROR)]
        try:
            reply = fn(args)
        except IndexError:
            log.error("op 0x%x: too few parameters (%d)", op, len(args))
            return [_i32(FS_PARAM_ERROR)]
        log.debug("op 0x%x -> %s", op, [r[:8].hex() for r in reply])
        return reply

    # -------------------------------------------------------------- helpers
    @staticmethod
    def _u32(args: list[bytes], i: int) -> int:
        return int.from_bytes(args[i][:4].ljust(4, b"\0"), "little")

    @staticmethod
    def _u8(args: list[bytes], i: int) -> int:
        return args[i][0] if args[i] else 0

    @staticmethod
    def _wcs_to_cs(raw: bytes) -> bytes:
        """FS_ConvWcsToCs: low byte of each UTF-16 unit, '\\' -> '/'.

        Ends at the first unit whose low byte is 0; raises FS_ACCESS_DENIED
        for '..' (stock: "can't use relative path for security").
        """
        out = bytearray()
        for i in range(0, len(raw) - 1, 2):
            c = raw[i]
            if c == 0 and out:
                break
            if len(out) >= PATH_MAX - 4:
                break
            c = ord("/") if c == ord("\\") else c
            if c == ord(".") and out and out[-1] == ord("."):
                log.error("can't use relative path for security: %r", bytes(out))
                raise FsError(FS_ACCESS_DENIED)
            out.append(c)
        return bytes(out.split(b"\0", 1)[0])

    def _drive_index(self, path: bytes) -> int:
        letter = path[:1].decode("latin-1")
        if path[1:2] == b":":
            for i, (d, _) in enumerate(DRIVES):
                if d == letter:
                    return i
        log.error("file path not find %r", path)
        raise FsError(FS_PATH_NOT_FOUND)

    def _host_path(self, raw: bytes) -> tuple[bytes, str]:
        """DOS path parameter -> (host path, drive letter)."""
        path = self._wcs_to_cs(raw)
        letter = DRIVES[self._drive_index(path)][0]
        base = self.bases[letter]
        if len(base) + len(path) - 2 >= PATH_MAX:
            log.error("file path too long: %r", path)
            raise FsError(FS_PATH_OVER_LEN_ERROR)
        return base + path[2:], letter

    def _alloc_handle(self) -> int:
        if len(self.handles) <= FS_MAX_HANDLE:
            for h in range(1, FS_MAX_HANDLE + 1):
                if h not in self.handles:
                    return h
        log.error("GetFreeHandle: file handle is full")
        return -1

    def _get(self, h: int) -> _Handle:
        hd = self.handles.get(h) if 0 <= h <= FS_MAX_HANDLE else None
        if hd is None:
            log.error("invalid handle %d", h)
            raise FsError(FS_INVALID_FILE_HANDLE)
        return hd

    def _sync_dir(self, path: bytes) -> None:
        if not self.extra_sync:
            return
        try:
            fd = os.open(os.path.dirname(path) or b".", os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(fd)
            finally:
                os.close(fd)
        except OSError:
            pass

    def _cluster_size(self, letter_code: int) -> int:
        """FS_CCCI_GetClusterSize: statfs block size of a drive."""
        letter = chr(letter_code) if 0x51 <= letter_code <= 0x5A else None
        if letter is None:
            return FS_DRIVE_NOT_FOUND
        try:
            return os.statvfs(self.bases[letter]).f_bsize
        except OSError as e:
            log.error("GetClusterSize: statfs failed: %s", e)
            return FS_GENERAL_FAILURE

    @staticmethod
    def _is_read_only(path: bytes) -> int:
        """FS_IsReadOnly: 1 if only read permission bits are set."""
        try:
            m = os.stat(path).st_mode
        except OSError as e:
            return errno_to_fs(e.errno)
        if m & 0o222 or m & 0o111:
            return 0
        return 1 if m & 0o444 else 0

    @staticmethod
    def _mode_test(path: bytes, bits: int) -> int:
        """FS_IsDIR / FS_IsFile: st_mode & bits, or an FS error code."""
        try:
            return os.stat(path).st_mode & bits
        except OSError as e:
            return errno_to_fs(e.errno)

    def _attributes(self, path: bytes) -> int:
        ro = self._is_read_only(path)
        if ro < 0:
            return ro
        d = self._mode_test(path, stat.S_IFDIR)
        if d < 0:
            return d
        return (FS_ATTR_READ_ONLY if ro == 1 else 0) | (FS_ATTR_DIR if d == stat.S_IFDIR else 0)

    # ------------------------------------------------------- file handles
    def _open(self, raw_path: bytes, flags: int) -> int:
        try:
            host, letter = self._host_path(raw_path)
        except FsError as e:
            return e.code
        if letter == "W" and self.dsp_name is not None:
            host = os.path.join(self.bases["W"], os.fsencode(f"dsp_{self.dsp_name}.bin"))
        oflag = os.O_RDONLY if flags & FS_READ_ONLY else os.O_RDWR
        if flags & FS_CREATE:
            oflag = os.O_RDWR | os.O_CREAT
        if flags & FS_CREATE_ALWAYS:
            oflag = os.O_RDWR | os.O_CREAT | os.O_TRUNC
        if flags & FS_NONBLOCK_MODE:
            oflag |= os.O_NONBLOCK
        h = self._alloc_handle()
        if h < 0:
            return FS_TOO_MANY_FILES
        try:
            fd = os.open(host, oflag, 0o660)
        except OSError as e:
            log.info("Open: fail open %r: %s (flags 0x%x)", host, e, flags)
            return errno_to_fs(e.errno)
        self.handles[h] = _Handle(path=host, flags=flags, fd=fd)
        log.debug("O: %r flags 0x%x -> %d", host, flags, h)
        return h

    def _seek(self, h: int, offset: int, whence: int) -> int:
        hd = self._get(h)
        if whence >= 3:
            return FS_PARAM_ERROR
        if hd.fd is None:
            return FS_GENERAL_FAILURE
        try:
            return _s32(os.lseek(hd.fd, offset, whence))
        except OSError as e:
            return errno_to_fs(e.errno)

    def _read(self, h: int, length: int) -> tuple[int, bytes]:
        hd = self._get(h)
        if hd.fd is None:
            return FS_GENERAL_FAILURE, b""
        try:
            return 0, os.read(hd.fd, length)
        except OSError as e:
            return errno_to_fs(e.errno), b""

    def _write(self, h: int, data: bytes) -> tuple[int, int]:
        hd = self._get(h)
        if hd.fd is None:
            return FS_GENERAL_FAILURE, -1
        try:
            n = os.write(hd.fd, data)
        except OSError as e:
            return errno_to_fs(e.errno), -1
        hd.dirty = True
        return 0, n

    def _file_size(self, h: int) -> tuple[int, int]:
        hd = self._get(h)
        if hd.fd is None:
            return FS_GENERAL_FAILURE, 0
        try:
            return 0, os.fstat(hd.fd).st_size & 0xFFFFFFFF
        except OSError as e:
            return errno_to_fs(e.errno), 0

    def _close(self, h: int) -> int:
        hd = self._get(h)
        if hd.search is not None:
            hd.search.close()
        else:
            if hd.flags & _FSYNC_ON_CLOSE or (self.extra_sync and hd.dirty):
                try:
                    os.fsync(hd.fd)
                except OSError as e:
                    log.error("Close: fsync %r failed: %s", hd.path, e)
            try:
                os.close(hd.fd)
            except OSError as e:
                return errno_to_fs(e.errno)
            if hd.flags & (FS_CREATE | FS_CREATE_ALWAYS):
                self._sync_dir(hd.path)
        del self.handles[h]
        return 0

    # ------------------------------------------------------------- ops: io
    def _op_open(self, a):
        return [_i32(self._open(a[0], self._u32(a, 1)))]

    def _op_open_hint(self, a):
        ret = self._open(a[0], self._u32(a, 1))
        return [_i32(ret), a[2][:8].ljust(8, b"\0")]

    def _op_seek(self, a):
        try:
            ret = self._seek(self._u32(a, 0), _s32(self._u32(a, 1)), self._u32(a, 2))
        except FsError as e:
            ret = e.code
        return [_i32(ret)]

    def _op_read(self, a):
        try:
            ret, data = self._read(self._u32(a, 0), self._u32(a, 1))
        except FsError as e:
            ret, data = e.code, b""
        return [_i32(ret), _i32(len(data)), data]

    def _op_write(self, a):
        try:
            ret, n = self._write(self._u32(a, 0), a[1][:self._u32(a, 2)])
        except FsError as e:
            ret, n = e.code, 0
        return [_i32(ret), _i32(n)]

    def _op_close(self, a):
        try:
            return [_i32(self._close(self._u32(a, 0)))]
        except FsError as e:
            return [_i32(e.code)]

    def _close_all(self) -> int:
        for h in sorted(self.handles):
            hd = self.handles[h]
            if hd.search is not None:
                hd.search.close()       # stock just drops the DIR*
            else:
                try:
                    os.close(hd.fd)     # no fsync in stock CloseAll
                except OSError as e:
                    return errno_to_fs(e.errno)
            del self.handles[h]
        return 0

    def _op_close_all(self, a):
        return [_i32(self._close_all())]

    def _op_shutdown(self, a):
        self._close_all()
        return []

    def _op_get_file_size(self, a):
        try:
            ret, size = self._file_size(self._u32(a, 0))
        except FsError as e:
            ret, size = e.code, 0
        return [_i32(ret), _i32(size)]

    # ---------------------------------------------------- ops: namespace
    def _path_op(self, raw: bytes, fn) -> int:
        """Convert a path parameter and run ``fn(host, letter)``."""
        try:
            host, letter = self._host_path(raw)
            return fn(host, letter)
        except FsError as e:
            return e.code
        except OSError as e:
            return errno_to_fs(e.errno)

    def _op_create_dir(self, a):
        def mk(host, letter):
            if letter == "U":
                os.makedirs(self.bases["U"], 0o770, exist_ok=True)
            os.mkdir(host, 0o770)
            self._sync_dir(host)
            return 0
        return [_i32(self._path_op(a[0], mk))]

    def _op_remove_dir(self, a):
        def rm(host, letter):
            os.rmdir(host)
            self._sync_dir(host)
            return 0
        return [_i32(self._path_op(a[0], rm))]

    def _op_delete(self, a):
        return [_i32(self._delete(a[0]))]

    def _delete(self, raw: bytes) -> int:
        def rm(host, letter):
            os.unlink(host)
            self._sync_dir(host)
            return 0
        return self._path_op(raw, rm)

    def _op_rename(self, a):
        try:
            src, _ = self._host_path(a[0])
            dst, _ = self._host_path(a[1])
            os.rename(src, dst)
        except FsError as e:
            return [_i32(e.code)]
        except OSError as e:
            return [_i32(errno_to_fs(e.errno))]
        self._sync_dir(dst)
        if os.path.dirname(src) != os.path.dirname(dst):
            self._sync_dir(src)
        return [_i32(0)]

    def _op_get_attributes(self, a):
        return [_i32(self._path_op(a[0], lambda host, letter: self._attributes(host)))]

    def _op_get_file_detail(self, a):
        detail = bytes(0x18)

        def st(host, letter):
            nonlocal detail
            s = os.stat(host)
            detail = struct.pack("<QQQ", int(s.st_mtime) & (2**64 - 1),
                                 int(s.st_atime) & (2**64 - 1), int(s.st_ctime) & (2**64 - 1))
            return 0
        ret = self._path_op(a[0], st)
        return [_i32(ret), detail]

    def _op_restore(self, a):
        # Stock restores the file from the nvram "bin region"; not available.
        def restore(host, letter):
            log.error("Restore Fail: %r (no bin region)", host)
            return FS_GENERAL_FAILURE
        return [_i32(self._path_op(a[0], restore))]

    # --------------------------------------------------- ops: directories
    def _recursive_search(self, path: bytes, mode: int) -> tuple[int, int]:
        """FS_RecursiveSearch -> (entry count or error, clusters)."""
        cs = self._cluster_size(ord("Z"))
        count = clusters = 0
        try:
            entries = list(os.scandir(path))
        except OSError as e:
            log.error("RecursiveSearch: fail open dir %r: %s", path, e)
            return 0, 0
        for ent in entries:
            if len(path) + len(ent.name) >= PATH_MAX - 1:
                return FS_PATH_OVER_LEN_ERROR, clusters
            full = path + b"/" + os.fsencode(ent.name)
            try:
                s = os.stat(full)
            except OSError:
                continue
            if stat.S_ISDIR(s.st_mode):
                count += 1 if mode & FS_DIR_TYPE else 0
                if mode & FS_RECURSIVE_TYPE:
                    n, c = self._recursive_search(full, mode)
                    if n < 0:
                        return n, clusters
                    count += n
                    clusters += c
            elif mode & FS_FILE_TYPE and stat.S_ISREG(s.st_mode):
                if s.st_size > 0 and cs > 0:
                    clusters += s.st_size // cs + 1
                count += 1
        return count, clusters

    def _require_dir(self, host: bytes) -> None:
        s = os.stat(host)
        if not stat.S_ISDIR(s.st_mode):
            raise FsError(FS_PARAM_ERROR)

    def _op_count(self, a):
        mode = self._u32(a, 1)

        def count(host, letter):
            self._require_dir(host)
            return self._recursive_search(host, mode)[0]
        return [_i32(self._path_op(a[0], count))]

    def _op_get_folder_size(self, a):
        flag = self._u32(a, 1)

        def size(host, letter):
            cs = self._cluster_size(ord(letter))
            self._require_dir(host)
            n, clusters = self._recursive_search(host, FS_FILE_TYPE | FS_DIR_TYPE | FS_RECURSIVE_TYPE)
            if n < 0:
                return n
            return _s32(clusters * (1 if flag == 2 else cs))
        return [_i32(self._path_op(a[0], size))]

    def _recursive_delete(self, path: bytes, flag: int) -> int:
        """FS_RecursiveDelete -> number of removed entries or error."""
        n = 0
        try:
            entries = list(os.scandir(path))
        except OSError as e:
            log.error("RecursiveDelete: fail open dir %r: %s", path, e)
            return 0
        for ent in entries:
            if len(path) + len(ent.name) >= PATH_MAX - 1:
                return FS_PATH_OVER_LEN_ERROR
            full = path + b"/" + os.fsencode(ent.name)
            try:
                s = os.stat(full)
            except OSError:
                continue
            try:
                if stat.S_ISDIR(s.st_mode):
                    if flag & FS_RECURSIVE_TYPE:
                        r = self._recursive_delete(full, flag)
                        if r < 0:
                            return r
                        n += r
                    if flag & FS_DIR_TYPE:
                        os.rmdir(full)
                        n += 1
                elif flag & FS_FILE_TYPE and stat.S_ISREG(s.st_mode):
                    os.unlink(full)
                    n += 1
            except OSError as e:
                return errno_to_fs(e.errno)
        self._sync_dir(path + b"/.")
        return n

    def _xdelete(self, raw: bytes, flag: int) -> int:
        def xdel(host, letter):
            if not flag & 0x1C or flag & ~0x1C & 0xFFFFFFFF:
                return FS_PARAM_ERROR
            self._require_dir(host)
            n = self._recursive_delete(host, flag)
            if flag & (FS_DIR_TYPE | FS_RECURSIVE_TYPE) == FS_DIR_TYPE | FS_RECURSIVE_TYPE:
                os.rmdir(host)
                self._sync_dir(host)
                n += 1
            return n
        return self._path_op(raw, xdel)

    def _op_xdelete(self, a):
        return [_i32(self._xdelete(a[0], self._u32(a, 1)))]

    def _op_move(self, a):
        return [_i32(self._move(a[0], a[1], self._u32(a, 2)))]

    def _move(self, raw_src: bytes, raw_dst: bytes, flag: int) -> int:
        try:
            src, _ = self._host_path(raw_src)
            dst, _ = self._host_path(raw_dst)
        except FsError as e:
            return e.code
        if src == dst:
            return FS_PARAM_ERROR
        if not flag & 3 or flag & 0xFFFEFFFC:
            return FS_PARAM_ERROR
        if flag & FS_MOVE_KILL and self._is_read_only(src) > 0:
            return FS_READ_ONLY_ERROR
        ret = FS_GENERAL_FAILURE
        if flag & FS_MOVE_OVERWRITE:
            if self._is_read_only(dst) > 0:
                return FS_READ_ONLY_ERROR
            if self._mode_test(dst, stat.S_IFDIR) > 0:
                ret = self._xdelete(raw_dst, 0x1C)
                if ret < 0:
                    return ret
            elif self._mode_test(dst, stat.S_IFREG) > 0:
                ret = self._delete(raw_dst)
                if ret < 0:
                    return ret
        # Stock only logs "kill %s, NULL" for FS_MOVE_KILL: the source stays.
        if not flag & FS_MOVE_COPY:
            return ret
        try:
            with open(src, "rb") as f:
                data = f.read()
        except OSError as e:
            return errno_to_fs(e.errno)
        try:
            fd = os.open(dst, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o770)
        except OSError as e:
            return errno_to_fs(e.errno)
        try:
            n = os.write(fd, data)
            if self.extra_sync:
                os.fsync(fd)
        except OSError as e:
            return errno_to_fs(e.errno)
        finally:
            os.close(fd)
        self._sync_dir(dst)
        return 0 if n >= len(data) else FS_ERROR_RESERVED

    # ------------------------------------------------------ ops: search
    def _dos_entry(self, path: bytes, name_len: int, maxlen: int) -> bytes:
        """FS_EntryLinuxToDos -> 0x34-byte FS_DOSDirEntry."""
        e = bytearray(DOS_DIR_ENTRY_SIZE)
        try:
            s = os.stat(path)
            mtime, size = s.st_mtime, s.st_size
        except OSError as err:
            log.error("EntryLinuxToDos: stat %r failed: %s", path, err)
            mtime, size = 0, 0
        e[0x0B] = self._attributes(path) & 0xFF
        e[0x0C] = 1 if name_len < maxlen else 0
        # Stock calls localtime() for atime then mtime and uses the shared
        # static result, so both date/time fields carry the mtime.
        dt = dos_datetime(mtime)
        struct.pack_into("<I", e, 0x0E, dt)
        struct.pack_into("<I", e, 0x16, dt)
        struct.pack_into("<I", e, 0x1C, size & 0xFFFFFFFF)
        return bytes(e)

    @staticmethod
    def _cs_to_wcs(name: bytes, maxlen: int) -> bytes:
        """FS_ConvCsToWcs + reply length rule: count*2+2 bytes, or empty."""
        if maxlen == 0:
            return b""
        count = len(name) if len(name) < maxlen else maxlen - 1
        if count == 0:
            return b""
        chars = name[:count].replace(b"/", b"\\")
        return b"".join(bytes((c, 0)) for c in chars) + b"\0\0"

    def _next_match(self, hd: _Handle, maxlen: int) -> tuple[int, bytes, bytes]:
        """Advance a search; returns (ret, entry, name). ret 0 = found."""
        for ent in hd.search:
            name = os.fsencode(ent.name)
            if not pat_match(hd.pattern, name):
                continue
            if len(hd.search_dir) + 1 + len(name) > PATH_MAX:
                return FS_PATH_OVER_LEN_ERROR, bytes(DOS_DIR_ENTRY_SIZE), b""
            full = hd.search_dir + b"/" + name
            fa = self._attributes(full) & 0xFFFFFFFF
            if hd.attr & ~fa & 0xFFFFFFFF or fa & hd.attr_mask:
                continue
            hd.path = full
            return 0, self._dos_entry(full, len(name), maxlen), self._cs_to_wcs(name, maxlen)
        return FS_NO_MORE_FILES, bytes(DOS_DIR_ENTRY_SIZE), b""

    def _op_find_first(self, a):
        attr, mask, maxlen = self._u8(a, 1), self._u8(a, 2), self._u32(a, 3)
        empty = bytes(DOS_DIR_ENTRY_SIZE)
        try:
            host, _ = self._host_path(a[0])
        except FsError as e:
            return [_i32(e.code), empty, b""]
        slash = host.rfind(b"/")
        if slash < 1:
            return [_i32(FS_PATH_NOT_FOUND), empty, b""]
        pattern, sdir = host[slash + 1:], host[:slash]
        if len(pattern) >= 0x100:
            return [_i32(FS_PATH_OVER_LEN_ERROR), empty, b""]
        try:
            it = os.scandir(sdir)
        except OSError as e:
            return [_i32(errno_to_fs(e.errno)), empty, b""]
        hd = _Handle(path=sdir, search=it, search_dir=sdir, pattern=pattern,
                     attr=attr, attr_mask=mask)
        ret, entry, name = self._next_match(hd, maxlen)
        if ret < 0:
            it.close()
            return [_i32(ret), entry, b""]
        h = self._alloc_handle()
        if h < 0:
            it.close()
            return [_i32(FS_TOO_MANY_FILES), empty, b""]
        self.handles[h] = hd
        return [_i32(h), entry, name]

    def _op_find_next(self, a):
        maxlen = self._u32(a, 1)
        try:
            hd = self._get(self._u32(a, 0))
        except FsError as e:
            return [_i32(e.code), bytes(DOS_DIR_ENTRY_SIZE), b""]
        if hd.search is None:
            return [_i32(FS_INVALID_FILE_HANDLE), bytes(DOS_DIR_ENTRY_SIZE), b""]
        ret, entry, name = self._next_match(hd, maxlen)
        return [_i32(ret), entry, name]

    def _op_find_close(self, a):
        try:
            hd = self._get(self._u32(a, 0))
        except FsError as e:
            return [_i32(e.code)]
        if hd.search is not None:
            hd.search.close()
        else:
            try:
                os.close(hd.fd)
            except OSError as e:
                return [_i32(errno_to_fs(e.errno))]
        del self.handles[self._u32(a, 0)]
        return [_i32(0)]

    # ------------------------------------------------------ ops: volume
    def _op_get_disk_info(self, a):
        info = bytearray(DISK_INFO_SIZE)
        try:
            st = os.statvfs(self.bases["Z"])
        except OSError as e:
            log.error("GetDiskInfo: statfs failed: %s", e)
            return [_i32(FS_ERROR_RESERVED), bytes(info)]
        struct.pack_into("<I", info, 0x30, 0x200)                        # BytesPerSector
        struct.pack_into("<I", info, 0x34, (st.f_bsize >> 9) & 0xFFFFFFFF)  # SectorsPerCluster
        struct.pack_into("<I", info, 0x38, st.f_blocks & 0xFFFFFFFF)      # TotalClusters
        struct.pack_into("<I", info, 0x40, 0x200)                        # FreeClusters (sic)
        return [_i32(0), bytes(info)]

    def _op_get_drive(self, a):
        typ, serial, alt = self._u32(a, 0), self._u32(a, 1), self._u32(a, 2)
        invalid = (not typ & 0x1C or typ & 0xFFFFFFE3
                   or (typ != 0x10 and serial > 2) or serial == 0
                   or (typ == 0x10 and serial != 1)
                   or (alt != 1 and alt & 1) or (alt != 2 and alt & 2))
        if invalid or typ != 4:
            return [_i32(FS_PARAM_ERROR)]
        return [_i32(ord("Z"))]

    def _op_get_cluster_size(self, a):
        return [_i32(self._cluster_size(self._u32(a, 0)))]

    # --------------------------------------------------------- ops: OTP
    # Stock talks to /dev/otp or libotp; without a device it returns -12.
    def _op_otp_simple(self, a):
        return [_i32(FS_UNSUPPORTED_DRIVER_FUNCTION)]

    def _op_otp_read(self, a):
        return [_i32(FS_UNSUPPORTED_DRIVER_FUNCTION), bytes(self._u32(a, 2))]

    def _op_otp_query_length(self, a):
        return [_i32(FS_UNSUPPORTED_DRIVER_FUNCTION), _i32(0)]

    # -------------------------------------------------------- ops: CMPT
    # Request structures are arrays of u32 words (32-bit modem structs):
    #   read  (0x1022/0x1026): [0] opid_map, [3] open flags, [4] file size,
    #         [5] offset, [6] whence, [8] length, [10] file handle (EX only)
    #   write (0x1024): [0] opid_map, [3] open flags, [4] offset, [5] whence,
    #         [7] length, [8] file handle (< 0: open the file first)
    # Each reply starts with an 8-byte {u32 done_opid_map, s32 error}.
    @staticmethod
    def _words(raw: bytes, n: int = 16) -> list[int]:
        raw = raw[:n * 4].ljust(n * 4, b"\0")
        return list(struct.unpack(f"<{n}I", raw))

    def _cmpt_read_steps(self, h: int, w: list[int], done: int):
        """Shared GETSIZE/SEEK/READ/CLOSE part of CMPT_Read(_EX).

        Returns (ok, done, err, size, data); on failure the file is closed.
        """
        size, data = w[4], b""
        steps = ((CMPT_GETSIZE, lambda: self._file_size(h)),
                 (CMPT_SEEK, lambda: (self._seek(h, _s32(w[5]), w[6]), None)),
                 (CMPT_READ, lambda: self._read(h, w[8])))
        for bit, fn in steps:
            if not w[0] & bit:
                continue
            ret, val = fn()
            if ret < 0:
                self._close(h)
                return False, done, ret, size, b""
            if bit == CMPT_GETSIZE:
                size = val
            elif bit == CMPT_READ:
                data = val
            done |= bit
        return True, done, 0, size, data

    def _op_cmpt_read(self, a):
        w = self._words(a[1])
        size, data = w[4], b""
        if not w[0] & CMPT_OPEN:
            done, err = 0, FS_GENERAL_FAILURE
        else:
            h = self._open(a[0], w[3])
            if h < 0:
                done, err = 0, h
            else:
                ok, done, err, size, data = self._cmpt_read_steps(h, w, CMPT_OPEN)
                if ok:
                    ret = self._close(h)
                    if not w[0] & CMPT_CLOSE:
                        log.error("cmpt_Read, miss close flag")
                    elif ret < 0:
                        err = ret
                    else:
                        done |= CMPT_CLOSE
        return [struct.pack("<Ii", done, err), _i32(size), _i32(len(data)), data]

    def _op_cmpt_read_ex(self, a):
        w = self._words(a[1])
        size, data, h_out = w[4], b"", -1
        if w[0] & CMPT_OPEN:
            h = self._open(a[0], w[3])
            done = CMPT_OPEN
        else:
            h, done = _s32(w[10]), 0
        if h < 0:
            done, err = 0, (h if w[0] & CMPT_OPEN else FS_ERROR_RESERVED)
        else:
            try:
                ok, done, err, size, data = self._cmpt_read_steps(h, w, done)
                if ok and w[0] & CMPT_CLOSE:
                    ret = self._close(h)
                    if ret < 0:
                        ok, err = False, ret
                    else:
                        done |= CMPT_CLOSE
                h_out = h if ok else -1
            except FsError as e:
                err = e.code
        return [struct.pack("<Ii", done, err), _i32(size), _i32(len(data)), data, _i32(h_out)]

    def _op_cmpt_write(self, a):
        w = self._words(a[1])
        payload = a[2] if len(a) > 2 else b""
        h, written, done, err = _s32(w[8]), 0, 0, 0
        if h < 0:
            if not w[0] & CMPT_OPEN:
                return [struct.pack("<Ii", 0, FS_PARAM_ERROR), _i32(-1), _i32(0)]
            h = self._open(a[0], w[3])
            if h < 0:
                return [struct.pack("<Ii", 0, h), _i32(-1), _i32(0)]
            done = CMPT_OPEN
        try:
            if w[0] & CMPT_SEEK:
                ret = self._seek(h, _s32(w[4]), w[5])
                if ret < 0:
                    self._close(h)
                    return [struct.pack("<Ii", done, ret), _i32(h), _i32(0)]
                done |= CMPT_SEEK
            if w[0] & CMPT_WRITE:
                ret, written = self._write(h, payload[:w[7]])
                if ret < 0:
                    self._close(h)
                    return [struct.pack("<Ii", done, ret), _i32(h), _i32(written)]
                done |= CMPT_WRITE
            if w[0] & CMPT_CLOSE:
                ret = self._close(h)
                if ret < 0:
                    return [struct.pack("<Ii", done, ret), _i32(h), _i32(written)]
                done |= CMPT_CLOSE
        except FsError as e:
            err = e.code
        return [struct.pack("<Ii", done, err), _i32(h), _i32(written)]
