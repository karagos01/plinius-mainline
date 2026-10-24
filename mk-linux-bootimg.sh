#!/bin/bash
# Pack the mainline kernel as a v2 boot.img for "fastboot boot" from U-Boot.
#   env: CMDLINE_EXTRA (e.g. root=LABEL=plinius), OUT (default out/boot-linux.img)
set -e
cd "$(dirname "$0")/.."
B=src/build-linux/arch/arm64/boot
python3 tools/mkbootimg/mkbootimg.py --header_version 2 \
	--kernel $B/Image.gz \
	--ramdisk out/initramfs.cpio.gz \
	--dtb $B/dts/mediatek/mt6878-volla-plinius.dtb \
	--base 0x40000000 --kernel_offset 0x00080000 \
	--ramdisk_offset 0x06000000 --dtb_offset 0x05f00000 \
	--cmdline "earlycon=uart8250,mmio32,0x11001000 console=tty0 console=ttyS0,921600 clk_ignore_unused${CMDLINE_EXTRA:+ $CMDLINE_EXTRA}" \
	--output "${OUT:-out/boot-linux.img}"
ls -la "${OUT:-out/boot-linux.img}"
