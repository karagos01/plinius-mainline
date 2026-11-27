#!/bin/sh
# Builds hwcomposer.waydroid.so (the shim) into the given directory with
# clang and lld, without the Android NDK: it links against stub bionic
# libraries that only provide the names of what it uses.
set -e
src=$(dirname "$0")
out=${1:-.}
cc="${CLANG:-clang} --target=aarch64-linux-android30 -fuse-ld=lld -nostdlib -shared"
mkdir -p "$out"/stubs
stub() {
	printf '%s\n' "$2" >"$out"/stubs/$1.c
	$cc -o "$out"/stubs/lib$1.so -Wl,-soname,lib$1.so "$out"/stubs/$1.c
}
stub dl 'void dlopen(void){} void dlsym(void){} void dlerror(void){} void dladdr(void){}'
stub c 'void mprotect(void){} void x_memcmp(void) __asm__("memcmp"); void x_memcmp(void){} void x_memcpy(void) __asm__("memcpy"); void x_memcpy(void){}'
stub log 'void __android_log_print(void){}'
$cc -O2 -Wall -Wextra -Werror -fPIC -Wl,-soname,hwcomposer.waydroid.so \
	-Wl,-z,relro,-z,now -L"$out"/stubs -ldl -lc -llog \
	-o "$out"/hwcomposer.waydroid.so "$src"/hwc-shim.c
