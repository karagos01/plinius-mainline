#!/bin/bash
# Build a Debian (trixie, arm64) rootfs for Plinius in out/debian-rootfs:
# debootstrap through qemu-user, the kernel modules of src/build-linux, the
# WiFi NVRAM, USB gadget networking (172.16.42.1, DHCP for the host),
# root ssh with the host key, root autologin on the serial consoles and
# the modem services.
#   usage: sudo mk-debian-rootfs.sh [ssh-pubkey]
#   env: WIFI_SSID / WIFI_PSK to preconfigure wlan0
set -e
cd "$(dirname "$0")/.."
R=out/debian-rootfs
PUBKEY=${1:-/home/${SUDO_USER:-$USER}/.ssh/id_ed25519.pub}
B=src/build-linux

if [ ! -x $R/sbin/init ]; then
	debootstrap --arch=arm64 --variant=minbase \
		--include=systemd-sysv,udev,dbus,kmod,iproute2,iputils-ping,openssh-server,sudo,ca-certificates,wpasupplicant,hostapd,iw,wireless-regdb,nano,less,procps,psmisc,e2fsprogs,dosfstools,util-linux,bash-completion,locales,tzdata,curl,wget,netbase,systemd-timesyncd,systemd-resolved,apt-utils,htop,usbutils,i2c-tools,file \
		trixie $R http://deb.debian.org/debian
fi

echo plinius > $R/etc/hostname
cat > $R/etc/hosts <<'EOF'
127.0.0.1	localhost
127.0.1.1	plinius
::1		localhost ip6-localhost ip6-loopback
EOF
echo 'LABEL=plinius	/	ext4	defaults,noatime	0	1' > $R/etc/fstab
cat > $R/etc/apt/sources.list <<'EOF'
deb http://deb.debian.org/debian trixie main contrib non-free non-free-firmware
deb http://deb.debian.org/debian trixie-updates main contrib non-free non-free-firmware
deb http://deb.debian.org/debian-security trixie-security main contrib non-free non-free-firmware
EOF
ln -sf /usr/share/zoneinfo/Europe/Prague $R/etc/localtime
echo Europe/Prague > $R/etc/timezone

