#!/usr/bin/env python3
"""Differential test: ccci_fs.py FsServer vs plinius-modemd (debug replay).

Runs the same scripted and random FS requests through both servers in
lockstep and compares the replies byte for byte (time stamps masked) and
the resulting directory trees.

    cd modemd && cargo build && python3 fsdiff.py .. target/debug/plinius-modemd /tmp/fsdiff

Usage: fsdiff.py SCRIPTS_DIR MODEMD_DEBUG_BIN WORKDIR [N_RANDOM] [SEED]
"""
import os
import random
import shutil
import struct
import subprocess
import sys

scripts, binary, work = sys.argv[1:4]
n_random = int(sys.argv[4]) if len(sys.argv) > 4 else 3000
seed = int(sys.argv[5]) if len(sys.argv) > 5 else 1
sys.path.insert(0, scripts)
import logging  # noqa: E402
logging.disable(logging.CRITICAL)
from ccci_fs import FsServer  # noqa: E402

RESP = 0xffff0000


def build(op, params):
    out = bytearray(struct.pack('<II', op, len(params)))
    for p in params:
        out += struct.pack('<I', len(p)) + p
        out += b'\0' * (-len(p) & 3)
    return bytes(out)


def parse(buf):
    op, n = struct.unpack_from('<II', buf)
    off, args = 8, []
    for _ in range(n):
        ln, = struct.unpack_from('<I', buf, off)
        args.append(buf[off + 4:off + 4 + ln])
        off += 4 + ((ln + 3) & ~3)
    return op, args


def w(s):
    return s.encode('latin1').decode('latin1').encode('utf-16-le') + b'\0\0'


u32 = lambda v: struct.pack('<I', v & 0xffffffff)  # noqa: E731

shutil.rmtree(work, ignore_errors=True)
proot, rroot = os.path.join(work, 'py'), os.path.join(work, 'rs')
os.makedirs(proot)
os.makedirs(rroot)
py = FsServer(proot)
rs = subprocess.Popen([binary], env={**os.environ, 'MODEMD_FS_REPLAY': rroot},
                      stdin=subprocess.PIPE, stdout=subprocess.PIPE)


def rs_call(op, args):
    req = build(op, args)
    rs.stdin.write(struct.pack('<I', len(req)) + req)
    rs.stdin.flush()
    n, = struct.unpack('<I', rs.stdout.read(4))
    return rs.stdout.read(n)


def py_call(op, args):
    try:
        params = py.handle(op, args)
    except Exception:
        params = [struct.pack('<i', -2)]
    return build(op | RESP, params)


def normalize(op, reply):
    """mask time-dependent fields (mtime/atime/ctime)"""
    o, params = parse(reply)
    params = [bytearray(p) for p in params]
    if op in (0x1012, 0x1013) and len(params) > 1 and len(params[1]) == 0x34:
        params[1][0x0e:0x12] = b'TTTT'
        params[1][0x16:0x1a] = b'TTTT'
    if op == 0x101e and len(params) > 1:
        params[1] = params[1][:1 << 24]
    if op == 0x1025 and len(params) > 1:
        params[1] = b'T' * len(params[1])
    return o, [bytes(p) for p in params]


fails = 0


def check(op, args, label=''):
    global fails
    p, r = py_call(op, args), rs_call(op, args)
    if normalize(op, p) != normalize(op, r):
        fails += 1
        if fails <= 25:
            print(f'MISMATCH {label} op {op:#x} args {[a[:24].hex() for a in args]}')
            print('   py', [x[:40].hex() for x in parse(p)[1]])
            print('   rs', [x[:40].hex() for x in parse(r)[1]])
    return parse(p)[1]


def both(fn):
    for root in (proot, rroot):
        fn(root)


