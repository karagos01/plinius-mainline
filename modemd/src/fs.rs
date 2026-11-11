//! Re-implementation of the MediaTek modem file system server (ccci_fsd).
//!
//! The stock server is the `fsd_main` thread of Android's `ccci_mdinit`.
//! The modem keeps its NVRAM (IMEI, calibration, ...) on the AP and accesses
//! it through the CCCI FS channel with DOS-like file operations. Integers are
//! little-endian 32-bit, path names are NUL-terminated UTF-16LE DOS paths
//! such as `Z:\NVRAM\NVD_IMEI\LD0B_001`. Results are signed: >= 0 on
//! success (a handle, a count or 0), otherwise a negative FS error code.
//!
//! | op     | name           | request                          | reply                               |
//! |--------|----------------|----------------------------------|-------------------------------------|
//! | 0x1001 | Open           | path, flags                      | ret (handle)                        |
//! | 0x1002 | Seek           | handle, offset (s32), whence     | ret (new position)                  |
//! | 0x1003 | Read           | handle, length                   | ret, nread, data                    |
//! | 0x1004 | Write          | handle, data, length             | ret, nwritten                       |
//! | 0x1005 | Close          | handle                           | ret                                 |
//! | 0x1006 | CloseAll       | -                                | ret                                 |
//! | 0x1007 | CreateDir      | path                             | ret                                 |
//! | 0x1008 | RemoveDir      | path                             | ret                                 |
//! | 0x1009 | GetFileSize    | handle                           | ret, size                           |
//! | 0x100A | GetFolderSize  | path, flag                       | ret (bytes, clusters if flag == 2)  |
//! | 0x100B | Rename         | old path, new path               | ret                                 |
//! | 0x100C | Move           | src path, dst path, flag         | ret                                 |
//! | 0x100D | Count          | path, flag                       | ret (number of entries)             |
//! | 0x100E | GetDiskInfo    | (ignored)                        | ret, FS_DiskInfo[0x54]              |
//! | 0x100F | Delete         | path                             | ret                                 |
//! | 0x1010 | GetAttributes  | path                             | ret (attribute bits)                |
//! | 0x1011 | OpenHint       | path, flags, hint[8]             | ret (handle), hint (echo)           |
//! | 0x1012 | FindFirst      | pattern, attr(u8), mask(u8), max | ret (handle), DOSDirEntry, name     |
//! | 0x1013 | FindNext       | handle, max                      | ret, DOSDirEntry[0x34], name        |
//! | 0x1014 | FindClose      | handle                           | ret                                 |
//! | 0x1015 | LockFAT        | (ignored)                        | 0                                   |
//! | 0x1016 | ("UK")         | (ignored)                        | 1                                   |
//! | 0x1017 | Shutdown       | -                                | (no parameters)                     |
//! | 0x1018 | XDelete        | path, flag                       | ret (number of deleted entries)     |
//! | 0x1019 | (CDF)          | (ignored)                        | 0                                   |
//! | 0x101A | GetDrive       | type, serial, alt_mask           | ret ('Z' or -2)                     |
//! | 0x101B | GetClusterSize | drive letter                     | ret                                 |
//! | 0x101C | (SDF)          | (ignored)                        | 0                                   |
//! | 0x101D | OTPWrite       | dev, offset, data, length        | ret (no OTP here: -12)              |
//! | 0x101E | OTPRead        | dev, offset, length              | ret, data                           |
//! | 0x101F | OTPQueryLength | dev                              | ret, length                         |
//! | 0x1020 | OTPLock        | dev                              | ret                                 |
//! | 0x1021 | Restore        | path                             | ret (no bin region here: -18)       |
//! | 0x1022 | CMPT_Read      | path, CmptReadReq                | (done, err), size, nread, data      |
//! | 0x1023 | BinRegion      | (ignored)                        | -2                                  |
//! | 0x1024 | CMPT_Write     | path, CmptWriteReq, data         | (done, err), handle, nwritten       |
//! | 0x1025 | GetFileDetail  | path                             | ret, {u64 mtime, atime, ctime}      |
//! | 0x1026 | CMPT_Read_EX   | path, CmptReadReq                | (done, err), size, nread, data, h   |
//!
//! "name" in the Find replies is the UTF-16LE file name with '/' turned into
//! '\', NUL-terminated, at most `max` - 1 characters; it is an empty
//! parameter when nothing was found.
//!
//! Drives: Z nvdata/md, X protect_f/md, Y protect_s/md, W firmware,
//! V nvdata/md_cmn, U mdlpm, T mdota, S nvcfg, R etc_md, Q data_de_md, all
//! relative to the server root (stock: /mnt/vendor/nvdata/md, ...).

use crate::wire::{i32p, u32p};
use crate::{log, verbose};
use std::collections::BTreeMap;
use std::ffi::{CString, OsStr};
use std::fs::{self, File, OpenOptions, ReadDir};
use std::io;
use std::os::fd::{AsRawFd, IntoRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};

// FS error codes (MediaTek fs_errcode.h)
const FS_ERROR_RESERVED: i32 = -1;
const FS_PARAM_ERROR: i32 = -2;
const FS_DRIVE_NOT_FOUND: i32 = -4;
const FS_TOO_MANY_FILES: i32 = -5;
const FS_NO_MORE_FILES: i32 = -6;
const FS_FILE_NOT_FOUND: i32 = -9;
const FS_INVALID_FILE_HANDLE: i32 = -10;
const FS_UNSUPPORTED_DRIVER_FUNCTION: i32 = -12;
const FS_ACCESS_DENIED: i32 = -16;
const FS_GENERAL_FAILURE: i32 = -18;
const FS_PATH_NOT_FOUND: i32 = -19;
const FS_READ_ONLY_ERROR: i32 = -45;
const FS_PATH_OVER_LEN_ERROR: i32 = -49;

// open flags
const FS_READ_ONLY: u32 = 0x100;
const FS_CREATE: u32 = 0x10000;
const FS_CREATE_ALWAYS: u32 = 0x20000;
const FS_COMMITTED: u32 = 0x100_0000;
const FS_NONBLOCK_MODE: u32 = 0x1000_0000;
const FSYNC_ON_CLOSE: u32 = FS_CREATE | FS_CREATE_ALWAYS | FS_COMMITTED;

// attribute bits, search flags, move flags
const FS_ATTR_READ_ONLY: i32 = 0x01;
const FS_ATTR_DIR: i32 = 0x10;
const FS_FILE_TYPE: u32 = 0x04;
const FS_DIR_TYPE: u32 = 0x08;
const FS_RECURSIVE_TYPE: u32 = 0x10;
const FS_MOVE_COPY: u32 = 0x1;
const FS_MOVE_KILL: u32 = 0x2;
const FS_MOVE_OVERWRITE: u32 = 0x10000;

// CMPT op bits (opid_map)
const CMPT_OPEN: u32 = 0x01;
const CMPT_GETSIZE: u32 = 0x02;
const CMPT_SEEK: u32 = 0x04;
const CMPT_READ: u32 = 0x08;
const CMPT_CLOSE: u32 = 0x10;
const CMPT_WRITE: u32 = 0x20;

