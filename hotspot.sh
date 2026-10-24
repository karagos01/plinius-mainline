#!/bin/sh
# WiFi hotspot on ap0 next to wlan0: hostapd with WPA2-PSK, networkd hands
# out addresses and masquerades to the uplinks, the mobile data or wlan0
# (/etc/systemd/network/30-ap0.network). The firmware runs both interfaces
# on one channel only, so with wlan0 connected the hotspot takes its channel.
#   env (/etc/default/plinius-hotspot): SSID, PSK, CHANNEL
set -e
: "${SSID:=plinius}" "${CHANNEL:=6}"
if [ ${#PSK} -lt 8 ] || [ ${#PSK} -gt 63 ]; then
	echo "PSK in /etc/default/plinius-hotspot: 8 to 63 characters" >&2
	exit 1
fi

# Give a configured WiFi connection the chance to come up first
if systemctl -q is-active wpa_supplicant@wlan0; then
	for _ in $(seq 30); do
		iw dev wlan0 link | grep -q '^Connected' && break
		sleep 1
	done
fi
ch=$(iw dev wlan0 info | sed -n 's/^[[:space:]]*channel \([0-9]*\).*/\1/p')
[ -n "$ch" ] && CHANNEL=$ch
[ "$CHANNEL" -gt 14 ] && mode=a || mode=g

# Own address: the one of wlan0, locally administered and the last byte
# changed (the P2P device already has the locally administered one)
set -- $(tr ':' ' ' < /sys/class/net/wlan0/address)
addr=$(printf '%02x:%s:%s:%s:%s:%02x' $((0x$1 | 2)) $2 $3 $4 $5 $(((0x$6 + 1) & 0xff)))

conf=/run/plinius-hotspot.conf
umask 077
cat > $conf <<EOF
interface=ap0
driver=nl80211
ssid=$SSID
hw_mode=$mode
channel=$CHANNEL
ieee80211n=1
wmm_enabled=1
wpa=2
wpa_key_mgmt=WPA-PSK
rsn_pairwise=CCMP
wpa_passphrase=$PSK
EOF

iw dev ap0 del 2>/dev/null || true
iw dev wlan0 interface add ap0 type __ap addr $addr
exec hostapd $conf
