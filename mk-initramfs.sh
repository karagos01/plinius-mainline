#!/bin/bash
# Minimal Alpine based initramfs with a root shell on the serial console.
set -e
cd "$(dirname "$0")/.."
ROOTFS_TGZ=${1:?usage: $0 alpine-minirootfs-aarch64.tar.gz [apk-dir] [ssh-pubkey]}
APK_DIR=${2:-}
PUBKEY=${3:-$HOME/.ssh/id_ed25519.pub}
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
tar -xzf "$ROOTFS_TGZ" -C "$W"

# Extra packages (e.g. dropbear and its libs), unpacked without apk
if [ -n "$APK_DIR" ]; then
	for apk in "$APK_DIR"/*.apk; do
		tar -xzf "$apk" -C "$W" --exclude='.PKGINFO' --exclude='.SIGN.*' \
			--exclude='.pre-*' --exclude='.post-*' --exclude='.trigger' 2>/dev/null
	done
fi
if [ -f "$PUBKEY" ]; then
	install -d -m 700 "$W/root/.ssh"
	install -m 600 "$PUBKEY" "$W/root/.ssh/authorized_keys"
fi
# Small /dev/mem peek/poke helper for register debugging
aarch64-linux-gnu-gcc -static -O2 -o "$W/usr/bin/devmem" tools/devmem.c
aarch64-linux-gnu-gcc -static -O2 -o "$W/usr/bin/memdump" tools/memdump.c
aarch64-linux-gnu-gcc -static -O2 -o "$W/usr/bin/connlog" tools/connlog.c

cat > "$W/init" <<'INIT'
#!/bin/sh
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /dev/pts && mount -t devpts devpts /dev/pts
mount -t debugfs debugfs /sys/kernel/debug 2>/dev/null
echo "=== plinius initramfs: $(uname -r) ==="
cat /proc/cmdline

# USB gadget: ACM serial console + NCM network (172.16.42.1)
setup_gadget() {
	mount -t configfs configfs /sys/kernel/config || return 1
	G=/sys/kernel/config/usb_gadget/g1
	mkdir -p $G/strings/0x409 $G/configs/c.1/strings/0x409 || return 1
	echo 0x18d1 > $G/idVendor
	echo 0xd001 > $G/idProduct
	echo Volla > $G/strings/0x409/manufacturer
	echo "Plinius mainline" > $G/strings/0x409/product
	echo plinius > $G/strings/0x409/serialnumber
	echo "ACM+NCM" > $G/configs/c.1/strings/0x409/configuration
	mkdir $G/functions/acm.usb0 $G/functions/ncm.usb0
	# Fixed MACs keep the host side interface name stable across boots
	# (must be set before linking, f_ncm locks its attributes afterwards)
	echo 02:00:00:6e:78:01 > $G/functions/ncm.usb0/dev_addr
	echo 02:00:00:6e:78:02 > $G/functions/ncm.usb0/host_addr
	ln -s $G/functions/acm.usb0 $G/configs/c.1/
	ln -s $G/functions/ncm.usb0 $G/configs/c.1/
	UDC=$(ls /sys/class/udc | head -n1)
	[ -n "$UDC" ] || { echo "gadget: no UDC"; return 1; }
	echo "$UDC" > $G/UDC && echo "gadget: bound to $UDC"
}
setup_gadget

# root=LABEL=...|UUID=...|/dev/...: hand over to the system on that
# partition, the gadget set up above stays. Without it (or if it fails)
# stay in the initramfs.
for arg in $(cat /proc/cmdline); do
	case $arg in root=*) ROOT=${arg#root=} ;; esac
done
if [ -n "$ROOT" ]; then
	for _ in $(seq 50); do
		DEV=$(findfs "$ROOT" 2>/dev/null) && break
		sleep 0.2
	done
	mkdir -p /newroot
	if [ -n "$DEV" ] && mount -o rw "$DEV" /newroot; then
		if [ -x /newroot/sbin/init ]; then
			echo "root: switching to $DEV"
			mount --move /dev /newroot/dev
			mount --move /sys /newroot/sys
			mount --move /proc /newroot/proc
			exec switch_root /newroot /sbin/init
		fi
		umount /newroot
	fi
	echo "root: $ROOT not usable, staying in the initramfs"
fi

ip link set usb0 up && ip addr add 172.16.42.1/24 dev usb0
# Hand the host 172.16.42.2 so NetworkManager & co. need no setup
if [ -x /usr/sbin/dnsmasq ]; then
	dnsmasq --user=root --interface=usb0 --bind-interfaces --port=0 \
		--dhcp-range=172.16.42.2,172.16.42.2,255.255.255.0,1h \
		--dhcp-option=3 --dhcp-option=6 --dhcp-authoritative \
		--dhcp-leasefile=/tmp/dnsmasq.leases
fi

if [ -x /usr/sbin/dropbear ]; then
	mkdir -p /etc/dropbear
	/usr/sbin/dropbear -R && echo "dropbear: listening on 172.16.42.1:22"
fi

# Shell on the USB serial console, respawned whenever it exits
(
	while true; do
		[ -c /dev/ttyGS0 ] && setsid /bin/sh -c 'exec /bin/sh </dev/ttyGS0 >/dev/ttyGS0 2>&1'
		sleep 1
	done
) &

exec setsid /bin/sh -c 'exec /bin/sh </dev/ttyS0 >/dev/ttyS0 2>&1'
INIT
chmod +x "$W/init"
(cd "$W" && find . | cpio -o -H newc -R 0:0 --quiet | gzip -9) > out/initramfs.cpio.gz
ls -la out/initramfs.cpio.gz
