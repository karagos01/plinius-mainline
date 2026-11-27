#!/bin/sh
# Copy the Volla Phone Plinius postmarketOS packages into a pmaports checkout.
#
#   postmarketos/install-pmaports.sh [PMAPORTS]
#
# PMAPORTS defaults to pmbootstrap's checkout (pmbootstrap config aports).
# Packages land where pmaports keeps them:
#   device/testing/device-volla-plinius
#   device/testing/linux-postmarketos-mediatek-mt6878  (+ configs/linux.config)
#   device/testing/plinius-modemd                     (modem userspace, ../modemd)
#   device/testing/modemmanager-plugin-mtk-ccci       (+ ../mm-plugin-mtk-ccci)
#   device/testing/waydroid-plinius                   (+ Mesa from waydroid-mesa.sh,
#                                                      keyboard from waydroid-ime.sh)
#   temp/callaudiod                                    (speaker button fix)
#   temp/phosh                                         (screenshot fix)
#   temp/libcamera                                     (CAMSV + GC32E1 + AF)
#   temp/snapshot                                      (landscape viewfinder)
#   temp/mesa                                          (dmabuf export fix)
#   temp/gtk4.0                                        (GstGL EGLDisplay fix)
# Then build and install as usual, e.g.:
#   pmbootstrap checksum linux-postmarketos-mediatek-mt6878
#   pmbootstrap build linux-postmarketos-mediatek-mt6878
#   pmbootstrap install --split
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
PMAPORTS=${1:-$(pmbootstrap config aports)}
[ -d "$PMAPORTS/device" ] || { echo "not a pmaports checkout: $PMAPORTS" >&2; exit 1; }

K=linux-postmarketos-mediatek-mt6878
for p in device/testing/device-volla-plinius device/testing/$K \
	device/testing/plinius-modemd device/testing/modemmanager-plugin-mtk-ccci \
	device/testing/waydroid-plinius \
	temp/callaudiod temp/phosh temp/libcamera temp/snapshot temp/mesa \
	temp/gtk4.0; do
	rm -rf "${PMAPORTS:?}/$p"
	mkdir -p "$PMAPORTS/$p"
	cp -r "$HERE/$(basename "$p")/." "$PMAPORTS/$p/"
	rm -f "$PMAPORTS/$p/README.md"
done
cp "$HERE"/../mm-plugin-mtk-ccci/*.[ch] \
	"$PMAPORTS/device/testing/modemmanager-plugin-mtk-ccci/"
# WiFi calibration NVRAM: device data, kept out of git (see build.sh)
cp "$HERE/../../firmware/vendor/WIFI" \
	"$PMAPORTS/device/testing/device-volla-plinius/mt6878-wifi-nvram.bin"
# Waydroid Mesa: prebuilt by waydroid-mesa.sh, kept out of git
W=$HERE/../../work/waydroid-mesa/waydroid-mesa-overlay.tar.gz
[ -f "$W" ] && cp "$W" "$PMAPORTS/device/testing/waydroid-plinius/" ||
	echo "no $W: run waydroid-mesa.sh to build waydroid-plinius" >&2
# Waydroid keyboard: prebuilt by waydroid-ime.sh, kept out of git
W=$HERE/../../work/waydroid-ime/PliniusIme.apk
[ -f "$W" ] && cp "$W" "$PMAPORTS/device/testing/waydroid-plinius/" ||
	echo "no $W: run waydroid-ime.sh to build waydroid-plinius" >&2
cp "$HERE/../configs/linux.config" \
	"$PMAPORTS/device/testing/$K/config-postmarketos-mediatek-mt6878.aarch64"
echo "installed into $PMAPORTS"
