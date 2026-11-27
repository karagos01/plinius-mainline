# postmarketOS port

postmarketOS packages for the **Volla Phone Plinius** (MediaTek MT6878 /
Dimensity 7300, `volla,plinius`). They are not submitted to pmaports yet;
this directory keeps them in git so the image can be rebuilt.

| Package | pmaports path | Contents |
|---|---|---|
| `device-volla-plinius` | `device/testing/` | `deviceinfo`, WiFi NVRAM, WiFi power save (`mt7921_soc ps_mdtim=3`, NetworkManager `wifi.powersave=3`, gratuitous ARP via a NetworkManager dispatcher), ALSA UCM profile, sensor udev rule, no automatic suspend on battery (gschema override), visible state of the in-call buttons (`/etc/skel` GTK3 stylesheet) |
| `linux-postmarketos-mediatek-mt6878` | `device/testing/` | our kernel, built from `karagos01/linux` at a pinned commit with `configs/linux.config` |
| `plinius-modemd` | `device/testing/` | modem userspace (`../modemd`, Rust) + OpenRC service, ordered before ModemManager |
| `modemmanager-plugin-mtk-ccci` | `device/testing/` | ModemManager plugin for the data path (`../mm-plugin-mtk-ccci`), installed with ModemManager |
| `phosh` | `temp/` | Alpine's Phosh with a fix for screenshots: one taken while the screen is off fails, and Phosh refused every later one until it restarted; to be sent upstream |
| `callaudiod` | `temp/` | Alpine's `callaudiod` with a patch for the in-call speaker button: PulseAudio puts the earpiece and the speaker (same PCM) into separate voice call profiles, so the button has to switch profiles; to be sent upstream |

With these, Phosh runs on our native MediaTek DRM driver and composites on
the GPU; touch, WiFi, Bluetooth, battery, sound, the accelerometer, the
light sensor, the proximity sensor and the modem (ModemManager: SIM PIN,
LTE registration, mobile data over NetworkManager) work. The SAR sensor also exposes
`in_proximity<N>_raw`, so a udev rule keeps iio-sensor-proxy off it. Open
items are in the top-level README roadmap.

## Kernel

The pmaports kernel package of the same name builds the `MT6878-Mainline`
tree, which is the merge-base of our `plinius` branch, so our branch is a
strict superset. Ours differs only in:

- source: `github.com/karagos01/linux` at `_commit` (bump it together with
  `pkgver` when the branch moves);
- config: `configs/linux.config` (copied in by `install-pmaports.sh`);
- `prepare()` turns off module signing and lockdown, which pmOS does not use
  and whose `sign-file` tool fails to build in the musl chroot;
- `pkgver` carries a `_git` suffix so it wins over the official `7.2-r0`.

For a local kernel tree, `pmbootstrap build --src <tree>
linux-postmarketos-mediatek-mt6878` still works.

## Boot

`deviceinfo` mirrors `mk-linux-bootimg.sh`: boot.img header v2, base
`0x40000000`, kernel `+0x80000`, ramdisk `+0x6000000`, dtb `+0x5f00000`,
dtb `mediatek/mt6878-volla-plinius.dtb` passed separately. The command line
has no `root=`; the pmOS initramfs finds the root by its label.

The boot chain is LK → U-Boot (`boot_a`) → boot.img. U-Boot offers
`fastboot boot`, so a pmOS boot.img can be tried without writing anything;
a plain reboot returns to whatever is in `vendor_boot_b`.

## Modem

`plinius-modemd` answers the modem's RPC and NVRAM file system requests and
starts it; its OpenRC service waits until the modem is up (`--ready-file`)
so that ModemManager, started after it, finds the modem on the WWAN AT port
`/dev/wwan0at0` (generic plugin, `ccmni*` as its net ports). The modem NVRAM
holds the IMEI and calibration and is device data, not packaged: copy
`/var/lib/modem` (from `scripts/build.sh nvram`, or from the Debian rootfs)
into the pmOS root, owned by root, mode 0700.

