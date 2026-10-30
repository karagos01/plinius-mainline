# Building the mainline OS for the Volla Phone Plinius (MT6878)

The procedure from a clean PC to a phone that boots Debian from microSD on its
own, with WiFi, modem and mobile data. All paths are relative to the project root
(the directory above `scripts/`).

## How the phone boots

```
preloader → stock LK → boot_a = U-Boot → vendor_boot_b = Linux + initramfs
                                           → switch_root to microSD (LABEL=plinius)
```

- **LK stays stock.** U-Boot is packed in `boot_a` as a "kernel" boot image v4
  (`out/boot-uboot.img`).
- After a 3 s countdown U-Boot loads a boot image v2 from `vendor_boot_b`
  (`out/boot-debian.img`). If that fails, it drops into fastboot. Volume Up opens
  the menu.
- The initramfs (Alpine) sets up the USB gadget (NCM network 172.16.42.1 and an
  ACM console) and, with `root=LABEL=plinius`, switches to Debian on the card.
  Without `root=` it stays in a rescue shell in RAM (`out/boot-linux.img`).
- Slot B is empty. `vendor_boot_b` serves only as storage for the boot image.

## Safety rules

- **Never `fastboot set_active b`** (slot B is empty, the phone would not boot).
- **Do not flash LK** and nothing other than `boot_a` and `vendor_boot_b`.
- Build the kernel **only with `LLVM=1`**. Running it through GCC rewrites
  `.config` without CFI/LTO and the modules then crash on CFI.
- Do not read modem memory (0xd0000000–0xee000000), EMI MPU (0x10226000), SMPU
  (0x10351000) or AP 0x2xxxxxxx over `/dev/mem`: the phone freezes or resets.
- These do not belong in git: `out/modem-nvram` (IMEI), `backup/`, WiFi PSK,
  SIM PIN.

## What you need

**Host** (tested on Ubuntu 24.04):

```
sudo apt install clang lld llvm gcc-aarch64-linux-gnu make bc bison flex \
    libssl-dev libelf-dev libgnutls28-dev uuid-dev python3-dev swig zstd cpio \
    debootstrap qemu-user-static binfmt-support e2fsprogs \
    fastboot curl python3-serial
```

**Sources** in `src/`. The `plinius` branch is on the forks under the karagos01
account:

| Directory | Repository | Branch |
|---|---|---|
| `src/MT6878-Mainline_linux` | github.com/karagos01/linux (fork) | `plinius` |
| `src/MT6878-Mainline_u-boot` | github.com/karagos01/u-boot (fork) | `plinius` |
| `scripts/` | github.com/karagos01/plinius-mainline | `main` |
| `tools/mkbootimg` | AOSP mkbootimg (`mkbootimg.py`) | — |

`tools/*.c` (devmem, memdump, connlog) are small helpers for the initramfs.

**A stock phone backup** in `backup/partitions/` (raw partition dumps, see
`backup/README.md`). The build takes from it:

- `nvram.img`: modem and WiFi NVRAM (calibration, IMEI, MAC),
- `protect1.img` and `protect2.img`: modem data.

The modem, WiFi and BT firmware is loaded by stock LK and read at runtime from
the `modem_a` partition, so it does not need to be in the rootfs.

**Hardware:** a USB cable (fastboot, network), a microSD card, optionally a 1.8V
UART (FT232R, 921600 8N1 on `/dev/ttyUSB0`). The UART is needed only for
`dev-boot.sh` and `update-debian.sh`, which use it to stop the U-Boot countdown.

## Build

```
WIFI_SSID=mynetwork WIFI_PSK=password scripts/build.sh
```

`build.sh` runs the steps in this order. Each can also be run on its own, e.g.
`scripts/build.sh kernel bootimg`.

| Step | Output | Note |
|---|---|---|
| `kernel` | `src/build-linux` (`Image.gz`, DTB, modules) | the first build takes `scripts/configs/linux.config` |
| `uboot` | `out/boot-uboot.img` | `mt6878_plinius_defconfig`, `aarch64-linux-gnu-` |
| `initramfs` | `out/initramfs.cpio.gz` | the Alpine minirootfs and packages are downloaded into `out/initramfs-inputs/` |
| `bootimg` | `out/boot-debian.img`, `out/boot-linux.img` | boot image v2: Image.gz + DTB + initramfs |
| `nvram` | `out/modem-nvram` | from the backup; does not overwrite an existing tree |
| `rootfs` | `out/debian-rootfs` | Debian trixie via debootstrap, calls `sudo` |

The rootfs contains:

- kernel modules, WiFi NVRAM and modem NVRAM in `/var/lib/modem`,
- network: usb0 172.16.42.1 with DHCP for the PC, wlan0 via DHCP,
- ssh for root and the `volla` user with the key `~/.ssh/id_ed25519.pub`
  (passwords are locked),