const FS_MAX_HANDLE: u32 = 0x80; // handles are table indexes 1..128
const PATH_MAX: usize = 0x1000;
const DOS_DIR_ENTRY_SIZE: usize = 0x34;
const DISK_INFO_SIZE: usize = 0x54;
const READ_MAX: usize = 1 << 24;

const S_IFDIR: u32 = 0o040000;
const S_IFREG: u32 = 0o100000;

/// Drive letters in stock table order, with the subdirectory used here.
const DRIVES: [(u8, &str); 10] = [
    (b'Z', "nvdata/md"),     // /mnt/vendor/nvdata/md
    (b'X', "protect_f/md"),  // /mnt/vendor/protect_f/md
    (b'Y', "protect_s/md"),  // /mnt/vendor/protect_s/md
    (b'W', "firmware"),      // /vendor/firmware
    (b'V', "nvdata/md_cmn"), // /mnt/vendor/nvdata/md_cmn
    (b'U', "mdlpm"),         // /data/vendor/mdlpm
    (b'T', "mdota"),         // /vendor/etc/mdota
    (b'S', "nvcfg"),         // /mnt/vendor/nvcfg
    (b'R', "etc_md"),        // /vendor/etc/md
    (b'Q', "data_de_md"),    // /data/vendor_de/md
];
/// Directories FS_Init creates (mode 0770).
const INIT_DIRS: [&str; 5] = ["nvdata", "nvdata/md_cmn", "nvdata/md", "protect_f/md", "protect_s/md"];

/// An FS error code, carried back to the op handler.
#[derive(Debug)]
struct FsErr(i32);

/// A missing request parameter: the whole op fails with FS_PARAM_ERROR.
#[derive(Debug)]
pub struct Missing;

type Params = Vec<Vec<u8>>;

/// FS_ErrorConv: map a Linux errno to an FS error code.
fn errno_to_fs(e: &io::Error) -> i32 {
    match e.raw_os_error() {
        Some(libc::EMFILE) => FS_TOO_MANY_FILES,
        Some(libc::EACCES) => FS_ACCESS_DENIED,
        Some(libc::ENOENT) => FS_FILE_NOT_FOUND,
        _ => FS_GENERAL_FAILURE,
    }
}

fn last_err() -> i32 {
    errno_to_fs(&io::Error::last_os_error())
}

/// PatMatch: case-sensitive '*' / '?' wildcard match.
fn pat_match(p: &[u8], n: &[u8]) -> bool {
    match p.first() {
        None => n.is_empty(),
        Some(b'*') => pat_match(&p[1..], n) || (!n.is_empty() && pat_match(p, &n[1..])),
        Some(b'?') => !n.is_empty() && pat_match(&p[1..], &n[1..]),
        Some(&c) => !n.is_empty() && n[0] == c && pat_match(&p[1..], &n[1..]),
    }
}

/// DOS date/time as packed by FS_EntryLinuxToDos. Stock quirks kept: the
/// month field holds tm_mday & 0xF and seconds are not halved.
#[allow(deprecated)] // libc::time_t: 64-bit on glibc and musl >= 1.2
fn dos_datetime(t: i64) -> u32 {
    // SAFETY: localtime_r fills the caller's tm
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let tt = t as libc::time_t;
    unsafe { libc::localtime_r(&tt, &mut tm) };
    let (year, mday) = (tm.tm_year as u64, tm.tm_mday as u64);
    let mut v = (tm.tm_sec as u64 & 0x1f) | ((tm.tm_min as u64 & 0x3f) << 5) | ((tm.tm_hour as u64 & 0x1f) << 11);
    v |= year << 25;
    v = (v & !(0xf << 21)) | ((mday & 0xf) << 21);
    v = (v & !(0x1f << 16)) | ((mday & 0x1f) << 16);
    (v.wrapping_add(0x6000_0000) & 0xffff_ffff) as u32
}

fn path_os(p: &[u8]) -> &OsStr {
    OsStr::from_bytes(p)
}

fn dirname(p: &[u8]) -> &[u8] {
    match p.iter().rposition(|&c| c == b'/') {
        Some(0) => b"/",
        Some(i) => &p[..i],
        None => b"",
    }
}

fn stat(p: &[u8]) -> io::Result<fs::Metadata> {
    fs::metadata(path_os(p))
}

enum Kind {
    File { fd: RawFd, flags: u32, dirty: bool },
    Search { iter: ReadDir, dir: Vec<u8>, pattern: Vec<u8>, attr: u32, mask: u32 },
}

struct Handle {
    path: Vec<u8>, // host path (for search handles: last match)
    kind: Kind,
}

struct Args<'a>(&'a [Vec<u8>]);

impl Args<'_> {
    fn raw(&self, i: usize) -> Result<&[u8], Missing> {
        self.0.get(i).map(|v| v.as_slice()).ok_or(Missing)
    }
    fn u32(&self, i: usize) -> Result<u32, Missing> {
        let r = self.raw(i)?;
        let mut b = [0u8; 4];
        let n = r.len().min(4);
        b[..n].copy_from_slice(&r[..n]);
        Ok(u32::from_le_bytes(b))
    }
    fn u8(&self, i: usize) -> Result<u32, Missing> {
        Ok(self.raw(i)?.first().copied().unwrap_or(0) as u32)
    }
}

pub struct FsServer {
    bases: BTreeMap<u8, Vec<u8>>,
    handles: BTreeMap<u32, Handle>,
    /// beyond stock behaviour: fsync written files on close and parent
    /// directories after namespace changes
    extra_sync: bool,
}

type OpResult = Result<Params, Missing>;

impl FsServer {
    pub fn new(root: &str) -> FsServer {
        let root = fs::canonicalize(root).unwrap_or_else(|_| root.into());
        let root = root.as_os_str().as_bytes().to_vec();
        let join = |sub: &str| {
            let mut p = root.clone();
            p.push(b'/');
            p.extend_from_slice(sub.as_bytes());
            p
        };
        for d in INIT_DIRS {
            let p = join(d);
            // like os.makedirs(p, 0o770): missing parents get the default mode
            let parent = dirname(&p).to_vec();
            let r = fs::create_dir_all(path_os(&parent)).and_then(|_| match fs::DirBuilder::new().mode(0o770).create(path_os(&p)) {
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
                r => r,
            });
            if let Err(e) = r {
                log(&format!("fs: FS_Init: mkdir {} failed: {e}", String::from_utf8_lossy(&p)));
            }
        }
        FsServer {
            bases: DRIVES.iter().map(|&(l, sub)| (l, join(sub))).collect(),
            handles: BTreeMap::new(),
            extra_sync: true,
        }
    }

