#!/bin/bash
# Build Mesa for the Waydroid vendor image, so that Android renders on the
# Mali-G615 (panthor). The Waydroid vendor image ships a Mesa that predates
# Mali-G615 support; this builds a newer one with the Android NDK and packs
# the 64-bit and 32-bit libraries as an overlay of /vendor/lib64 and
# /vendor/lib.
#
#   usage: waydroid-mesa.sh [workdir]   (default: work/waydroid-mesa)
#   output: <workdir>/waydroid-mesa-overlay.tar.gz, to be extracted into
#           /var/lib/waydroid/overlay on the phone
#
# Host needs: git, curl, unzip, python3-venv, ninja, bison, flex, glslang-tools,
# llvm-18-dev, libclang-18-dev, libclang-cpp18-dev, libclc-18-dev,
# libllvmspirvlib-18-dev, llvm-spirv-18 (for the host mesa_clc compiler).
set -e
cd "$(dirname "$0")/../.."

MESA_TAG=mesa-26.3.1
NDK=android-ndk-r27d
API=33				# Android 13, the LineageOS 20 Waydroid image
W=$(realpath -m "${1:-work/waydroid-mesa}")
mkdir -p "$W"
cd "$W"

[ -d $NDK ] || { curl -sSfL -o ndk.zip \
	https://dl.google.com/android/repository/$NDK-linux.zip &&
	unzip -q ndk.zip && rm ndk.zip; }
[ -d mesa ] || git clone -q --depth 1 -b $MESA_TAG \
	https://gitlab.freedesktop.org/mesa/mesa.git
[ -x venv/bin/meson ] || { python3 -m venv venv &&
	venv/bin/pip -q install 'meson>=1.5,<1.9' mako pyyaml packaging; }
export PATH=$W/host/bin:$W/venv/bin:$PATH

# The panfrost shaders are precompiled at build time by host tools.
if [ ! -x host/bin/panfrost_compile ]; then
	meson setup build-host mesa -Dprefix="$W/host" -Dbuildtype=release \
		-Dgallium-drivers=panfrost -Dvulkan-drivers= -Dplatforms= \
		-Dglx=disabled -Degl=disabled -Dgbm=disabled -Dgles1=disabled \
		-Dgles2=disabled -Dopengl=false -Dllvm=enabled \
		-Dmesa-clc=enabled -Dinstall-mesa-clc=true \
		-Dprecomp-compiler=enabled -Dinstall-precomp-compiler=true \
		-Dvalgrind=disabled -Dlibunwind=disabled
	ninja -C build-host install
fi

# Library names and the GBM backend path match the vendor image's own
# Mesa (libEGL_mesa.so, libgbm_mesa.so, dri_gbm.so in /vendor/lib64 and
# /vendor/lib), and libdrm is linked in statically: the vendor libdrm.so
# stays untouched.
TC=$W/$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin
rm -rf overlay

# build <meson cpu_family> <cpu> <NDK target> <vendor lib dir> <build dir>
build() {
	cat > android-$1.ini <<EOF2
[constants]
tc = '$TC'

[binaries]
ar = tc / 'llvm-ar'
c = [tc / '$3$API-clang']
cpp = [tc / '$3$API-clang++', '-fno-exceptions', '-fno-unwind-tables', '-fno-asynchronous-unwind-tables', '--start-no-unused-arguments', '-static-libstdc++', '--end-no-unused-arguments']
c_ld = 'lld'
cpp_ld = 'lld'
strip = tc / 'llvm-strip'
pkg-config = '/bin/false'

[host_machine]
system = 'android'
cpu_family = '$1'
cpu = '$2'
endian = 'little'
EOF2

	[ -f $5/build.ninja ] ||
		meson setup $5 mesa --cross-file android-$1.ini \
			-Dbuildtype=release -Db_ndebug=true \
			-Dplatforms=android -Dplatform-sdk-version=$API \
			-Dandroid-stub=true -Dandroid-libbacktrace=disabled \
			-Dgallium-drivers=panfrost -Dvulkan-drivers=panfrost \
			-Degl=enabled -Dgbm=enabled -Dgles1=enabled -Dgles2=enabled \
			-Dglx=disabled -Degl-lib-suffix=_mesa -Dgles-lib-suffix=_mesa \
			-Dgbm-backends-path=/vendor/$4 -Dllvm=disabled \
			-Dmesa-clc=system -Dprecomp-compiler=system \
			-Dallow-fallback-for=libdrm -Dlibdrm:default_library=static \
			-Dxmlconfig=disabled -Dzstd=disabled -Dexpat=disabled \
			-Dvalgrind=disabled -Dlibunwind=disabled
	ninja -C $5

	local B=$5/src O=overlay/vendor/$4
	mkdir -p $O/egl $O/hw
	cp $B/gallium/targets/dri/libgallium_dri.so $B/gbm/backends/dri/dri_gbm.so \
		$B/gbm/libgbm_mesa.so $O/
	cp $B/egl/libEGL_mesa.so $B/mesa/glapi/es1api/libGLESv1_CM_mesa.so \
		$B/mesa/glapi/es2api/libGLESv2_mesa.so $O/egl/
	cp $B/panfrost/vulkan/libvulkan_panfrost.so $O/hw/vulkan.panfrost.so
	find $O -name '*.so' -exec $TC/llvm-strip --strip-unneeded {} +
}

# 64-bit Android, and 32-bit for the apps that only ship armeabi-v7a code
build aarch64 armv8 aarch64-linux-android lib64 build-android
build arm armv7 armv7a-linux-androideabi lib build-android-arm

# A reproducible archive, so that a rebuild keeps the package checksum
tar -C overlay --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
	-cf - vendor | gzip -n > waydroid-mesa-overlay.tar.gz
ls -la "$W/waydroid-mesa-overlay.tar.gz"
