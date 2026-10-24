#!/bin/bash
# Pack U-Boot as the "kernel" of a v4 boot.img so the stock LK chainloads it.
set -e
cd "$(dirname "$0")/.."
B=src/build-uboot
gzip -9 -n -c $B/u-boot.bin > out/u-boot.bin.gz
python3 tools/mkbootimg/mkbootimg.py --header_version 4 --os_version 14.0.0 --os_patch_level 2025-04 \
	--kernel out/u-boot.bin.gz --output out/boot-uboot.img
ls -la out/boot-uboot.img