- autologin on `ttyS0` and `ttyGS0`,
- the modem services.

Re-running the `rootfs` step only updates the configuration; debootstrap is not
repeated.

Save kernel config changes back:
`cp src/build-linux/.config scripts/configs/linux.config`.

## First installation

1. **microSD**: a single ext4 partition labeled `plinius`, with the rootfs on it.
   In a card reader on the PC:

   ```
   sudo mkfs.ext4 -L plinius /dev/sdX1
   sudo mount /dev/sdX1 /mnt/sd
   sudo cp -a out/debian-rootfs/. /mnt/sd/ && sudo umount /mnt/sd
   ```

   Alternative without a reader: boot `out/boot-linux.img` (initramfs in RAM),
   attach the card in the phone and send the rootfs over ssh:
   `sudo tar -C out/debian-rootfs --numeric-owner -cpf - . | ssh root@172.16.42.1 'tar -C /mnt/sd -xpf -'`.

2. **U-Boot into `boot_a`** from stock LK's fastboot (from the stock system
   `adb reboot bootloader`):

   ```
   fastboot flash boot_a out/boot-uboot.img
   fastboot reboot
   ```

3. **Linux into `vendor_boot_b`**, this time from U-Boot's fastboot (without a
   valid `vendor_boot_b`, U-Boot drops into it on its own):

   ```
   fastboot flash vendor_boot_b out/boot-debian.img
   fastboot reboot
   ```

4. The phone boots Debian. A network interface with the address 172.16.42.2
   appears on the PC, then `ssh root@172.16.42.1`.

5. **Mobile data:** write `SIM_PIN=...` and optionally `APN=...` into
   `/etc/default/plinius-modem` (mode 600). After a restart,
   `plinius-modem.service` starts the modem and
   `plinius-mobile-data.service` brings up data on `ccmniN` with a default route
   at metric 2048 (WiFi has priority). The PIN is entered only when all three
   remaining tries are available. PUK never automatically.

6. **Hotspot:** write `PSK=...` (WPA2, 8 to 63 characters) into
   `/etc/default/plinius-hotspot`, optionally `SSID` and `CHANNEL`, then
   `systemctl enable --now plinius-hotspot`. The hotspot runs on `ap0` alongside
   `wlan0`, clients get an address from 192.168.77.0/24 and NAT to mobile data or
   to WiFi. The firmware can run both modes only on a single channel, so with
   `wlan0` connected the hotspot takes over its channel.

Returning to stock: `fastboot flash boot_a backup/partitions/boot_a.img`
(from U-Boot's or LK's fastboot).

## Development cycle

- **New kernel onto the phone:** `scripts/update-debian.sh`. It builds the
  kernel, uploads the modules to the card over ssh, packs `out/boot-debian.img`,
  restarts the phone, stops U-Boot over UART and flashes `vendor_boot_b`.
- **Test without flashing:** `scripts/dev-boot.sh [seconds] [log]` runs
  `fastboot boot out/boot-linux.img` (or `BOOTIMG=...`) and saves the serial
  console to `logs/`.
- **New U-Boot:** `scripts/build.sh uboot`, then from U-Boot's fastboot
  `fastboot flash boot_a out/boot-uboot.img`. `fastboot boot` cannot launch a new
  U-Boot.
- **FT232R** gets stuck after every phone reset: `scripts/uart-reset.sh`
  (calls sudo). U-Boot console: `scripts/ubcon.py`.

## Modem: overview

- Kernel: `drivers/soc/mediatek/mtk-ccci.c` (module `mtk_ccci`) boots the modem
  (firmware loaded by LK) and exposes channels as `/dev/ccci_*` and data as the
  network interfaces `ccmni0..7` (DPMAIF).
- `ccci-server.py`: the RPC and FS server (NVRAM from `/var/lib/modem`), DRDI.
  With `--radio` it turns on the SIM over MIPC after the modem starts.
- `mobile-data.py`: PIN, radio, registration, PDP context and interface
  confirmation (`AT+EIF ... "ifst","up"`), `ccmniN` configuration and the
  operator DNS for systemd-resolved.
- `sms.py` (in the rootfs as `sms`): `sms send <number> <text>`, `sms list`,
  `sms watch [--delete]`. PDU mode, Czech (UCS2) and long messages too. Incoming
  SMS are stored on the SIM (20 slots), `--delete` removes them after printing.
  There is only one AT channel (`/dev/ccci_at`), so the scripts take turns via
  `flock` on the device.
- UFS runtime suspend while the modem runs hangs the CPU, so
  `plinius-modem.service` disables it.
