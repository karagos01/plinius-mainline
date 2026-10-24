#!/usr/bin/env python3
"""Build the modem NVRAM tree for the CCCI file system server.

The modem reads its calibration and IMEI through the CCCI FS channel from
nvdata/md (drive Z:), protect_f/md (X:) and protect_s/md (Y:). The stock
nvdata partition of this phone is empty; Android restores it from the
"bin region" copy in the nvram partition, so do the same here.

  usage: mk-modem-nvram.py BACKUP_PARTITIONS_DIR OUT_DIR
  (protect1/protect2 are ext4 images, read with debugfs)
"""
import os
import struct
import subprocess
import sys

BIN_TABLE = 0x40
BIN_DATA = 0x20000
ENTRY = 0x8c


def extract_bin_region(img, out):
    with open(img, 'rb') as f:
        d = f.read(0x4000000)
    table_size, data_size = struct.unpack_from('<II', d, 0)
    n = 0
    for i in range(table_size // ENTRY):
        off = BIN_TABLE + i * ENTRY
        kind, foff, size = struct.unpack_from('<III', d, off)
        path = d[off + 12:off + ENTRY].split(b'\0')[0].decode(errors='replace')
        if not path.startswith('/mnt/vendor/') or foff + size > data_size:
            continue
        dst = os.path.join(out, path[len('/mnt/vendor/'):])
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        with open(dst, 'wb') as f:
            f.write(d[BIN_DATA + foff:BIN_DATA + foff + size])
        n += 1
    print(f'{img}: {n} files from the bin region')


def extract_ext4_dir(img, src, out):
    os.makedirs(out, exist_ok=True)
    subprocess.run(['debugfs', '-R', f'rdump {src} {out}', img], check=True,
                   stderr=subprocess.DEVNULL)
    print(f'{img}: {src} -> {out}')


def main():
    parts, out = sys.argv[1], sys.argv[2]
    extract_bin_region(os.path.join(parts, 'nvram.img'), out)
    extract_ext4_dir(os.path.join(parts, 'protect1.img'), '/md',
                     os.path.join(out, 'protect_f'))
    extract_ext4_dir(os.path.join(parts, 'protect2.img'), '/md',
                     os.path.join(out, 'protect_s'))
    for d in ('nvdata/md', 'nvdata/md_cmn', 'protect_f/md', 'protect_s/md',
              'nvcfg'):
        os.makedirs(os.path.join(out, d), exist_ok=True)


if __name__ == '__main__':
    main()
