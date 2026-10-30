# Volla Phone Plinius — mainline Linux port

A mainline Linux (and U-Boot) port for the **Volla Phone Plinius**, a phone with
the **MediaTek MT6878** SoC. The phone boots Debian from a microSD card on its
own, with working WiFi, mobile data, SMS and Bluetooth — all through mainline
drivers, with no downstream kernel.

This repository (`plinius-mainline`) holds the **build scripts, kernel
configuration and runtime tools**. The kernel and U-Boot sources themselves live
in two separate forks (see below).

## Repositories

| Repo | Branch | Contents |
|---|---|---|
| **[karagos01/linux](https://github.com/karagos01/linux)** | `plinius` | kernel — DTS, drivers (connsys WiFi/BT, CCCI modem, MSDC) |
| **[karagos01/u-boot](https://github.com/karagos01/u-boot)** | `plinius` | bootloader — Plinius board support, boot from `vendor_boot_b` |
| **karagos01/plinius-mainline** (this one) | `main` | `build.sh`, `configs/linux.config`, boot-image packaging/flashing, runtime (modem/data/SMS/hotspot) |

Both kernels are forks of the respective `MT6878-Mainline/*` repos (the link to
upstream is preserved); the default branch is set to `plinius` on both.

## What works

| Subsystem | Status | Note |
|---|---|---|
| Boot chain | ✅ | stock LK → U-Boot (`boot_a`) → Linux (`vendor_boot_b`) → `switch_root` to microSD |
| microSD (MSDC1) | ✅ | SDR104; key fix `no_src_clk_gate` |
| Debian | ✅ | Debian trixie from microSD (`root=LABEL=plinius`) |
| USB gadget | ✅ | NCM network `172.16.42.1` + ACM console from initramfs |
| WiFi STA | ✅ | mainline mt76 / mt7921 over connsys |
| WiFi AP / hotspot | ✅ | `hotspot.sh` + hostapd |
| Modem — SIM/radio | ✅ | CCCI driver, SIM, network registration |
| Mobile data | ✅ | LTE, NetAgent EIF (`mobile-data.py`) |
| SMS | ✅ | receive and send (`sms.py`) |
| Bluetooth BR/EDR + LE | ✅ | custom `btmtk_connsys` driver (BGF subsystem, BEIF EMI-ring, WMT); scan, pairing, A2DP audio, IRQ-driven RX |

## How the phone boots

```
preloader → stock LK → boot_a = U-Boot → vendor_boot_b = Linux + initramfs
                                          → switch_root to microSD (LABEL=plinius)
```

- **LK stays stock.** U-Boot is packed as a boot image in `boot_a`.
- After a short countdown U-Boot loads Linux from `vendor_boot_b`; on failure it
  drops into fastboot (Volume Up = menu).
- The initramfs (Alpine) brings up the USB gadget and switches to Debian on the card.
- **Slot B is empty** — `vendor_boot_b` serves only as storage for the boot image.

## Build and deploy

The detailed procedure, from a clean PC to a running phone, is in
**[BUILD.md](BUILD.md)**. In short (from the project root, i.e. the directory
above `scripts/`):

```sh
# full build (kernel + U-Boot + initramfs + boot image + rootfs)
WIFI_SSID=mynetwork WIFI_PSK=password scripts/build.sh

# first flash over fastboot (U-Boot to boot_a, Linux to vendor_boot_b)
#   see BUILD.md

# update a running system (rebuild kernel + modules + boot image, reflash)
scripts/update-debian.sh
```

For a module-only change (without changing the DTB) it is enough to repack the
modules onto the phone and reboot — no UART reflash. A DTB change requires
`update-debian.sh`.

## Important rules

- Build the kernel **only with `LLVM=1`** (GCC rewrites `.config` without CFI/LTO
  → modules crash).
- **Never `fastboot set_active b`** (slot B is empty) and **do not flash LK**.
- Do not read modem, EMI MPU or SMPU memory over `/dev/mem` — the phone freezes.
- Secrets do not belong in git: WiFi PSK, SIM PIN/PUK, `out/modem-nvram`
  (IMEI) or `backup/`.

More in [BUILD.md](BUILD.md).