# ---------------------------------------------------------- scripted
FS_CREATE, FS_CREATE_ALWAYS, FS_READ_ONLY = 0x10000, 0x20000, 0x100
check(0x1007, [w('Z:\\NVRAM')], 'mkdir')
check(0x1007, [w('Z:\\NVRAM\\NVD_IMEI')], 'mkdir2')
check(0x1007, [w('Z:\\NVRAM')], 'mkdir-exists')
check(0x1007, [w('U:\\LPM')], 'mkdir-U')
h = check(0x1001, [w('Z:\\NVRAM\\NVD_IMEI\\LD0B_001'), u32(FS_CREATE_ALWAYS)], 'open')
check(0x1004, [h[0], b'0123456789' * 50, u32(500)], 'write')
check(0x1004, [h[0], b'abc', u32(10)], 'write-shortlen')
check(0x1002, [h[0], u32(-5), u32(2)], 'seek-end')
check(0x1002, [h[0], u32(0), u32(3)], 'seek-bad')
check(0x1003, [h[0], u32(100)], 'read')
check(0x1009, [h[0]], 'size')
check(0x1005, [h[0]], 'close')
check(0x1005, [h[0]], 'close-again')
check(0x1005, [u32(0)], 'close-0')
check(0x1005, [u32(500)], 'close-500')
check(0x1001, [w('Z:\\NVRAM\\..\\x'), u32(0)], 'dotdot')
check(0x1001, [w('A:\\x'), u32(0)], 'bad-drive')
check(0x1001, [w('Z'), u32(0)], 'short')
check(0x1001, [b'', u32(0)], 'empty')
check(0x1001, [w('Z:\\' + 'a' * 5000), u32(0)], 'long')
check(0x1011, [w('Z:\\NVRAM\\NVD_IMEI\\LD0B_001'), u32(0), b'HINT'], 'openhint')
check(0x1006, [], 'closeall')
check(0x1010, [w('Z:\\NVRAM\\NVD_IMEI\\LD0B_001')], 'attr-file')
check(0x1010, [w('Z:\\NVRAM')], 'attr-dir')
check(0x1010, [w('Z:\\nope')], 'attr-missing')
both(lambda r: os.chmod(os.path.join(r, 'nvdata/md/NVRAM/NVD_IMEI/LD0B_001'), 0o444))
check(0x1010, [w('Z:\\NVRAM\\NVD_IMEI\\LD0B_001')], 'attr-ro')
check(0x100c, [w('Z:\\NVRAM\\NVD_IMEI\\LD0B_001'), w('Z:\\NVRAM\\COPY'), u32(3)], 'move-kill-ro')
check(0x100c, [w('Z:\\NVRAM\\NVD_IMEI\\LD0B_001'), w('Z:\\NVRAM\\COPY'), u32(1)], 'move-copy')
check(0x100c, [w('Z:\\NVRAM\\NVD_IMEI\\LD0B_001'), w('Z:\\NVRAM\\COPY'), u32(0x10001)], 'move-ow')
check(0x100c, [w('Z:\\NVRAM\\COPY'), w('Z:\\NVRAM\\COPY'), u32(1)], 'move-same')
check(0x100c, [w('Z:\\NVRAM\\COPY'), w('Z:\\NVRAM\\C2'), u32(4)], 'move-badflag')
check(0x1007, [w('Z:\\ODIR')], 'mkdir-ODIR')
check(0x1007, [w('Z:\\ODIR\\IN')], 'mkdir-ODIR-IN')
check(0x100c, [w('Z:\\NVRAM\\COPY'), w('Z:\\ODIR'), u32(0x10001)], 'move-ow-dir')
both(lambda r: os.chmod(os.path.join(r, 'nvdata/md/NVRAM/NVD_IMEI/LD0B_001'), 0o660))
for i in range(5):
    hh = check(0x1001, [w(f'Z:\\D\\F{i}'), u32(FS_CREATE)], 'open-nodir')
check(0x1007, [w('Z:\\D')], 'mkdir-D')
check(0x1007, [w('Z:\\D\\SUB')], 'mkdir-SUB')
for i in range(5):
    hh = check(0x1001, [w(f'Z:\\D\\F{i}'), u32(FS_CREATE)], 'open-D')
    check(0x1004, [hh[0], b'x' * (i * 3000), u32(i * 3000)], 'write-D')
    check(0x1005, [hh[0]], 'close-D')
hh = check(0x1001, [w('Z:\\D\\SUB\\G'), u32(FS_CREATE)], 'open-G')
check(0x1005, [hh[0]], 'close-G')
for flag in (0, 4, 8, 0xc, 0x10, 0x14, 0x18, 0x1c):
    check(0x100d, [w('Z:\\D'), u32(flag)], f'count {flag}')