    /// Run one request; any failure turns into FS_PARAM_ERROR, like the
    /// stock server's "too few parameters" and unknown-op paths.
    pub fn handle(&mut self, op: u32, args: &[Vec<u8>]) -> Params {
        let a = Args(args);
        let r = (|| -> OpResult { match op {
            0x1001 => self.op_open(&a),
            0x1002 => self.op_seek(&a),
            0x1003 => self.op_read(&a),
            0x1004 => self.op_write(&a),
            0x1005 => self.op_close(&a),
            0x1006 => Ok(vec![i32p(self.close_all())]),
            0x1007 => self.op_create_dir(&a),
            0x1008 => self.op_remove_dir(&a),
            0x1009 => self.op_get_file_size(&a),
            0x100a => self.op_get_folder_size(&a),
            0x100b => self.op_rename(&a),
            0x100c => self.op_move(&a),
            0x100d => self.op_count(&a),
            0x100e => Ok(self.op_get_disk_info()),
            0x100f => Ok(vec![i32p(self.delete(a.raw(0)?))]),
            0x1010 => self.op_get_attributes(&a),
            0x1011 => self.op_open_hint(&a),
            0x1012 => self.op_find_first(&a),
            0x1013 => self.op_find_next(&a),
            0x1014 => self.op_find_close(&a),
            0x1015 | 0x1019 | 0x101c => Ok(vec![i32p(0)]),
            0x1016 => Ok(vec![i32p(1)]),
            0x1017 => {
                self.close_all();
                Ok(vec![])
            }
            0x1018 => Ok(vec![i32p(self.xdelete(a.raw(0)?, a.u32(1)?))]),
            0x101a => self.op_get_drive(&a),
            0x101b => Ok(vec![i32p(self.cluster_size(a.u32(0)?))]),
            // stock talks to /dev/otp or libotp; without a device it returns -12
            0x101d | 0x1020 => Ok(vec![i32p(FS_UNSUPPORTED_DRIVER_FUNCTION)]),
            0x101e => Ok(vec![
                i32p(FS_UNSUPPORTED_DRIVER_FUNCTION),
                vec![0; (a.u32(2)? as usize).min(READ_MAX)],
            ]),
            0x101f => Ok(vec![i32p(FS_UNSUPPORTED_DRIVER_FUNCTION), i32p(0)]),
            0x1021 => self.op_restore(&a),
            0x1022 => self.op_cmpt_read(&a),
            0x1023 => Ok(vec![i32p(FS_PARAM_ERROR)]),
            0x1024 => self.op_cmpt_write(&a),
            0x1025 => self.op_get_file_detail(&a),
            0x1026 => self.op_cmpt_read_ex(&a),
            _ => {
                log(&format!("fs: unknown file op id {op:#x}"));
                Ok(vec![i32p(FS_PARAM_ERROR)])
            }
        } })();
        r.unwrap_or_else(|Missing| {
            log(&format!("fs: op {op:#x}: too few parameters ({})", args.len()));
            vec![i32p(FS_PARAM_ERROR)]
        })
    }

    // ------------------------------------------------------------ helpers

    /// FS_ConvWcsToCs: low byte of each UTF-16 unit, '\' -> '/'. Ends at the
    /// first unit whose low byte is 0; '..' is refused ("can't use relative
    /// path for security").
    fn wcs_to_cs(raw: &[u8]) -> Result<Vec<u8>, FsErr> {
        let mut out: Vec<u8> = Vec::new();
        let mut i = 0;
        while i + 1 < raw.len() {
            let mut c = raw[i];
            i += 2;
            if c == 0 && !out.is_empty() {
                break;
            }
            if out.len() >= PATH_MAX - 4 {
                break;
            }
            if c == b'\\' {
                c = b'/';
            }
            if c == b'.' && out.last() == Some(&b'.') {
                log(&format!("fs: can't use relative path for security: {}", String::from_utf8_lossy(&out)));
                return Err(FsErr(FS_ACCESS_DENIED));
            }
            out.push(c);
        }
        if let Some(z) = out.iter().position(|&c| c == 0) {
            out.truncate(z);
        }
        Ok(out)
    }

    /// DOS path parameter -> (host path, drive letter)
    fn host_path(&self, raw: &[u8]) -> Result<(Vec<u8>, u8), FsErr> {
        let path = Self::wcs_to_cs(raw)?;
        let letter = match (path.first(), path.get(1)) {
            (Some(&l), Some(b':')) if DRIVES.iter().any(|d| d.0 == l) => l,
            _ => {
                log(&format!("fs: file path not find {}", String::from_utf8_lossy(&path)));
                return Err(FsErr(FS_PATH_NOT_FOUND));
            }
        };
        let base = &self.bases[&letter];
        if base.len() + path.len() - 2 >= PATH_MAX {
            log(&format!("fs: file path too long: {}", String::from_utf8_lossy(&path)));
            return Err(FsErr(FS_PATH_OVER_LEN_ERROR));
        }
        let mut host = base.clone();
        host.extend_from_slice(&path[2..]);
        Ok((host, letter))
    }

    fn alloc_handle(&self) -> Option<u32> {
        let h = (1..=FS_MAX_HANDLE).find(|h| !self.handles.contains_key(h));
        if h.is_none() {
            log("fs: GetFreeHandle: file handle is full");
        }
        h
    }

    fn get(&mut self, h: u32) -> Result<&mut Handle, FsErr> {
        match self.handles.get_mut(&h) {
            Some(hd) if h <= FS_MAX_HANDLE => Ok(hd),
            _ => {
                log(&format!("fs: invalid handle {h}"));
                Err(FsErr(FS_INVALID_FILE_HANDLE))
            }
        }
    }

    fn file_fd(&mut self, h: u32) -> Result<RawFd, FsErr> {
        match self.get(h)?.kind {
            Kind::File { fd, .. } => Ok(fd),
            Kind::Search { .. } => Err(FsErr(FS_GENERAL_FAILURE)),
        }
    }

    fn sync_dir(&self, path: &[u8]) {
        if !self.extra_sync {
            return;
        }
        let d = dirname(path);
        let d: &[u8] = if d.is_empty() { b"." } else { d };
        if let Ok(f) = File::open(path_os(d)) {
            let _ = f.sync_all();
        }
    }

    /// FS_CCCI_GetClusterSize: statfs block size of a drive
    fn cluster_size(&self, letter: u32) -> i32 {
        if !(0x51..=0x5a).contains(&letter) {
            return FS_DRIVE_NOT_FOUND;
        }
        match statvfs(&self.bases[&(letter as u8)]) {
            Ok(st) => st.f_bsize as i32,
            Err(e) => {
                log(&format!("fs: GetClusterSize: statfs failed: {e}"));
                FS_GENERAL_FAILURE
            }
        }
    }

    /// FS_IsReadOnly: 1 if only read permission bits are set
    fn is_read_only(path: &[u8]) -> i32 {
        match stat(path) {
            Err(e) => errno_to_fs(&e),
            Ok(m) => {
                let m = m.mode();
                if m & 0o222 != 0 || m & 0o111 != 0 {
                    0
                } else if m & 0o444 != 0 {
                    1
                } else {
                    0
                }
            }
        }
    }

    /// FS_IsDIR / FS_IsFile: st_mode & bits, or an FS error code
    fn mode_test(path: &[u8], bits: u32) -> i32 {
        match stat(path) {
            Err(e) => errno_to_fs(&e),
            Ok(m) => (m.mode() & bits) as i32,
        }
    }

    fn attributes(path: &[u8]) -> i32 {
        let ro = Self::is_read_only(path);
        if ro < 0 {
            return ro;
        }
        let d = Self::mode_test(path, S_IFDIR);
        if d < 0 {
            return d;
        }
        (if ro == 1 { FS_ATTR_READ_ONLY } else { 0 }) | (if d as u32 == S_IFDIR { FS_ATTR_DIR } else { 0 })
    }

