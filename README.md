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
| GPU (Mali-G615 MC2) | ✅ | panthor on the GPUEB power controller (`mtk-mfg-pmdomain`); OpenGL ES 3.1 with Mesa ≥ 26.3 (in Alpine edge), devfreq 265 MHz–1.047 GHz, thermal throttling at 85 °C; Phosh composites on the GPU |

## Roadmap

- **Standby power.** The SoC reaches its deep idle states at runtime: with
  the display off the `system-vcore` cpuidle state lets SPM put the 26 MHz
  clock and VCORE to sleep (needs the APU host driver, the UFS rtff-mtcmos,
  the mtu3/SD/I2C context restore and the LVTS fixes). WiFi stays associated
  in power save with the firmware's low power mode (`mt7921_soc
  ps_mdtim=3`, NetworkManager `wifi.powersave=3`), and a gratuitous ARP every
  minute keeps the router finding the phone, since the firmware drops
  broadcast ARP in that mode. The WFDMA is reinitialised on every wake; the
  DMA is stopped and drained first, so the reinit neither wedges the link nor
  drops the frames received before it. Measured unplugged with the display
  off and WiFi associated: Debian ~20 mA (21.6 h), postmarketOS at the
  greeter ~19 mA (6.6 h overnight), about what the stock system draws idle
  (~21 mA); it was ~72 mA at the start, and is ~75 mA with WiFi power save
  off. Incoming calls and SMS still wake the phone (SMS needs
  `AT+CNMI=2,1`). The figures come from the MT6375 coulomb counter,
  calibrated (2026-11-23) against the board's SH366100 gauge: with the vendor
  DT's 1 mOhm sense resistor it counted exactly twice as much, so the DT sets
  2 mOhm. Open items:
  - On Debian, two of ~30 boots so far had WiFi authentication time out after
    boot; not reproduced on purpose, a diagnostic hook in the watchdog waits
    for it.

- **System suspend (s2idle) — parked.** Investigated on pmOS (2026-11-20).
  `s2idle` itself works: the power key and the RTC wake it and Phosh survives
  it. Because runtime VCORE idle already gives most of the saving, battery
  auto-suspend in Phosh stays **off** (`sleep-inactive-battery-type
  'nothing'`). What a proper suspend still needs:
  - **Modem wake-ups.** The CCIF interrupt is a wake source now (kernel
    `mtk-ccci: let the CCIF interrupt wake the system`), so calls and SMS
    wake the phone. But the modem also writes its NVRAM through `ccci_fs`
    (channel 14) every 20-65 s, and every write wakes the system. Needed:
    let the modem server handle the request and suspend again without
    waking userspace/the display, or find out why the modem writes so often
    (stock may batch these or the modem may expect a low-power indication
    from the AP).
  - **Display on after every resume.** gsd-power turns the screen on after
    each wake-up (laptop behaviour). Non-user wakes (modem fs, RTC) must not
    light the screen; only the power key, calls and SMS should.
  - **Phosh notification loop.** gsd-power sends a critical "Automatic
    suspend" warning; with `wakeup-screen-triggers ['urgency']` Phosh lights
    the screen, which resets the idle timer, so the phone never suspends.
    Removing `urgency` from the triggers breaks the loop (but then other
    urgent notifications no longer wake the screen either).
  - **Kernel errors on every suspend:** `mtk-mfg-pmdomain: GPUFreq IPI
    command 6 timed out`; once `irq 309: nobody cared` (`mtk_apu_irq`), after
    which the IRQ got disabled.
  - `mtu3_suspend` returns `-EBUSY` with a USB cable attached, which aborts
    the suspend (test unplugged or with RTC wake).
  - Measure the actual saving of s2idle vs. runtime VCORE idle before
    putting more work into it.
  - **Do not set `printk.console_suspend=N`** (the console debugging switch):
    the kernel then switches VT on every suspend, elogind pauses the DRM
    device, phoc loses the output and Phosh exits back to the greeter, which
    looks like a reboot.

- **postmarketOS / Phosh.** A pmOS image built with `pmbootstrap` from our
  kernel and `device-volla-plinius` packages boots into Phosh (greetd + phrog
  + phoc on the native MediaTek DRM driver). Login, touch, colours (after the
  dither and gamma fixes), WiFi scanning in NetworkManager, Bluetooth, battery
  in upower, sound in the session (PulseAudio with the UCM profile: speaker,
  earpiece and microphone, verified 2026-11-23), the accelerometer, the
  ambient light sensor and the proximity sensor (through iio-sensor-proxy) all
  work, and Phosh composites on the GPU. It runs on current edge (pmaports and
  packages updated 2026-11-23, Phosh 0.59, Mesa 26.3.1 from Alpine), and the
  kernel config passes `pmbootstrap kconfig check` (community, including
  Binder for Waydroid and ftrace). The packages (device, kernel, modem) live
  in [`postmarketos/`](postmarketos/README.md). Open items:
  - **Calls: loose ends.** Incoming and outgoing voice calls work in Phosh
    (GNOME Calls over ModemManager: MIPC call approval in the `mtk-ccci`
    plugin, call audio through UCM and `plinius-speechd`). The in-call mute
    and speaker buttons work (verified 2026-11-23): mute uses the codec's
    hardware `Mic Capture Switch`, so it also silences the modem uplink, and
    the speaker button switches PulseAudio's voice call profile through a
    patched `callaudiod` (`postmarketos/callaudiod`). Left over:
    - Screenshots fail on pmOS: phoc's image copy capture reports a 0x0
      buffer, so `grim` and Phosh's screenshot both give up.
    - The microphone level during a call (hostless modem path) once seemed
      low; a test call on 2026-11-23 sounded fine. The mic preamp is at its
      maximum (24 dB) and `plinius-speechd` leaves the modem's speech
      parameters at their defaults, so if it is too quiet after all, the
      next step is the modem's uplink digital gain.
    - GNOME Calls once disappeared when a call was accepted, after several
      ModemManager restarts during testing; not reproduced since. Watch for
      it.
  - **Waydroid.** The kernel has Binder now; Waydroid itself is untested.
  - **Compass.** iio-sensor-proxy does not pick up the magnetometer (it expects
    a tilt-compensated heading channel); low priority.

- **New userspace tools in Rust.** Kernel drivers stay in C: they mostly extend
  existing C drivers (mtk-drm, mt76, ASoC, TCPM), and the subsystems we need
  (regmap, IIO, power_supply, ASoC, mfd) have no Rust bindings yet. New
  userspace tools are written in Rust instead of Python: the modem service
  (`modemd/`, done; mobile data and SMS still Python), the GNSS host and
  similar daemons. Existing Python scripts are ported when they are reworked
  anyway, not as a separate rewrite.

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
