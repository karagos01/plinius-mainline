#!/bin/bash
# Build the Plinius mainline OS from the sources in src/ into out/:
#   kernel    Image.gz, DTB and modules (src/build-linux, clang/LLVM)
#   uboot     U-Boot packed as boot_a image (out/boot-uboot.img)
#   initramfs Alpine based initramfs (out/initramfs.cpio.gz)
#   bootimg   out/boot-debian.img (root=LABEL=plinius) and the RAM-only
#             rescue image out/boot-linux.img
#   nvram     modem/WiFi NVRAM tree from the stock backup (out/modem-nvram)
#   rootfs    Debian rootfs (out/debian-rootfs, needs sudo)
#   usage: scripts/build.sh [step...]   (default: all steps in this order)
#   env: WIFI_SSID / WIFI_PSK for the rootfs, JOBS (default nproc)
set -e
cd "$(dirname "$0")/.."
JOBS=${JOBS:-$(nproc)}
LINUX=src/MT6878-Mainline_linux
UBOOT=src/MT6878-Mainline_u-boot
ALPINE=3.22
ALPINE_MIRROR=https://dl-cdn.alpinelinux.org/alpine/v$ALPINE
# Extra initramfs packages: ssh, DHCP for the host, WiFi tools
ALPINE_PKGS="dropbear dnsmasq dnsmasq-common iw libnl3 wpa_supplicant dbus-libs
	pcsc-lite-libs skalibs-libs utmps-libs"
INITRAMFS_INPUTS=${INITRAMFS_INPUTS:-out/initramfs-inputs}

step_kernel() {
	local B=$PWD/src/build-linux
	# LLVM=1 always: a GCC run rewrites .config without CFI/LTO
	local MK="make -s -C $LINUX O=$B ARCH=arm64 LLVM=1"
	if [ ! -f $B/.config ]; then
		install -D -m 644 scripts/configs/linux.config $B/.config
		$MK olddefconfig </dev/null
	fi
	$MK -j"$JOBS" Image.gz dtbs modules
	echo "kernel: $(cat $B/include/config/kernel.release)"
}

step_uboot() {
	local B=$PWD/src/build-uboot
	local MK="make -s -C $UBOOT O=$B CROSS_COMPILE=aarch64-linux-gnu-"
	[ -f $B/.config ] || $MK mt6878_plinius_defconfig
	$MK -j"$JOBS"
	scripts/mk-uboot-bootimg.sh
}

# Pick the newest version of a package from the APKINDEX
apk_version() {
	awk -v p="$1" 'BEGIN { RS = "" }
		{ n = v = ""
		  split($0, l, "\n")
		  for (i in l) {
			if (l[i] ~ /^P:/) n = substr(l[i], 3)
			if (l[i] ~ /^V:/) v = substr(l[i], 3)
		  }
		  if (n == p) { print v; exit } }'
}

fetch_initramfs_inputs() {
	local I=$INITRAMFS_INPUTS ver idx p v
	ls $I/alpine-minirootfs-*-aarch64.tar.gz >/dev/null 2>&1 && return
	mkdir -p $I/apk
	ver=$(curl -fsSL $ALPINE_MIRROR/releases/aarch64/latest-releases.yaml |
	      awk '$1 == "version:" { print $2; exit }')
	curl -fL -o $I/alpine-minirootfs-$ver-aarch64.tar.gz \
		$ALPINE_MIRROR/releases/aarch64/alpine-minirootfs-$ver-aarch64.tar.gz
	idx=$(curl -fsSL $ALPINE_MIRROR/main/aarch64/APKINDEX.tar.gz | tar -xzOf - APKINDEX)
	for p in $ALPINE_PKGS; do
		v=$(apk_version $p <<<"$idx")
		[ -n "$v" ] || { echo "initramfs: no package $p" >&2; exit 1; }
		curl -fsSL -o $I/apk/$p-$v.apk $ALPINE_MIRROR/main/aarch64/$p-$v.apk
	done
}

step_initramfs() {
	fetch_initramfs_inputs
	scripts/mk-initramfs.sh $(ls $INITRAMFS_INPUTS/alpine-minirootfs-*-aarch64.tar.gz | tail -1) \
		$INITRAMFS_INPUTS/apk
}

step_bootimg() {
	OUT=out/boot-linux.img scripts/mk-linux-bootimg.sh
	CMDLINE_EXTRA="root=LABEL=plinius" OUT=out/boot-debian.img scripts/mk-linux-bootimg.sh
}

step_nvram() {
	if [ -d out/modem-nvram ]; then
		echo "nvram: out/modem-nvram exists, kept"
	elif [ -f backup/partitions/nvram.img ]; then
		scripts/mk-modem-nvram.py backup/partitions out/modem-nvram
	else
		echo "nvram: no backup/partitions/nvram.img, the modem and WiFi need it" >&2
	fi
}

step_rootfs() {
	sudo --preserve-env=WIFI_SSID,WIFI_PSK scripts/mk-debian-rootfs.sh
}

mkdir -p out
for s in ${@:-kernel uboot initramfs bootimg nvram rootfs}; do
	declare -F step_$s >/dev/null || { echo "unknown step: $s" >&2; exit 1; }
	echo "=== $s"
	step_$s
done
