# postmarketOS port

postmarketOS packages for the **Volla Phone Plinius** (MediaTek MT6878 /
Dimensity 7300, `volla,plinius`). They are not submitted to pmaports yet;
this directory keeps them in git so the image can be rebuilt.

| Package | pmaports path | Contents |
|---|---|---|
| `device-volla-plinius` | `device/testing/` | `deviceinfo`, WiFi NVRAM, audio init (udev), sensor udev rule |
| `linux-postmarketos-mediatek-mt6878` | `device/testing/` | our kernel, built from `karagos01/linux` at a pinned commit with `configs/linux.config` |
| `mesa` | `temp/` | Mesa 26.3.0 with Mali-G615 support; drop once Alpine has ≥ 26.3 |

With these, Phosh runs on our native MediaTek DRM driver and composites on
the GPU; touch, WiFi, Bluetooth, battery, sound, the accelerometer, the
light sensor and the proximity sensor work. The SAR sensor also exposes
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

## Build and run

```sh
postmarketos/install-pmaports.sh          # copy the packages into pmaports
pmbootstrap checksum linux-postmarketos-mediatek-mt6878 device-volla-plinius
pmbootstrap build mesa linux-postmarketos-mediatek-mt6878 device-volla-plinius
pmbootstrap install --split               # UI e.g. phosh
pmbootstrap export
```

The root image goes onto the unused `userdata` partition (`dd` from a
running Linux, label `pmOS_root`); then stop U-Boot's countdown and
`fastboot boot` the exported `boot.img`.

## pmbootstrap quirks

- Some pmbootstrap versions reject pmaports' `deviceinfo_schema.toml`
  (`header_version` has an integer interval but no `datatype = "integer"`);
  add the datatype line locally.
- `--src` builds drop `kernel/time/timeconst.bc` because rsync applies the
  tree's `.gitignore` (`*.bc`); building from the tarball avoids it.
