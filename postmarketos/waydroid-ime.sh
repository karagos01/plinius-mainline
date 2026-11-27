#!/bin/sh -e
# Builds the "pmOS keyboard" Android input method used by waydroid-plinius
# (see waydroid-plinius/ime-bridge) into work/waydroid-ime/PliniusIme.apk.
# Downloads the Android build-tools and platform once into work/android-sdk.
# The signing key is generated on the first run and kept in work/waydroid-ime;
# keep it, Android refuses an update signed with a different key.

HERE=$(cd "$(dirname "$0")" && pwd)
SRC=$HERE/waydroid-plinius/ime
SDK=$HERE/../../work/android-sdk
OUT=$HERE/../../work/waydroid-ime
BT=build-tools_r34-linux.zip
PLATFORM=platform-33-ext5_r01.zip

mkdir -p "$SDK" "$OUT"
cd "$SDK"
for z in $BT $PLATFORM; do
	[ -f $z ] || curl -sSfL -o $z https://dl.google.com/android/repository/$z
done
[ -x android-14/d8 ] || unzip -qo $BT
[ -f android-33-ext5/android.jar ] || unzip -qo $PLATFORM
TOOLS=$SDK/android-14
JAR=$SDK/android-33-ext5/android.jar

cd "$OUT"
rm -rf build && mkdir -p build/classes build/dex build/res
"$TOOLS"/aapt2 compile -o build/res --dir "$SRC"/res
"$TOOLS"/aapt2 link -o build/unsigned.apk -I "$JAR" \
	--manifest "$SRC"/AndroidManifest.xml \
	--min-sdk-version 30 --target-sdk-version 33 build/res/*.flat
javac -nowarn --release 11 -classpath "$JAR" -d build/classes \
	$(find "$SRC"/src -name '*.java')
"$TOOLS"/d8 --release --min-api 30 --lib "$JAR" --output build/dex \
	$(find build/classes -name '*.class')
(cd build/dex && zip -qX ../unsigned.apk classes.dex)
"$TOOLS"/zipalign -f 4 build/unsigned.apk build/aligned.apk

[ -f release.jks ] || keytool -genkeypair -keystore release.jks \
	-storepass waydroid-plinius -keypass waydroid-plinius -alias ime \
	-keyalg RSA -keysize 3072 -validity 36500 \
	-dname "CN=waydroid-plinius pmOS keyboard" >/dev/null 2>&1
"$TOOLS"/apksigner sign --ks release.jks --ks-pass pass:waydroid-plinius \
	--ks-key-alias ime --out PliniusIme.apk build/aligned.apk
rm -f PliniusIme.apk.idsig
echo "$OUT/PliniusIme.apk"