check(0x100d, [w('Z:\\D\\F1'), u32(4)], 'count-file')
for flag in (0, 1, 2):
    check(0x100a, [w('Z:\\D'), u32(flag)], f'foldersize {flag}')
for attr, mask in ((0, 0), (0x10, 0), (0, 0x10), (1, 0), (0x10, 0x10)):
    for pat in ('Z:\\D\\*', 'Z:\\D\\F?', 'Z:\\D\\Q*', 'Z:\\*', 'Z:\\D\\SUB'):
        for mx in (0, 1, 2, 3, 64):
            r = check(0x1012, [w(pat), bytes([attr]), bytes([mask]), u32(mx)], f'ff {pat} {attr} {mask} {mx}')
            ret = struct.unpack('<i', r[0])[0]
            if ret > 0:
                for _ in range(8):
                    check(0x1013, [r[0], u32(mx)], 'fn')
                check(0x1014, [r[0]], 'fc')
check(0x1012, [w('Z:'), b'\0', b'\0', u32(10)], 'ff-noslash')
check(0x1012, [w('Z:\\' + 'p' * 300), b'\0', b'\0', u32(10)], 'ff-longpat')
check(0x1012, [w('Z:\\nodir\\*'), b'\0', b'\0', u32(10)], 'ff-nodir')
check(0x1013, [u32(77), u32(10)], 'fn-bad')
check(0x1014, [u32(77)], 'fc-bad')
check(0x100b, [w('Z:\\D\\F1'), w('Z:\\D\\SUB\\F1R')], 'rename')
check(0x100b, [w('Z:\\D\\F1'), w('Z:\\D\\SUB\\F1R')], 'rename-missing')
check(0x100f, [w('Z:\\D\\F2')], 'delete')
check(0x100f, [w('Z:\\D\\F2')], 'delete-missing')
check(0x100f, [w('Z:\\D\\SUB')], 'delete-dir')
check(0x1008, [w('Z:\\D\\SUB')], 'rmdir-nonempty')
for flag in (0, 0x20, 4, 0x1c):
    check(0x1018, [w('Z:\\D'), u32(flag)], f'xdelete {flag}')
check(0x1018, [w('Z:\\D'), u32(0x1c)], 'xdelete-gone')
check(0x1008, [w('Z:\\NVRAM\\NVD_IMEI')], 'rmdir-nonempty2')
for t, s, a in ((4, 1, 1), (4, 1, 2), (4, 1, 3), (4, 3, 1), (4, 0, 1), (8, 1, 1), (0x10, 1, 1),
                (0x10, 2, 1), (0, 1, 1), (0x20, 1, 1), (4, 2, 0)):
    check(0x101a, [u32(t), u32(s), u32(a)], f'getdrive {t} {s} {a}')
for d in (0x50, 0x51, 0x5a, 0x5b, ord('W')):
    check(0x101b, [u32(d)], f'cluster {d:#x}')
check(0x100e, [], 'diskinfo')
for op in (0x1015, 0x1016, 0x1019, 0x101c, 0x101d, 0x1020, 0x101f, 0x1023, 0x1999, 0):
    check(op, [], f'misc {op:#x}')
check(0x101e, [u32(0), u32(0), u32(64)], 'otp read')
check(0x1021, [w('Z:\\NVRAM')], 'restore')
check(0x1025, [w('Z:\\NVRAM\\NVD_IMEI\\LD0B_001')], 'detail')
check(0x1025, [w('Z:\\nope')], 'detail-missing')


def cmpt_read(m, flags=0, size=0, off=0, wh=0, ln=0, h=0):
    ws = [0] * 16
    ws[0], ws[3], ws[4], ws[5], ws[6], ws[8], ws[10] = m, flags, size, off, wh, ln, h
    return struct.pack('<16I', *[x & 0xffffffff for x in ws])


def cmpt_write(m, flags=0, off=0, wh=0, ln=0, h=0):
    ws = [0] * 16
    ws[0], ws[3], ws[4], ws[5], ws[7], ws[8] = m, flags, off, wh, ln, h
    return struct.pack('<16I', *[x & 0xffffffff for x in ws])