# Kernel modules and the NVRAM of WiFi and the modem from scripts/build.sh
# nvram (the WiFi firmware itself comes from LK)
rm -rf $R/lib/modules/*
make -s -C src/MT6878-Mainline_linux O=$PWD/$B ARCH=arm64 LLVM=1 \
	INSTALL_MOD_PATH=$PWD/$R INSTALL_MOD_STRIP=1 modules_install
NV=out/modem-nvram
WIFI_NVRAM=$NV/nvdata/APCFG/APRDEB/WIFI
[ -f $WIFI_NVRAM ] || WIFI_NVRAM=firmware/vendor/WIFI
install -D -m 644 $WIFI_NVRAM $R/lib/firmware/mediatek/mt6878-wifi-nvram.bin
# Modem NVRAM (calibration, IMEI): the modem writes it, never overwrite it
if [ -d $NV ] && [ ! -d $R/var/lib/modem ]; then
	install -d -m 700 $R/var/lib/modem
	cp -a $NV/. $R/var/lib/modem/
fi

# Modem userspace and mobile data (the PIN goes to /etc/default/plinius-modem)
install -d $R/usr/local/lib/plinius-modem
install -m 755 scripts/ccci-server.py scripts/ccci_fs.py scripts/mobile-data.py \
	scripts/sms.py $R/usr/local/lib/plinius-modem/
ln -sf /usr/local/lib/plinius-modem/sms.py $R/usr/local/bin/sms
install -m 644 scripts/systemd/plinius-*.service $R/etc/systemd/system/
[ -f $R/etc/default/plinius-modem ] ||
	install -m 600 scripts/systemd/plinius-modem.default $R/etc/default/plinius-modem
# WiFi hotspot, off until the passphrase is set in /etc/default/plinius-hotspot
install -m 755 scripts/hotspot.sh $R/usr/local/sbin/plinius-hotspot
[ -f $R/etc/default/plinius-hotspot ] ||
	install -m 600 scripts/systemd/plinius-hotspot.default $R/etc/default/plinius-hotspot

# ssh: root with the host key only
if [ -f "$PUBKEY" ]; then
	install -d -m 700 $R/root/.ssh
	install -m 600 "$PUBKEY" $R/root/.ssh/authorized_keys
fi

# Hardware watchdog: let systemd arm /dev/watchdog (mtk-wdt) so a wedged system
# resets itself. systemd pings at half of RuntimeWatchdogSec, well inside the
# 31s HW timeout; RebootWatchdogSec bounds a hung shutdown.
install -d $R/etc/systemd/system.conf.d
cat > $R/etc/systemd/system.conf.d/watchdog.conf <<'EOF'
[Manager]
RuntimeWatchdogSec=20s
RebootWatchdogSec=2min
EOF

# Root autologin on the UART and USB ACM consoles
for tty in serial-getty@ttyS0 serial-getty@ttyGS0; do
	install -d $R/etc/systemd/system/$tty.service.d
	cat > $R/etc/systemd/system/$tty.service.d/autologin.conf <<'EOF'
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin root --keep-baud 921600,115200 %I $TERM
EOF
done
ln -sf /lib/systemd/system/serial-getty@.service \
	$R/etc/systemd/system/getty.target.wants/serial-getty@ttyGS0.service

# Networking: the initramfs sets up the gadget, networkd its address and
# a DHCP server handing the host 172.16.42.2; wlan0 by DHCP
install -d $R/etc/systemd/network
cat > $R/etc/systemd/network/10-usb0.network <<'EOF'
[Match]
Name=usb0

[Network]
Address=172.16.42.1/24
DHCPServer=yes
ConfigureWithoutCarrier=yes

[DHCPServer]
PoolOffset=2
PoolSize=1
EmitDNS=no
EmitRouter=no
EOF
# Prefer IPv4: networks handing out a prefix without a route are common
echo 'precedence ::ffff:0:0/96  100' >> $R/etc/gai.conf
cat > $R/etc/systemd/network/20-wlan0.network <<'EOF'
[Match]
Name=wlan0

[Network]
DHCP=yes
EOF
# Hotspot clients: addresses by DHCP, NAT to the mobile data and wlan0
cat > $R/etc/systemd/network/30-ap0.network <<'EOF'
[Match]
Name=ap0

[Network]
Address=192.168.77.1/24
DHCPServer=yes
IPMasquerade=ipv4
ConfigureWithoutCarrier=yes

[DHCPServer]
PoolOffset=10
PoolSize=100
DNS=1.1.1.1 8.8.8.8
EOF
if [ -n "$WIFI_SSID" ]; then
	install -d $R/etc/wpa_supplicant
	cat > $R/etc/wpa_supplicant/wpa_supplicant-wlan0.conf <<EOF
ctrl_interface=DIR=/run/wpa_supplicant GROUP=netdev
update_config=1
country=CZ

network={
	ssid="$WIFI_SSID"
	psk="$WIFI_PSK"
}
EOF
	chmod 600 $R/etc/wpa_supplicant/wpa_supplicant-wlan0.conf
	ln -sf /lib/systemd/system/wpa_supplicant@.service \
		$R/etc/systemd/system/multi-user.target.wants/wpa_supplicant@wlan0.service
fi

sed -i 's/^# *\(en_US.UTF-8\|cs_CZ.UTF-8\)/\1/' $R/etc/locale.gen
echo LANG=cs_CZ.UTF-8 > $R/etc/default/locale
chroot $R /bin/sh -e <<'EOF'
export LC_ALL=C
locale-gen >/dev/null
# Added after the first debootstrap of older rootfs trees
dpkg -s hostapd >/dev/null 2>&1 || apt-get install -y -q --no-install-recommends hostapd
systemctl enable systemd-networkd systemd-resolved ssh systemd-timesyncd \
	plinius-modem plinius-mobile-data
# Do not hold the boot until WiFi is up
systemctl mask systemd-networkd-wait-online
passwd -l root
# User "volla": ssh with the root key, sudo without a password
id volla >/dev/null 2>&1 || useradd -m -s /bin/bash \
	-G sudo,netdev,dialout,video,audio,plugdev,input volla
passwd -l volla
if [ -f /root/.ssh/authorized_keys ]; then
	install -d -m 700 -o volla -g volla /home/volla/.ssh
	install -m 600 -o volla -g volla /root/.ssh/authorized_keys /home/volla/.ssh/
fi
echo 'volla ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/volla
chmod 440 /etc/sudoers.d/volla
depmod -a $(ls /lib/modules)
EOF
du -sh $R
