#!/bin/bash
# Install the current kernel (src/build-linux) as the default system: the
# modules go to the Debian rootfs on the microSD, the boot image with
# root=LABEL=plinius to vendor_boot_b, which U-Boot boots by default.
# The phone must be running (Debian or the initramfs) and reachable by ssh.
set -e
cd "$(dirname "$0")/.."
SSH="ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 root@172.16.42.1"
B=src/build-linux
M=$(mktemp -d)
trap 'rm -rf "$M"' EXIT

make -s -C src/MT6878-Mainline_linux O=$PWD/$B ARCH=arm64 LLVM=1 \
	INSTALL_MOD_PATH="$M" INSTALL_MOD_STRIP=1 modules_install
KVER=$(ls "$M/lib/modules")

# Debian has the card as /, the initramfs needs to mount it
$SSH "set -e
	if grep -q ' / ext4' /proc/mounts && [ -f /etc/debian_version ]; then R=; else
		mkdir -p /mnt/sd; mountpoint -q /mnt/sd || mount LABEL=plinius /mnt/sd; R=/mnt/sd; fi
	rm -rf \$R/lib/modules/$KVER
	tar -C \$R/lib/modules -xf -
	chroot \${R:-/} depmod -a $KVER
	sync" < <(tar -C "$M/lib/modules" -cf - "$KVER")
echo "modules $KVER installed"

CMDLINE_EXTRA="root=LABEL=plinius" OUT=out/boot-debian.img scripts/mk-linux-bootimg.sh >/dev/null
timeout 5 $SSH 'systemctl reboot 2>/dev/null || reboot -f' >/dev/null 2>&1 || true
sleep 2
scripts/uart-reset.sh
scripts/ub-stop-autoboot.py -t 60
for _ in $(seq 60); do fastboot devices 2>/dev/null | grep -q . && break; sleep 1; done
fastboot flash vendor_boot_b out/boot-debian.img
fastboot reboot
