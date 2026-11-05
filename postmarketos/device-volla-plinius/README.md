# device-volla-plinius (DRAFT postmarketOS port)

A draft [pmaports](https://gitlab.postmarketos.org/postmarketOS/pmaports)
device package for the **Volla Phone Plinius** (MediaTek MT6878 / Dimensity
7300, `volla,plinius`). It is **not yet submitted**; this directory lets us
track the port in the pmOS standard while the kernel stays separate from the
OS, exactly as pmOS itself does (kernel = its own APKBUILD, rootfs = assembled
by pmbootstrap).

## Kernel relationship (why this is low-risk)

Our mainline tree (`src/MT6878-Mainline_linux`, remote
`github.com/MT6878-Mainline/linux`) is the **same upstream** pmOS packages as
`linux-postmarketos-mediatek-mt6878` (maintainer BotchedRPR / Igor Belwon).

- pmOS kernel package pins `_commit=12f2e9f0…` on branch `postmarketOS/v7.2`.
- That commit is **exactly the merge-base** of our `plinius` branch.
- `plinius` = `12f2e9f0` + **188 commits** of Volla Plinius bring-up
  (audio, USB-C/OTG host, GPS, SCP sensors, haptics, modem, wifi, BT, RTC,
  LVTS, MT6375 subPMIC, SPMI, pinctrl…). We are **0 commits behind**.

So we are a clean superset of what pmOS already ships — no divergence to
untangle. The upstream path is: land our 188 commits into
`MT6878-Mainline/linux` (or mainline proper); pmOS then bumps `_commit` and
picks them up for free.

## Files

| File | Role |
|------|------|
| `deviceinfo`     | canonical device description (boot offsets, screen, dtb, cmdline) |
| `APKBUILD`       | device package; `depends` on the kernel package + `mkbootimg` |
| `modules-initfs` | initramfs modules — empty, everything needed is `=y` |

## How the boot values were derived

Everything in `deviceinfo` mirrors `scripts/mk-linux-bootimg.sh`, the v2
boot.img this device already boots via U-Boot:

```
header_version 2   pagesize 2048 (default)
base           0x40000000
kernel_offset  0x00080000   ramdisk_offset 0x06000000
dtb_offset     0x05f00000   second 0x00f00000 (default)  tags 0x00000100 (default)
dtb            mediatek/mt6878-volla-plinius.dtb   (passed separately, append_dtb=false)
cmdline        earlycon=uart8250,mmio32,0x11001000 console=tty0 console=ttyS0,921600 clk_ignore_unused
```

Boot chain on the device: **LK → U-Boot (in `boot_a`) → U-Boot loads a v2
boot.img**. U-Boot provides `fastboot boot`, so `deviceinfo_flash_method` is
`fastboot`. Permanent install is a `dd` of the boot.img to **`vendor_boot_b`
(`/dev/sdc66`, 64 MiB)**; U-Boot's `normal_boot` autoboots that slot. This is
non-standard for pmbootstrap's flasher — persist manually for now.

## Trying it with pmbootstrap (not yet done)

```sh
# point pmbootstrap at a pmaports checkout that contains this folder under
# device/testing/device-volla-plinius/, then:
pmbootstrap checksum device-volla-plinius
pmbootstrap init                 # pick volla-plinius, no UI or a simpledrm-friendly UI
pmbootstrap build device-volla-plinius
pmbootstrap install              # rootfs onto microSD
# boot: fastboot boot $(pmbootstrap export) ... then dd to /dev/sdc66 to persist
```

The kernel can come from the published `linux-postmarketos-mediatek-mt6878`
package (which currently lacks our 188 commits) or from our tree via
`pmbootstrap build --src /path/to/MT6878-Mainline_linux linux-postmarketos-mediatek-mt6878`.

## Open gaps / TODO before this is real

- [ ] **Kernel not upstreamed.** Our 188 commits aren't in `MT6878-Mainline`
      yet, so the stock pmOS kernel package boots but has no audio/USB-host/GPS/
      sensors. Land them upstream, or ship a `--src` build.
- [ ] **No touchscreen on mainline.** The DTS has no touch node yet, so an
      on-screen unlock/UI is limited; console + SSH + USB gadget work. `drm`
      is `true` via simpledrm (no GPU accel / Mali driver).
- [ ] **Install target.** microSD (`mmc1`) is proven; internal UFS as the pmOS
      root is untested here.
- [ ] **GPS approach.** pmOS kernel config enables `gnss-serial`
      (`CONFIG_GNSS_MTK_SERIAL`); our GPS is a `soc/mediatek/mtk-connsys-gnss`
      connsys-DSP driver. Reconcile before upstreaming.
- [ ] **Submit** as `device/testing/device-volla-plinius/` once the above
      settle, plus a `dt-bindings`/MAINTAINERS nod to Volla.