P = w('Z:\\NVRAM\\NVD_IMEI\\LD0B_001')
for op in (0x1022, 0x1026):
    for m in (0, 1, 0x1b, 0x1f, 0x0f, 0x1d, 0x08, 0x10, 0x15):
        for off, wh in ((0, 0), (10, 0), (-3, 2), (0, 5)):
            r = check(op, [P, cmpt_read(m, FS_READ_ONLY, 7, off, wh, 50)], f'cmpt {op:#x} {m:#x} {off} {wh}')
            if op == 0x1026 and len(r) > 4:
                hv = struct.unpack('<i', r[4])[0]
                if hv > 0:
                    check(op, [P, cmpt_read(0x18, 0, 0, 0, 0, 20, hv)], 'cmpt-ex-cont')
    check(op, [w('Z:\\nope'), cmpt_read(0x1f, 0, 0, 0, 0, 5)], 'cmpt-missing')
    check(op, [P, cmpt_read(0x18, 0, 0, 0, 0, 5, 99)], 'cmpt-badh')
    check(op, [P, cmpt_read(0x18, 0, 0, 0, 0, 5, -1)], 'cmpt-negh')
    check(op, [P], 'cmpt-noparam')
W = w('Z:\\NVRAM\\CW')
for m, h, off, wh in ((0x31, -1, 0, 0), (0x35, -1, 3, 0), (0x30, -1, 0, 0), (0x25, -1, 0, 7), (0x21, -1, 0, 0),
                      (0x20, 1, 0, 0), (0x30, 99, 0, 0)):
    check(0x1024, [W, cmpt_write(m, FS_CREATE, off, wh, 6, h), b'PAYLOADXX'], f'cw {m:#x} {h}')
check(0x1024, [W, cmpt_write(0x31, FS_CREATE, 0, 0, 6, -1)], 'cw-nopayload')
check(0x1006, [], 'closeall2')

# ---------------------------------------------------------- random
rnd = random.Random(seed)
names = ['Z:\\R', 'Z:\\R\\a', 'Z:\\R\\b', 'Z:\\R\\S', 'Z:\\R\\S\\c', 'Z:\\R\\*', 'Z:\\R\\S\\*', 'Z:\\R\\?',
         'X:\\p', 'Y:\\q', 'U:\\u', 'U:\\u\\v', 'Z:\\NVRAM', 'Q:\\x', 'B:\\x', 'Z:\\R\\..', 'Z:\\']
handles = [0, 1, 2, 3, 4, 5, 127, 128, 129, 0xffffffff]
ops = list(range(0x1001, 0x1027))


def rarg():
    k = rnd.random()
    if k < 0.4:
        return w(rnd.choice(names))
    if k < 0.7:
        return u32(rnd.choice(handles + [0x100, 0x10000, 0x20000, 0x1c, 0x10, 3, 0x10001, 2, 1, 4]))
    if k < 0.85:
        return bytes(rnd.randrange(256) for _ in range(rnd.randrange(0, 70)))
    return bytes([rnd.randrange(32)])


for i in range(n_random):
    op = rnd.choice(ops)
    args = [rarg() for _ in range(rnd.randrange(0, 5))]
    if op in (0x1022, 0x1024, 0x1026) and len(args) >= 2 and rnd.random() < 0.7:
        args[0] = w(rnd.choice(names[:5]))
        args[1] = (cmpt_read if op != 0x1024 else cmpt_write)(
            rnd.randrange(0x40), rnd.choice([0, 0x100, 0x10000, 0x20000]), rnd.randrange(20),
            rnd.randrange(20), rnd.randrange(4), rnd.randrange(40))
    check(op, args, f'rnd#{i}')

rs.stdin.close()
rs.wait()


def tree(root):
    out = {}
    for d, dirs, files in os.walk(root):
        for f in files + dirs:
            p = os.path.join(d, f)
            st = os.lstat(p)
            out[os.path.relpath(p, root)] = (oct(st.st_mode), None if f in dirs else open(p, 'rb').read())
    return out


tp, tr = tree(proot), tree(rroot)
if tp != tr:
    fails += 1
    print('TREE MISMATCH', sorted(set(tp.items()) ^ set(tr.items()))[:10])
print(f'done, {fails} mismatches')
sys.exit(1 if fails else 0)