    // ------------------------------------------------------- file handles

    fn open(&mut self, raw_path: &[u8], flags: u32) -> i32 {
        let host = match self.host_path(raw_path) {
            Ok((h, _)) => h,
            Err(FsErr(c)) => return c,
        };
        let mut oo = OpenOptions::new();
        oo.read(true).mode(0o660);
        if flags & FS_READ_ONLY == 0 {
            oo.write(true);
        }
        if flags & FS_CREATE != 0 {
            oo.write(true).create(true);
        }
        if flags & FS_CREATE_ALWAYS != 0 {
            oo.write(true).create(true).truncate(true);
        }
        if flags & FS_NONBLOCK_MODE != 0 {
            oo.custom_flags(libc::O_NONBLOCK);
        }
        let Some(h) = self.alloc_handle() else {
            return FS_TOO_MANY_FILES;
        };
        match oo.open(path_os(&host)) {
            Ok(f) => {
                if verbose() {
                    log(&format!("fs: O: {} flags {flags:#x} -> {h}", String::from_utf8_lossy(&host)));
                }
                let fd = f.into_raw_fd();
                self.handles.insert(h, Handle { path: host, kind: Kind::File { fd, flags, dirty: false } });
                h as i32
            }
            Err(e) => {
                if verbose() {
                    log(&format!("fs: Open: fail open {}: {e} (flags {flags:#x})", String::from_utf8_lossy(&host)));
                }
                errno_to_fs(&e)
            }
        }
    }

    fn seek(&mut self, h: u32, offset: i32, whence: u32) -> Result<i32, FsErr> {
        let fd = self.file_fd(h)?;
        if whence >= 3 {
            return Ok(FS_PARAM_ERROR);
        }
        // SAFETY: lseek on a file descriptor this server owns
        let r = unsafe { libc::lseek(fd, offset as libc::off_t, whence as i32) };
        Ok(if r < 0 { last_err() } else { r as i32 })
    }