Mobile data needs MediaTek's `+EIF` handshake, which the `mtk-ccci`
ModemManager plugin implements (see `mm-plugin-mtk-ccci/README.md`). It
builds against Alpine's exact ModemManager source and configuration, so
bump `_mmver`/`_mmcommit` together with Alpine's `modemmanager`. pmOS puts
the mobile default route into its own table (1430, used when WiFi is not
connected).

The package builds from a `karagos01/plinius-mainline` tarball at
`_commit`; `pmbootstrap build --src scripts/modemd plinius-modemd` builds
the working tree instead (run `cargo clean` there first).

## Build and run

```sh
postmarketos/install-pmaports.sh          # copy the packages into pmaports
pmbootstrap checksum linux-postmarketos-mediatek-mt6878 device-volla-plinius plinius-modemd
pmbootstrap build linux-postmarketos-mediatek-mt6878 plinius-modemd device-volla-plinius
pmbootstrap build --arch aarch64 callaudiod
pmbootstrap config service_manager openrc  # see the quirks
pmbootstrap build --force --arch aarch64 phosh
pmbootstrap config -r service_manager
pmbootstrap install --split               # UI e.g. phosh
pmbootstrap export
```

The root image goes onto the unused `userdata` partition (`dd` from a
running Linux, label `pmOS_root`); then stop U-Boot's countdown and
`fastboot boot` the exported `boot.img`.

## Waydroid

Waydroid runs with GPU rendering (tested 2026-11-24: LineageOS 20 /
Android 13 images, SurfaceFlinger on "Mali-G615 MC2 (Panfrost)", touch
and network work). The newest Waydroid vendor image (2026-10-22) has
Mesa 26.0.8, which does not know the Mali-G615 (arch 11, supported since
Mesa 26.3): gralloc cannot allocate a single buffer and Android never
finishes booting. `waydroid-mesa.sh` builds Mesa 26.3 with the Android
NDK, 64-bit and 32-bit (EGL/GLES, GBM, panfrost Vulkan), as an overlay of
`/vendor/lib64` and `/vendor/lib`; the vendor image itself stays
untouched.

The `waydroid-plinius` package carries that overlay and sets Waydroid up
so that Android apps feel native in Phosh:

- **Windows and launchers.** `persist.waydroid.multi_windows` opens every
  Android app in its own window; Waydroid itself adds a launcher with the
  app's icon to Phosh for every Android app. Starting one from Phosh
  boots Android first if needed (about 15 s).
- **GPU.** The gralloc and EGL properties select the overlay's Mesa.
  Vulkan (panfrost) is enabled for the apps that use it; Mesa marks the
  Mali-G615 (v11) as not well tested, so it needs
  `debug.mesa.pan.i.want.a.broken.vulkan.driver`, and the Android UI
  itself keeps rendering with GLES.
- **Keyboard.** The Phosh on-screen keyboard types into Android apps,
  with its layouts, diacritics and completion. Android's own keyboard
  cannot be used with separate windows (Waydroid maps it as a window of
  its own that covers the app), and Waydroid has neither text-input nor
  keymap support, so the keys of the on-screen keyboard arrive as
  garbage. Instead the package installs the "pmOS keyboard" Android
  input method (`waydroid-plinius/ime`) and `waydroid-ime-bridge`
  (`waydroid-plinius/ime-bridge`, autostarted in the session). When an
  Android field wants a keyboard, the input method tells the bridge over
  the Waydroid network; the bridge maps an invisible 1x1 layer surface
  with text-input, so Phosh raises its keyboard, and sends the typed
  text back. Switching to a native app gives the keyboard back. The
  bridge listens only on `192.168.240.1` and accepts only the token of
  the input method, which the `waydroid-plinius` service copies from the
  Android data after boot; the service also selects the input method on
  the first boot (afterwards the choice in Android settings is kept).
  `WAYDROID_IME_DEBUG=1` makes the bridge log focus changes.
  Phosh makes room for the keyboard by shrinking the window, which
  Waydroid turns into a resize of the whole Android display; apps that
  don't handle size changes (e.g. F-Droid) are relaunched, so they blink
  when the keyboard opens and closes. Android relaunches them in a new
  window, so the bridge takes the focus back and both sides wait a
  moment before dropping the keyboard. Avoiding the relaunch would need
  Waydroid to report the keyboard as an inset instead.
