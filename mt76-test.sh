#!/bin/bash
# Reboot into the current out/boot-linux.img, push the mainline connsys and
# mt7921 SoC WiFi modules + the NVRAM and load them, logging the UART.
#   usage: [SOC_ARGS=...] [WAIT=s] mt76-test.sh [--no-reboot]
set -e
cd "$(dirname "$0")/.."
SSH="ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 root@172.16.42.1"
B=src/build-linux
MODS="lib/crypto/libarc4.ko
crypto/cmac.ko
net/wireless/cfg80211.ko
net/mac80211/mac80211.ko
drivers/net/wireless/mediatek/mt76/mt76.ko
drivers/net/wireless/mediatek/mt76/mt76-connac-lib.ko
drivers/net/wireless/mediatek/mt76/mt792x-lib.ko
drivers/net/wireless/mediatek/mt76/mt7921/mt7921-common.ko
drivers/soc/mediatek/mtk-connsys.ko
drivers/net/wireless/mediatek/mt76/mt7921/mt7921-soc.ko"
LOG=logs/mt76-$(date +%H%M%S)

if [ "$1" != --no-reboot ]; then
	scripts/mk-linux-bootimg.sh >/dev/null
	$SSH 'reboot -f' 2>/dev/null || printf '\nreboot -f\n' > /dev/ttyACM0 || true
	scripts/dev-boot.sh 25 "$LOG-boot.txt" | tail -1
fi
for _ in $(seq 30); do $SSH true 2>/dev/null && break; sleep 1; done

$SSH 'mkdir -p /lib/modules /lib/firmware/mediatek'
[ -n "$CONNLOG" ] && $SSH 'cat > /usr/bin/connlog; chmod +x /usr/bin/connlog; connlog init' < "$CONNLOG"
$SSH 'cat > /lib/firmware/mediatek/mt6878-wifi-nvram.bin' < firmware/vendor/WIFI
for m in $MODS; do
	llvm-strip --strip-debug -o /tmp/claude-mt76.ko "$B/$m"
	$SSH "cat > /lib/modules/$(basename "$m")" < /tmp/claude-mt76.ko
done
rm -f /tmp/claude-mt76.ko

(timeout 60 cat /dev/ttyUSB0 | tr -d '\r' > "$LOG-uart.txt") &
sleep 0.3
NAMES=$(for m in $MODS; do basename "$m"; done | tr '\n' ' ')
$SSH "echo 8 > /proc/sys/kernel/printk; dmesg -c >/dev/null; for m in $NAMES; do a=; [ \$m = mt7921-soc.ko ] && a='$SOC_ARGS'; insmod /lib/modules/\$m \$a || echo \"insmod \$m failed\"; done; sleep ${WAIT:-8}; dmesg; ip link" > "$LOG-dmesg.txt" 2>&1 || true
[ -n "$CONNLOG" ] && $SSH 'connlog dump' > "$LOG-fwlog.txt" 2>&1
echo "logs: $LOG-*"
