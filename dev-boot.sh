#!/bin/bash
# Wait for U-Boot fastboot, boot out/boot-linux.img and log the serial console.
#   usage: dev-boot.sh [seconds-to-log] [logfile]
set -e
cd "$(dirname "$0")/.."
SECS=${1:-60}
LOG=${2:-logs/boot-$(date +%Y%m%d-%H%M%S).txt}
# U-Boot boots the system on its own: stop its countdown on the UART
# (the FT232R needs a reset after the phone has reset)
if ! fastboot devices 2>/dev/null | grep -q .; then
	sleep 2
	scripts/uart-reset.sh
	scripts/ub-stop-autoboot.py -t 60 || true
fi
for _ in $(seq 120); do fastboot devices 2>/dev/null | grep -q . && break; sleep 1; done
fastboot devices | grep -q . || { echo "no fastboot device"; exit 1; }
scripts/uart-reset.sh
(timeout "$SECS" cat /dev/ttyUSB0 | tr -d '\r' > "$LOG") &
sleep 0.3
fastboot boot "${BOOTIMG:-out/boot-linux.img}"
wait
echo "log: $LOG"
