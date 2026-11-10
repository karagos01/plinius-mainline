#!/bin/sh
# Copy the Volla Phone Plinius postmarketOS packages into a pmaports checkout.
#
#   postmarketos/install-pmaports.sh [PMAPORTS]
#
# PMAPORTS defaults to pmbootstrap's checkout (pmbootstrap config aports).
# Packages land where pmaports keeps them:
#   device/testing/device-volla-plinius
#   device/testing/linux-postmarketos-mediatek-mt6878  (+ configs/linux.config)
#   temp/mesa                                          (until Alpine has >= 26.3)
# Then build and install as usual, e.g.:
#   pmbootstrap checksum linux-postmarketos-mediatek-mt6878
#   pmbootstrap build linux-postmarketos-mediatek-mt6878 mesa
#   pmbootstrap install --split
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
PMAPORTS=${1:-$(pmbootstrap config aports)}
[ -d "$PMAPORTS/device" ] || { echo "not a pmaports checkout: $PMAPORTS" >&2; exit 1; }

K=linux-postmarketos-mediatek-mt6878
for p in device/testing/device-volla-plinius device/testing/$K temp/mesa; do
	rm -rf "${PMAPORTS:?}/$p"
	mkdir -p "$PMAPORTS/$p"
	cp -r "$HERE/$(basename "$p")/." "$PMAPORTS/$p/"
	rm -f "$PMAPORTS/$p/README.md"
done
# WiFi calibration NVRAM: device data, kept out of git (see build.sh)
cp "$HERE/../../firmware/vendor/WIFI" \
	"$PMAPORTS/device/testing/device-volla-plinius/mt6878-wifi-nvram.bin"
cp "$HERE/../configs/linux.config" \
	"$PMAPORTS/device/testing/$K/config-postmarketos-mediatek-mt6878.aarch64"
echo "installed into $PMAPORTS"