    fn read(&mut self, h: u32, len: u32) -> Result<(i32, Vec<u8>), FsErr> {
        let fd = self.file_fd(h)?;
        let mut buf = vec![0u8; (len as usize).min(READ_MAX)];
        // SAFETY: reads into a buffer of the given length
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            return Ok((last_err(), vec![]));
        }
        buf.truncate(n as usize);
        Ok((0, buf))
    }

    fn write(&mut self, h: u32, data: &[u8]) -> Result<(i32, i32), FsErr> {
        let hd = self.get(h)?;
        let Kind::File { fd, ref mut dirty, .. } = hd.kind else {
            return Ok((FS_GENERAL_FAILURE, -1));
        };
        // SAFETY: writes from a buffer of the given length
        let n = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
        if n < 0 {
            return Ok((last_err(), -1));
        }
        *dirty = true;
        Ok((0, n as i32))
    }

    fn file_size(&mut self, h: u32) -> Result<(i32, u32), FsErr> {
        let fd = self.file_fd(h)?;
        // SAFETY: fstat into a zeroed stat buffer
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut st) } < 0 {
            return Ok((last_err(), 0));
        }
        Ok((0, st.st_size as u32))
    }

    fn close(&mut self, h: u32) -> Result<i32, FsErr> {
        self.get(h)?;
        let hd = self.handles.remove(&h).unwrap();
        if let Kind::File { fd, flags, dirty } = hd.kind {
            // SAFETY: fsync/close of a descriptor this server owns
            if (flags & FSYNC_ON_CLOSE != 0 || (self.extra_sync && dirty)) && unsafe { libc::fsync(fd) } < 0 {
                log(&format!(
                    "fs: Close: fsync {} failed: {}",
                    String::from_utf8_lossy(&hd.path),
                    io::Error::last_os_error()
                ));
            }
            if unsafe { libc::close(fd) } < 0 {
                return Ok(last_err());
            }
            if flags & (FS_CREATE | FS_CREATE_ALWAYS) != 0 {
                self.sync_dir(&hd.path);
            }
        }
        Ok(0)
    }

    fn close_all(&mut self) -> i32 {
        while let Some((h, hd)) = self.handles.pop_first() {
            if let Kind::File { fd, .. } = hd.kind {
                // no fsync in stock CloseAll
                // SAFETY: close of a descriptor this server owns
                if unsafe { libc::close(fd) } < 0 {
                    let _ = h;
                    return last_err();
                }
            }
        }
        0
    }

    // ------------------------------------------------------------ ops: io

    fn op_open(&mut self, a: &Args) -> OpResult {
        Ok(vec![i32p(self.open(a.raw(0)?, a.u32(1)?))])
    }

    fn op_open_hint(&mut self, a: &Args) -> OpResult {
        let ret = self.open(a.raw(0)?, a.u32(1)?);
        let mut hint = a.raw(2)?.iter().copied().take(8).collect::<Vec<_>>();
        hint.resize(8, 0);
        Ok(vec![i32p(ret), hint])
    }

    fn op_seek(&mut self, a: &Args) -> OpResult {
        let ret = self.seek(a.u32(0)?, a.u32(1)? as i32, a.u32(2)?).unwrap_or_else(|e| e.0);
        Ok(vec![i32p(ret)])
    }

    fn op_read(&mut self, a: &Args) -> OpResult {
        let (ret, data) = self.read(a.u32(0)?, a.u32(1)?).unwrap_or_else(|e| (e.0, vec![]));
        Ok(vec![i32p(ret), u32p(data.len() as u32), data])
    }

    fn op_write(&mut self, a: &Args) -> OpResult {
        let len = a.u32(2)? as usize;
        let data = a.raw(1)?;
        let data = &data[..len.min(data.len())];
        let (ret, n) = self.write(a.u32(0)?, data).unwrap_or_else(|e| (e.0, 0));
        Ok(vec![i32p(ret), i32p(n)])
    }

    fn op_close(&mut self, a: &Args) -> OpResult {
        Ok(vec![i32p(self.close(a.u32(0)?).unwrap_or_else(|e| e.0))])
    }

    fn op_get_file_size(&mut self, a: &Args) -> OpResult {
        let (ret, size) = self.file_size(a.u32(0)?).unwrap_or_else(|e| (e.0, 0));
        Ok(vec![i32p(ret), u32p(size)])
    }

    // ----------------------------------------------------- ops: namespace

    /// Convert a path parameter and run `f(host, letter)`
    fn path_op<F>(&mut self, raw: &[u8], f: F) -> i32
    where
        F: FnOnce(&mut Self, &[u8], u8) -> Result<i32, PathErr>,
    {
        match self.host_path(raw) {
            Err(FsErr(c)) => c,
            Ok((host, letter)) => match f(self, &host, letter) {
                Ok(v) => v,
                Err(PathErr::Fs(c)) => c,
                Err(PathErr::Io(e)) => errno_to_fs(&e),
            },
        }
    }

    fn op_create_dir(&mut self, a: &Args) -> OpResult {
        let r = self.path_op(a.raw(0)?, |s, host, letter| {
            if letter == b'U' {
                fs::DirBuilder::new().recursive(true).mode(0o770).create(path_os(&s.bases[&b'U']))?;
            }
            fs::DirBuilder::new().mode(0o770).create(path_os(host))?;
            s.sync_dir(host);
            Ok(0)
        });
        Ok(vec![i32p(r)])
    }

    fn op_remove_dir(&mut self, a: &Args) -> OpResult {
        let r = self.path_op(a.raw(0)?, |s, host, _| {
            fs::remove_dir(path_os(host))?;
            s.sync_dir(host);
            Ok(0)
        });
        Ok(vec![i32p(r)])
    }

    fn delete(&mut self, raw: &[u8]) -> i32 {
        self.path_op(raw, |s, host, _| {
            fs::remove_file(path_os(host))?;
            s.sync_dir(host);
            Ok(0)
        })
    }

    fn op_rename(&mut self, a: &Args) -> OpResult {
        // like stock: the source path is checked before the target exists
        let src = match self.host_path(a.raw(0)?) {
            Ok((h, _)) => h,
            Err(FsErr(c)) => return Ok(vec![i32p(c)]),
        };
        let dst = match self.host_path(a.raw(1)?) {
            Ok((h, _)) => h,
            Err(FsErr(c)) => return Ok(vec![i32p(c)]),
        };
        if let Err(e) = fs::rename(path_os(&src), path_os(&dst)) {
            return Ok(vec![i32p(errno_to_fs(&e))]);
        }
        self.sync_dir(&dst);
        if dirname(&src) != dirname(&dst) {
            self.sync_dir(&src);
        }
        Ok(vec![i32p(0)])
    }

    fn op_get_attributes(&mut self, a: &Args) -> OpResult {
        Ok(vec![i32p(self.path_op(a.raw(0)?, |_, host, _| Ok(Self::attributes(host))))])
    }

    fn op_get_file_detail(&mut self, a: &Args) -> OpResult {
        let mut detail = vec![0u8; 0x18];
        let r = self.path_op(a.raw(0)?, |_, host, _| {
            let m = stat(host)?;
            detail.clear();
            for t in [m.mtime(), m.atime(), m.ctime()] {
                detail.extend_from_slice(&(t as u64).to_le_bytes());
            }
            Ok(0)
        });
        Ok(vec![i32p(r), detail])
    }

    fn op_restore(&mut self, a: &Args) -> OpResult {
        // stock restores the file from the nvram "bin region"; not available
        let r = self.path_op(a.raw(0)?, |_, host, _| {
            log(&format!("fs: Restore Fail: {} (no bin region)", String::from_utf8_lossy(host)));
            Ok(FS_GENERAL_FAILURE)
        });
        Ok(vec![i32p(r)])
    }

    // --------------------------------------------------- ops: directories

    /// FS_RecursiveSearch -> (entry count or error, clusters)
    fn recursive_search(&self, path: &[u8], mode: u32) -> (i32, i64) {
        let cs = self.cluster_size(b'Z' as u32) as i64;
        let (mut count, mut clusters) = (0i32, 0i64);
        let entries = match fs::read_dir(path_os(path)) {
            Ok(r) => r.filter_map(Result::ok).collect::<Vec<_>>(),
            Err(e) => {
                log(&format!("fs: RecursiveSearch: fail open dir {}: {e}", String::from_utf8_lossy(path)));
                return (0, 0);
            }
        };
        for ent in entries {
            let name = ent.file_name();
            let name = name.as_bytes();
            if path.len() + name.len() >= PATH_MAX - 1 {
                return (FS_PATH_OVER_LEN_ERROR, clusters);
            }
            let full = [path, b"/", name].concat();
            let Ok(m) = stat(&full) else { continue };
            if m.is_dir() {
                if mode & FS_DIR_TYPE != 0 {
                    count += 1;
                }
                if mode & FS_RECURSIVE_TYPE != 0 {
                    let (n, c) = self.recursive_search(&full, mode);
                    if n < 0 {
                        return (n, clusters);
                    }
                    count += n;
                    clusters += c;
                }
            } else if mode & FS_FILE_TYPE != 0 && m.is_file() {
                if m.size() > 0 && cs > 0 {
                    clusters += m.size() as i64 / cs + 1;
                }
                count += 1;
            }
        }
        (count, clusters)
    }

    fn require_dir(host: &[u8]) -> Result<(), PathErr> {
        if !stat(host)?.is_dir() {
            return Err(PathErr::Fs(FS_PARAM_ERROR));
        }
        Ok(())
    }

    fn op_count(&mut self, a: &Args) -> OpResult {
        let mode = a.u32(1)?;
        let r = self.path_op(a.raw(0)?, |s, host, _| {
            Self::require_dir(host)?;
            Ok(s.recursive_search(host, mode).0)
        });
        Ok(vec![i32p(r)])
    }

    fn op_get_folder_size(&mut self, a: &Args) -> OpResult {
        let flag = a.u32(1)?;
        let r = self.path_op(a.raw(0)?, |s, host, letter| {
            let cs = s.cluster_size(letter as u32) as i64;
            Self::require_dir(host)?;
            let (n, clusters) = s.recursive_search(host, FS_FILE_TYPE | FS_DIR_TYPE | FS_RECURSIVE_TYPE);
            if n < 0 {
                return Ok(n);
            }
            Ok((clusters * if flag == 2 { 1 } else { cs }) as i32)
        });
        Ok(vec![i32p(r)])
    }

    /// FS_RecursiveDelete -> number of removed entries or error
    fn recursive_delete(&self, path: &[u8], flag: u32) -> i32 {
        let mut n = 0;
        let entries = match fs::read_dir(path_os(path)) {
            Ok(r) => r.filter_map(Result::ok).collect::<Vec<_>>(),
            Err(e) => {
                log(&format!("fs: RecursiveDelete: fail open dir {}: {e}", String::from_utf8_lossy(path)));
                return 0;
            }
        };
        for ent in entries {
            let name = ent.file_name();
            let name = name.as_bytes();
            if path.len() + name.len() >= PATH_MAX - 1 {
                return FS_PATH_OVER_LEN_ERROR;
            }
            let full = [path, b"/", name].concat();
            let Ok(m) = stat(&full) else { continue };
            let r: io::Result<()> = (|| {
                if m.is_dir() {
                    if flag & FS_RECURSIVE_TYPE != 0 {
                        let r = self.recursive_delete(&full, flag);
                        if r < 0 {
                            return Err(io::Error::other(FsCode(r)));
                        }
                        n += r;
                    }
                    if flag & FS_DIR_TYPE != 0 {
                        fs::remove_dir(path_os(&full))?;
                        n += 1;
                    }
                } else if flag & FS_FILE_TYPE != 0 && m.is_file() {
                    fs::remove_file(path_os(&full))?;
                    n += 1;
                }
                Ok(())
            })();
            if let Err(e) = r {
                return match e.get_ref().and_then(|i| i.downcast_ref::<FsCode>()) {
                    Some(FsCode(c)) => *c,
                    None => errno_to_fs(&e),
                };
            }
        }
        self.sync_dir(&[path, b"/."].concat());
        n
    }

    fn xdelete(&mut self, raw: &[u8], flag: u32) -> i32 {
        self.path_op(raw, |s, host, _| {
            if flag & 0x1c == 0 || flag & !0x1c != 0 {
                return Ok(FS_PARAM_ERROR);
            }
            Self::require_dir(host)?;
            let mut n = s.recursive_delete(host, flag);
            if flag & (FS_DIR_TYPE | FS_RECURSIVE_TYPE) == FS_DIR_TYPE | FS_RECURSIVE_TYPE {
                fs::remove_dir(path_os(host))?;
                s.sync_dir(host);
                n += 1;
            }
            Ok(n)
        })
    }

    fn op_move(&mut self, a: &Args) -> OpResult {
        Ok(vec![i32p(self.mv(a.raw(0)?, a.raw(1)?, a.u32(2)?))])
    }

    fn mv(&mut self, raw_src: &[u8], raw_dst: &[u8], flag: u32) -> i32 {
        let (src, dst) = match (self.host_path(raw_src), self.host_path(raw_dst)) {
            (Ok(s), Ok(d)) => (s.0, d.0),
            (Err(FsErr(c)), _) | (_, Err(FsErr(c))) => return c,
        };
        if src == dst || flag & 3 == 0 || flag & 0xfffe_fffc != 0 {
            return FS_PARAM_ERROR;
        }
        if flag & FS_MOVE_KILL != 0 && Self::is_read_only(&src) > 0 {
            return FS_READ_ONLY_ERROR;
        }
        let mut ret = FS_GENERAL_FAILURE;
        if flag & FS_MOVE_OVERWRITE != 0 {
            if Self::is_read_only(&dst) > 0 {
                return FS_READ_ONLY_ERROR;
            }
            if Self::mode_test(&dst, S_IFDIR) > 0 {
                ret = self.xdelete(raw_dst, 0x1c);
                if ret < 0 {
                    return ret;
                }
            } else if Self::mode_test(&dst, S_IFREG) > 0 {
                ret = self.delete(raw_dst);
                if ret < 0 {
                    return ret;
                }
            }
        }
        // stock only logs "kill %s, NULL" for FS_MOVE_KILL: the source stays
        if flag & FS_MOVE_COPY == 0 {
            return ret;
        }
        let data = match fs::read(path_os(&src)) {
            Ok(d) => d,
            Err(e) => return errno_to_fs(&e),
        };
        let f = match OpenOptions::new().write(true).create(true).truncate(true).mode(0o770).open(path_os(&dst)) {
            Ok(f) => f,
            Err(e) => return errno_to_fs(&e),
        };
        // SAFETY: one write from a buffer of the given length
        let n = unsafe { libc::write(f.as_raw_fd(), data.as_ptr().cast(), data.len()) };
        if n < 0 {
            return last_err();
        }
        if self.extra_sync {
            if let Err(e) = f.sync_all() {
                return errno_to_fs(&e);
            }
        }
        drop(f);
        self.sync_dir(&dst);
        if n as usize >= data.len() {
            0
        } else {
            FS_ERROR_RESERVED
        }
    }

    // ------------------------------------------------------- ops: search

    /// FS_EntryLinuxToDos -> 0x34-byte FS_DOSDirEntry
    fn dos_entry(path: &[u8], name_len: usize, maxlen: u32) -> Vec<u8> {
        let mut e = vec![0u8; DOS_DIR_ENTRY_SIZE];
        let (mtime, size) = match stat(path) {
            Ok(m) => (m.mtime(), m.size()),
            Err(err) => {
                log(&format!("fs: EntryLinuxToDos: stat {} failed: {err}", String::from_utf8_lossy(path)));
                (0, 0)
            }
        };
        e[0x0b] = Self::attributes(path) as u8;
        e[0x0c] = (name_len < maxlen as usize) as u8;
        // stock calls localtime() for atime then mtime and uses the shared
        // static result, so both date/time fields carry the mtime
        let dt = dos_datetime(mtime).to_le_bytes();
        e[0x0e..0x12].copy_from_slice(&dt);
        e[0x16..0x1a].copy_from_slice(&dt);
        e[0x1c..0x20].copy_from_slice(&(size as u32).to_le_bytes());
        e
    }

    /// FS_ConvCsToWcs + reply length rule: count*2+2 bytes, or empty
    fn cs_to_wcs(name: &[u8], maxlen: u32) -> Vec<u8> {
        let maxlen = maxlen as usize;
        if maxlen == 0 {
            return vec![];
        }
        let count = if name.len() < maxlen { name.len() } else { maxlen - 1 };
        if count == 0 {
            return vec![];
        }
        let mut out = Vec::with_capacity(count * 2 + 2);
        for &c in &name[..count] {
            out.push(if c == b'/' { b'\\' } else { c });
            out.push(0);
        }
        out.extend_from_slice(&[0, 0]);
        out
    }

    /// Advance a search: (ret, entry, name), ret 0 = found
    fn next_match(hd: &mut Handle, maxlen: u32) -> (i32, Vec<u8>, Vec<u8>) {
        let Kind::Search { iter, dir, pattern, attr, mask } = &mut hd.kind else {
            return (FS_INVALID_FILE_HANDLE, vec![0; DOS_DIR_ENTRY_SIZE], vec![]);
        };
        for ent in iter.by_ref().filter_map(Result::ok) {
            let name = ent.file_name();
            let name = name.as_bytes();
            if !pat_match(pattern, name) {
                continue;
            }
            if dir.len() + 1 + name.len() > PATH_MAX {
                return (FS_PATH_OVER_LEN_ERROR, vec![0; DOS_DIR_ENTRY_SIZE], vec![]);
            }
            let full = [dir.as_slice(), b"/", name].concat();
            let fa = Self::attributes(&full) as u32;
            if *attr & !fa != 0 || fa & *mask != 0 {
                continue;
            }
            let entry = Self::dos_entry(&full, name.len(), maxlen);
            let wname = Self::cs_to_wcs(name, maxlen);
            hd.path = full;
            return (0, entry, wname);
        }
        (FS_NO_MORE_FILES, vec![0; DOS_DIR_ENTRY_SIZE], vec![])
    }

    fn op_find_first(&mut self, a: &Args) -> OpResult {
        let (attr, mask, maxlen) = (a.u8(1)?, a.u8(2)?, a.u32(3)?);
        let empty = vec![0u8; DOS_DIR_ENTRY_SIZE];
        let host = match self.host_path(a.raw(0)?) {
            Ok((h, _)) => h,
            Err(FsErr(c)) => return Ok(vec![i32p(c), empty, vec![]]),
        };
        let slash = match host.iter().rposition(|&c| c == b'/') {
            Some(s) if s >= 1 => s,
            _ => return Ok(vec![i32p(FS_PATH_NOT_FOUND), empty, vec![]]),
        };
        let (dir, pattern) = (host[..slash].to_vec(), host[slash + 1..].to_vec());
        if pattern.len() >= 0x100 {
            return Ok(vec![i32p(FS_PATH_OVER_LEN_ERROR), empty, vec![]]);
        }
        let iter = match fs::read_dir(path_os(&dir)) {
            Ok(i) => i,
            Err(e) => return Ok(vec![i32p(errno_to_fs(&e)), empty, vec![]]),
        };
        let mut hd = Handle { path: dir.clone(), kind: Kind::Search { iter, dir, pattern, attr, mask } };
        let (ret, entry, name) = Self::next_match(&mut hd, maxlen);
        if ret < 0 {
            return Ok(vec![i32p(ret), entry, vec![]]);
        }
        let Some(h) = self.alloc_handle() else {
            return Ok(vec![i32p(FS_TOO_MANY_FILES), empty, vec![]]);
        };
        self.handles.insert(h, hd);
        Ok(vec![i32p(h as i32), entry, name])
    }

    fn op_find_next(&mut self, a: &Args) -> OpResult {
        let maxlen = a.u32(1)?;
        let empty = vec![0u8; DOS_DIR_ENTRY_SIZE];
        let hd = match self.get(a.u32(0)?) {
            Ok(hd) => hd,
            Err(FsErr(c)) => return Ok(vec![i32p(c), empty, vec![]]),
        };
        let (ret, entry, name) = Self::next_match(hd, maxlen);
        Ok(vec![i32p(ret), entry, name])
    }

    fn op_find_close(&mut self, a: &Args) -> OpResult {
        let h = a.u32(0)?;
        if let Err(FsErr(c)) = self.get(h) {
            return Ok(vec![i32p(c)]);
        }
        let hd = self.handles.remove(&h).unwrap();
        if let Kind::File { fd, .. } = hd.kind {
            // SAFETY: close of a descriptor this server owns
            if unsafe { libc::close(fd) } < 0 {
                return Ok(vec![i32p(last_err())]);
            }
        }
        Ok(vec![i32p(0)])
    }

    // ------------------------------------------------------- ops: volume

    fn op_get_disk_info(&self) -> Params {
        let mut info = vec![0u8; DISK_INFO_SIZE];
        let st = match statvfs(&self.bases[&b'Z']) {
            Ok(st) => st,
            Err(e) => {
                log(&format!("fs: GetDiskInfo: statfs failed: {e}"));
                return vec![i32p(FS_ERROR_RESERVED), info];
            }
        };
        let mut put = |off: usize, v: u32| info[off..off + 4].copy_from_slice(&v.to_le_bytes());
        put(0x30, 0x200); // BytesPerSector
        put(0x34, (st.f_bsize >> 9) as u32); // SectorsPerCluster
        put(0x38, st.f_blocks as u32); // TotalClusters
        put(0x40, 0x200); // FreeClusters (sic)
        vec![i32p(0), info]
    }

    fn op_get_drive(&mut self, a: &Args) -> OpResult {
        let (typ, serial, alt) = (a.u32(0)?, a.u32(1)?, a.u32(2)?);
        let invalid = typ & 0x1c == 0
            || typ & 0xffff_ffe3 != 0
            || (typ != 0x10 && serial > 2)
            || serial == 0
            || (typ == 0x10 && serial != 1)
            || (alt != 1 && alt & 1 != 0)
            || (alt != 2 && alt & 2 != 0);
        if invalid || typ != 4 {
            return Ok(vec![i32p(FS_PARAM_ERROR)]);
        }
        Ok(vec![i32p(b'Z' as i32)])
    }

    // --------------------------------------------------------- ops: CMPT
    // Request structures are arrays of u32 words (32-bit modem structs):
    //   read  (0x1022/0x1026): [0] opid_map, [3] open flags, [4] file size,
    //         [5] offset, [6] whence, [8] length, [10] file handle (EX only)
    //   write (0x1024): [0] opid_map, [3] open flags, [4] offset, [5] whence,
    //         [7] length, [8] file handle (< 0: open the file first)
    // Each reply starts with an 8-byte {u32 done_opid_map, s32 error}.

    fn words(raw: &[u8]) -> [u32; 16] {
        let mut w = [0u32; 16];
        for (i, c) in raw.chunks(4).take(16).enumerate() {
            let mut b = [0u8; 4];
            b[..c.len()].copy_from_slice(c);
            w[i] = u32::from_le_bytes(b);
        }
        w
    }

    fn done_err(done: u32, err: i32) -> Vec<u8> {
        [done.to_le_bytes(), err.to_le_bytes()].concat()
    }

    /// Shared GETSIZE/SEEK/READ part of CMPT_Read(_EX):
    /// (ok, done, err, size, data); on failure the file is closed.
    fn cmpt_read_steps(&mut self, h: u32, w: &[u32; 16], mut done: u32) -> Result<(bool, u32, i32, u32, Vec<u8>), FsErr> {
        let (mut size, mut data) = (w[4], vec![]);
        for bit in [CMPT_GETSIZE, CMPT_SEEK, CMPT_READ] {
            if w[0] & bit == 0 {
                continue;
            }
            let ret = match bit {
                CMPT_GETSIZE => {
                    let (r, s) = self.file_size(h)?;
                    size = s;
                    r
                }
                CMPT_SEEK => self.seek(h, w[5] as i32, w[6])?,
                _ => {
                    let (r, d) = self.read(h, w[8])?;
                    data = d;
                    r
                }
            };
            if ret < 0 {
                self.close(h)?;
                return Ok((false, done, ret, size, vec![]));
            }
            done |= bit;
        }
        Ok((true, done, 0, size, data))
    }

    fn op_cmpt_read(&mut self, a: &Args) -> OpResult {
        let w = Self::words(a.raw(1)?);
        let (size, data, done, mut err);
        if w[0] & CMPT_OPEN == 0 {
            (size, data, done, err) = (w[4], vec![], 0, FS_GENERAL_FAILURE);
        } else {
            let h = self.open(a.raw(0)?, w[3]);
            if h < 0 {
                (size, data, done, err) = (w[4], vec![], 0, h);
            } else {
                let h = h as u32;
                let (ok, mut d, e, s, dt) = match self.cmpt_read_steps(h, &w, CMPT_OPEN) {
                    Ok(r) => r,
                    Err(FsErr(c)) => return Ok(vec![i32p(c)]),
                };
                (size, data, err) = (s, dt, e);
                if ok {
                    let ret = self.close(h).unwrap_or_else(|e| e.0);
                    if w[0] & CMPT_CLOSE == 0 {
                        log("fs: cmpt_Read, miss close flag");
                    } else if ret < 0 {
                        err = ret;
                    } else {
                        d |= CMPT_CLOSE;
                    }
                }
                done = d;
            }
        }
        let n = data.len() as u32;
        Ok(vec![Self::done_err(done, err), u32p(size), u32p(n), data])
    }

    fn op_cmpt_read_ex(&mut self, a: &Args) -> OpResult {
        let w = Self::words(a.raw(1)?);
        let (mut size, mut data, mut h_out) = (w[4], vec![], -1i32);
        let (h, mut done) = if w[0] & CMPT_OPEN != 0 {
            (self.open(a.raw(0)?, w[3]), CMPT_OPEN)
        } else {
            (w[10] as i32, 0)
        };
        let mut err = FS_ERROR_RESERVED;
        if h < 0 {
            done = 0;
            err = if w[0] & CMPT_OPEN != 0 { h } else { FS_ERROR_RESERVED };
        } else {
            let hu = h as u32;
            let r: Result<(), FsErr> = (|| {
                let (mut ok, d, e, s, dt) = self.cmpt_read_steps(hu, &w, done)?;
                (done, err, size, data) = (d, e, s, dt);
                if ok && w[0] & CMPT_CLOSE != 0 {
                    let ret = self.close(hu)?;
                    if ret < 0 {
                        ok = false;
                        err = ret;
                    } else {
                        done |= CMPT_CLOSE;
                    }
                }
                h_out = if ok { h } else { -1 };
                Ok(())
            })();
            err = match r {
                Ok(()) => err,
                Err(FsErr(c)) => c,
            };
        }
        let n = data.len() as u32;
        Ok(vec![Self::done_err(done, err), u32p(size), u32p(n), data, i32p(h_out)])
    }

    fn op_cmpt_write(&mut self, a: &Args) -> OpResult {
        let w = Self::words(a.raw(1)?);
        let payload = a.raw(2).unwrap_or(&[]);
        let (mut h, mut written, mut done) = (w[8] as i32, 0i32, 0u32);
        if h < 0 {
            if w[0] & CMPT_OPEN == 0 {
                return Ok(vec![Self::done_err(0, FS_PARAM_ERROR), i32p(-1), i32p(0)]);
            }
            h = self.open(a.raw(0)?, w[3]);
            if h < 0 {
                return Ok(vec![Self::done_err(0, h), i32p(-1), i32p(0)]);
            }
            done = CMPT_OPEN;
        }
        let hu = h as u32;
        let r: Result<Option<Params>, FsErr> = (|| {
            if w[0] & CMPT_SEEK != 0 {
                let ret = self.seek(hu, w[4] as i32, w[5])?;
                if ret < 0 {
                    self.close(hu)?;
                    return Ok(Some(vec![Self::done_err(done, ret), i32p(h), i32p(0)]));
                }
                done |= CMPT_SEEK;
            }
            if w[0] & CMPT_WRITE != 0 {
                let len = (w[7] as usize).min(payload.len());
                let (ret, n) = self.write(hu, &payload[..len])?;
                written = n;
                if ret < 0 {
                    self.close(hu)?;
                    return Ok(Some(vec![Self::done_err(done, ret), i32p(h), i32p(written)]));
                }
                done |= CMPT_WRITE;
            }
            if w[0] & CMPT_CLOSE != 0 {
                let ret = self.close(hu)?;
                if ret < 0 {
                    return Ok(Some(vec![Self::done_err(done, ret), i32p(h), i32p(written)]));
                }
                done |= CMPT_CLOSE;
            }
            Ok(None)
        })();
        let err = match r {
            Ok(Some(p)) => return Ok(p),
            Ok(None) => 0,
            Err(FsErr(c)) => c,
        };
        Ok(vec![Self::done_err(done, err), i32p(h), i32p(written)])
    }
}