- **Clipboard.** Waydroid shares it through `py3-pyclip` and
  `wl-clipboard`, pulled in as dependencies.
- **Storage.** The Android shared storage (Download, Pictures, DCIM, ...)
  appears as `~/Android`, also bookmarked in the file chooser. The
  `waydroid-plinius` service gives the user ACL access to it at boot,
  with default ACLs that keep files created on either side accessible
  to both. (A bind mount of a host folder into the Android storage does
  not work: Android reaches its storage through a non-recursive bind of
  `/data/media`.) Before the first Waydroid session there is no storage
  yet; run `rc-service waydroid-plinius restart` after it, or reboot.
- The properties go into `waydroid.cfg` (kept by `waydroid init`); the
  package also enables the `waydroid-container` service and the Waydroid
  firewall rules (the pmOS firewall otherwise drops DHCP, DNS and
  forwarding from `waydroid0`).

The NDK only runs on x86_64, so the Mesa is built on the PC and the
package takes it prebuilt, like the input method APK; neither is kept in
git. `waydroid-ime.sh` downloads the Android build-tools and keeps the
APK signing key in `work/waydroid-ime`; keep it, Android refuses an
update signed with another key:

```sh
postmarketos/waydroid-mesa.sh   # host packages: see the script
postmarketos/waydroid-ime.sh    # needs a JDK
postmarketos/install-pmaports.sh "$(pmbootstrap config aports)"
pmbootstrap checksum waydroid-plinius
pmbootstrap build waydroid-plinius
```

On the phone, as root, install the package and download the Android
images (LineageOS 20):

```sh
apk add waydroid-plinius
waydroid init
rc-service waydroid-container start
```

Then open any Android app from the Phosh app grid; the first start boots
Android, which takes a while. `waydroid show-full-ui` shows the whole
Android UI instead. A running session keeps Android awake; stop it with
`waydroid session stop` before standby. Once a vendor image ships Mesa
26.3 or later, the Mesa overlay is no longer needed.

## pmbootstrap quirks

- `pmbootstrap checksum` with several packages may skip some: if a
  build fails with "missing in checksums", checksum them one at a time.
- `temp/` packages without `arch="aarch64"` build for the host unless
  `--arch aarch64` is given.
- With the default service manager (systemd for Phosh), pmbootstrap
  takes `phosh` from the systemd repository (version `99990.*`) instead
  of `temp/phosh`, also for `checksum`. Our image runs OpenRC: set
  `service_manager` to `openrc` for checksumming and building `phosh`,
  and build with `--force`.
- After updating pmaports, run `pmbootstrap zap` before building: build
  chroots from the older edge fail the cross builds (`cannot execute
  'cc1'`).
- On the phone, update with `apk upgrade`, not `apk upgrade -a`: the
  pmOS repository has an older `linux-postmarketos-mediatek-mt6878`
  (`7.2`, the upstream fork), and `-a` "downgrades" to it.
- `plinius-modemd` builds in QEMU (`!pmb:crossdirect`), and pmbootstrap
  3.12 still points `RUSTC_WRAPPER` at sccache there, which the build
  chroot does not have: build it with `pmbootstrap --no-ccache build
  plinius-modemd`.
- OpenRC (0.63) treats a service variable named `ready` as its readiness
  setting; don't use it for anything else in an init script.
- `--src` builds drop `kernel/time/timeconst.bc` because rsync applies the
  tree's `.gitignore` (`*.bc`); building from the tarball avoids it.