/// Error of a path operation: an FS code or an OS error mapped by errno
enum PathErr {
    Fs(i32),
    Io(io::Error),
}

impl From<io::Error> for PathErr {
    fn from(e: io::Error) -> Self {
        PathErr::Io(e)
    }
}

/// An FS code tunnelled through io::Error inside recursive_delete
#[derive(Debug)]
struct FsCode(i32);

impl std::fmt::Display for FsCode {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "fs error {}", self.0)
    }
}

impl std::error::Error for FsCode {}

fn statvfs(p: &[u8]) -> io::Result<libc::statvfs> {
    let c = CString::new(p).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: statvfs into a zeroed buffer
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wpath(s: &str) -> Vec<u8> {
        s.bytes().flat_map(|c| [c, 0]).chain([0, 0]).collect()
    }

    #[test]
    fn wildcards() {
        assert!(pat_match(b"*", b"abc"));
        assert!(pat_match(b"LD0B_*", b"LD0B_001"));
        assert!(pat_match(b"?D*1", b"LD0B_001"));
        assert!(!pat_match(b"LD0C*", b"LD0B_001"));
        assert!(pat_match(b"", b""));
    }

    #[test]
    fn paths() {
        assert_eq!(FsServer::wcs_to_cs(&wpath("Z:\\NVRAM\\A")).unwrap(), b"Z:/NVRAM/A");
        assert!(FsServer::wcs_to_cs(&wpath("Z:\\..\\x")).is_err());
        assert_eq!(FsServer::cs_to_wcs(b"ab/c", 3), b"a\0b\0\0\0");
        assert!(FsServer::cs_to_wcs(b"abc", 1).is_empty());
    }

    #[test]
    fn file_ops() {
        let dir = std::env::temp_dir().join(format!("modemd-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut s = FsServer::new(dir.to_str().unwrap());
        let p = wpath("Z:\\F1");
        let h = s.handle(0x1001, &[p.clone(), u32p(FS_CREATE_ALWAYS)]);
        let h = u32::from_le_bytes(h[0][..4].try_into().unwrap());
        assert_eq!(h, 1);
        let r = s.handle(0x1004, &[u32p(h), b"hello".to_vec(), u32p(5)]);
        assert_eq!(r, vec![i32p(0), i32p(5)]);
        assert_eq!(s.handle(0x1002, &[u32p(h), u32p(1), u32p(0)]), vec![i32p(1)]);
        let r = s.handle(0x1003, &[u32p(h), u32p(10)]);
        assert_eq!(r, vec![i32p(0), u32p(4), b"ello".to_vec()]);
        assert_eq!(s.handle(0x1009, &[u32p(h)]), vec![i32p(0), u32p(5)]);
        assert_eq!(s.handle(0x1005, &[u32p(h)]), vec![i32p(0)]);
        assert_eq!(s.handle(0x1005, &[u32p(h)]), vec![i32p(FS_INVALID_FILE_HANDLE)]);
        // find
        let r = s.handle(0x1012, &[wpath("Z:\\F*"), vec![0], vec![0], u32p(16)]);
        assert_eq!(r[0], i32p(1));
        assert_eq!(r[2], b"F\x001\x00\x00\x00");
        assert_eq!(s.handle(0x1013, &[u32p(1), u32p(16)])[0], i32p(FS_NO_MORE_FILES));
        assert_eq!(s.handle(0x1014, &[u32p(1)]), vec![i32p(0)]);
        // CMPT read of the whole file
        let mut req = vec![0u8; 64];
        req[0] = (CMPT_OPEN | CMPT_GETSIZE | CMPT_READ | CMPT_CLOSE) as u8;
        req[12..16].copy_from_slice(&FS_READ_ONLY.to_le_bytes());
        req[32..36].copy_from_slice(&100u32.to_le_bytes());
        let r = s.handle(0x1022, &[p.clone(), req]);
        assert_eq!(r[0], FsServer::done_err(0x1b, 0));
        assert_eq!(r[1], u32p(5));
        assert_eq!(r[3], b"hello");
        // missing parameters, unknown ops, bad drives
        assert_eq!(s.handle(0x1001, &[]), vec![i32p(FS_PARAM_ERROR)]);
        assert_eq!(s.handle(0x1999, &[]), vec![i32p(FS_PARAM_ERROR)]);
        assert_eq!(s.handle(0x1001, &[wpath("A:\\x"), u32p(0)]), vec![i32p(FS_PATH_NOT_FOUND)]);
        assert_eq!(s.handle(0x1001, &[wpath("Z:\\nope"), u32p(0)]), vec![i32p(FS_FILE_NOT_FOUND)]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
