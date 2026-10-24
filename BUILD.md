# Build mainline OS pro Volla Phone Plinius (MT6878)

Postup od čistého PC po telefon, který sám bootuje Debian z microSD
s WiFi, modemem a mobilními daty. Všechny cesty jsou relativní ke kořeni
projektu (adresář nad `scripts/`).

## Jak telefon bootuje

```
preloader → stock LK → boot_a = U-Boot → vendor_boot_b = Linux + initramfs
                                           → switch_root na microSD (LABEL=plinius)
```

- **LK zůstává stock.** U-Boot je v `boot_a` zabalený jako „kernel“ boot
  image v4 (`out/boot-uboot.img`).
- U-Boot po 3 s odpočtu načte boot image v2 z `vendor_boot_b`
  (`out/boot-debian.img`). Když to nejde, přejde do fastboot. Volume Up
  otevře menu.
- Initramfs (Alpine) nastaví USB gadget (NCM síť 172.16.42.1 a ACM
  konzoli) a s `root=LABEL=plinius` přepne na Debian z karty. Bez `root=`
  zůstane v záchranném shellu v RAM (`out/boot-linux.img`).
- Slot B je prázdný. `vendor_boot_b` slouží jen jako úložiště boot image.

## Bezpečnostní pravidla

- **Nikdy `fastboot set_active b`** (slot B je prázdný, telefon by nenabootoval).
- **Neflashovat LK** a nic kromě `boot_a` a `vendor_boot_b`.
- Kernel stavět **jen s `LLVM=1`**. Běh přes GCC přepíše `.config` bez
  CFI/LTO a moduly pak padají na CFI.
- Přes `/dev/mem` nečíst paměť modemu (0xd0000000–0xee000000), EMI MPU
  (0x10226000), SMPU (0x10351000) ani AP 0x2xxxxxxx: telefon zamrzne nebo
  se resetuje.
- Do gitu nepatří: `out/modem-nvram` (IMEI), `backup/`, WiFi PSK, PIN SIM.

## Co je potřeba

**Hostitel** (testováno na Ubuntu 24.04):

```
sudo apt install clang lld llvm gcc-aarch64-linux-gnu make bc bison flex \
    libssl-dev libelf-dev libgnutls28-dev uuid-dev python3-dev swig zstd cpio \
    debootstrap qemu-user-static binfmt-support e2fsprogs \
    fastboot curl python3-serial
```

**Zdroje** v `src/`. Větev `plinius` zatím existuje jen lokálně:

| Adresář | Repozitář | Větev |
|---|---|---|
| `src/MT6878-Mainline_linux` | github.com/MT6878-Mainline/linux | `plinius` |
| `src/MT6878-Mainline_u-boot` | github.com/MT6878-Mainline/u-boot | `plinius` |
| `scripts/` | tento repozitář | `main` |
| `tools/mkbootimg` | AOSP mkbootimg (`mkbootimg.py`) | — |

`tools/*.c` (devmem, memdump, connlog) jsou malé pomocníky do initramfs.

**Záloha stock telefonu** v `backup/partitions/` (raw dumpy partition,
viz `backup/README.md`). Build z ní bere:

- `nvram.img`: NVRAM modemu a WiFi (kalibrace, IMEI, MAC),
- `protect1.img` a `protect2.img`: data modemu.

Firmware modemu, WiFi a BT nahrává už stock LK a za běhu se čte
z partition `modem_a`, takže v rootfs být nemusí.

**Hardware:** USB kabel (fastboot, síť), microSD karta, volitelně 1,8V
UART (FT232R, 921600 8N1 na `/dev/ttyUSB0`). UART je potřeba jen pro
`dev-boot.sh` a `update-debian.sh`, které přes něj zastaví odpočet U-Bootu.

## Build

```
WIFI_SSID=mojesit WIFI_PSK=heslo scripts/build.sh
```

`build.sh` provede kroky v tomto pořadí. Každý jde spustit i samostatně,
např. `scripts/build.sh kernel bootimg`.

| Krok | Výstup | Poznámka |
|---|---|---|
| `kernel` | `src/build-linux` (`Image.gz`, DTB, moduly) | první build vezme `scripts/configs/linux.config` |
| `uboot` | `out/boot-uboot.img` | `mt6878_plinius_defconfig`, `aarch64-linux-gnu-` |
| `initramfs` | `out/initramfs.cpio.gz` | Alpine minirootfs a balíky se stáhnou do `out/initramfs-inputs/` |
| `bootimg` | `out/boot-debian.img`, `out/boot-linux.img` | boot image v2: Image.gz + DTB + initramfs |
| `nvram` | `out/modem-nvram` | ze zálohy; existující strom nepřepisuje |
| `rootfs` | `out/debian-rootfs` | Debian trixie přes debootstrap, volá `sudo` |

Rootfs obsahuje:

- moduly kernelu, WiFi NVRAM a NVRAM modemu v `/var/lib/modem`,
- síť: usb0 172.16.42.1 s DHCP pro PC, wlan0 přes DHCP,
- ssh pro root a uživatele `volla` s klíčem `~/.ssh/id_ed25519.pub`
  (hesla jsou zamčená),
- autologin na `ttyS0` a `ttyGS0`,
- modemové služby.

Opakovaný `rootfs` krok jen aktualizuje konfiguraci, debootstrap se
neopakuje.

Změny konfigurace kernelu ukládej zpátky:
`cp src/build-linux/.config scripts/configs/linux.config`.

## První instalace

1. **microSD**: jedna ext4 partition s labelem `plinius`, na ni rootfs.
   Ve čtečce v PC:

   ```
   sudo mkfs.ext4 -L plinius /dev/sdX1
   sudo mount /dev/sdX1 /mnt/sd
   sudo cp -a out/debian-rootfs/. /mnt/sd/ && sudo umount /mnt/sd
   ```

   Alternativa bez čtečky: nabootuj `out/boot-linux.img` (initramfs v RAM),
   kartu připoj v telefonu a pošli rootfs přes ssh:
   `sudo tar -C out/debian-rootfs --numeric-owner -cpf - . | ssh root@172.16.42.1 'tar -C /mnt/sd -xpf -'`.

2. **U-Boot do `boot_a`** z fastbootu stock LK (ze stock systému
   `adb reboot bootloader`):

   ```
   fastboot flash boot_a out/boot-uboot.img
   fastboot reboot
   ```

3. **Linux do `vendor_boot_b`**, tentokrát z fastbootu U-Bootu (bez
   platného `vendor_boot_b` do něj U-Boot přejde sám):

   ```
   fastboot flash vendor_boot_b out/boot-debian.img
   fastboot reboot
   ```

4. Telefon nabootuje Debian. Na PC se objeví síťové rozhraní s adresou
   172.16.42.2, pak `ssh root@172.16.42.1`.

5. **Mobilní data:** do `/etc/default/plinius-modem` (práva 600) zapiš
   `SIM_PIN=...` a případně `APN=...`. Po restartu
   `plinius-modem.service` nastartuje modem a
   `plinius-mobile-data.service` připojí data na `ccmniN` s výchozí trasou
   s metrikou 2048 (WiFi má přednost). PIN se zadá jen při všech třech
   zbývajících pokusech. PUK nikdy automaticky.

6. **Hotspot:** do `/etc/default/plinius-hotspot` zapiš `PSK=...` (WPA2,
   8 až 63 znaků), případně `SSID` a `CHANNEL`, a pak
   `systemctl enable --now plinius-hotspot`. Hotspot běží na `ap0` vedle
   `wlan0`, klienti dostanou adresu z 192.168.77.0/24 a NAT do mobilních dat
   nebo do WiFi. Firmware umí oba režimy jen na jednom kanálu, takže
   s připojenou `wlan0` hotspot převezme její kanál.

Návrat ke stocku: `fastboot flash boot_a backup/partitions/boot_a.img`
(z fastbootu U-Bootu nebo LK).

## Vývojový cyklus

- **Nový kernel na telefon:** `scripts/update-debian.sh`. Postaví kernel,
  nahraje moduly na kartu přes ssh, zabalí `out/boot-debian.img`, telefon
  restartuje, přes UART zastaví U-Boot a flashne `vendor_boot_b`.
- **Test bez flashování:** `scripts/dev-boot.sh [sekundy] [log]` provede
  `fastboot boot out/boot-linux.img` (nebo `BOOTIMG=...`) a uloží
  sériovou konzoli do `logs/`.
- **Nový U-Boot:** `scripts/build.sh uboot`, pak z fastbootu U-Bootu
  `fastboot flash boot_a out/boot-uboot.img`. `fastboot boot` nový U-Boot
  spustit neumí.
- **FT232R** se po každém resetu telefonu zasekne: `scripts/uart-reset.sh`
  (volá sudo). Konzole U-Bootu: `scripts/ubcon.py`.

## Modem: přehled

- Kernel: `drivers/soc/mediatek/mtk-ccci.c` (modul `mtk_ccci`) nabootuje
  modem (firmware nahrál LK) a vystaví kanály jako `/dev/ccci_*` a data
  jako síťová rozhraní `ccmni0..7` (DPMAIF).
- `ccci-server.py`: RPC a FS server (NVRAM z `/var/lib/modem`), DRDI.
  S `--radio` po startu modemu zapne SIM přes MIPC.
- `mobile-data.py`: PIN, rádio, registrace, PDP kontext a potvrzení
  rozhraní (`AT+EIF ... "ifst","up"`), konfigurace `ccmniN`.
- `sms.py` (v rootfs jako `sms`): `sms send <číslo> <text>`, `sms list`,
  `sms watch [--delete]`. PDU režim, čeština (UCS2) i dlouhé zprávy.
  Příchozí SMS se ukládají na SIM (20 míst), `--delete` je po vypsání
  smaže. AT kanál je jen jeden (`/dev/ccci_at`), proto se skripty střídají
  přes `flock` na zařízení.
- Runtime suspend UFS za běhu modemu zasekne CPU, proto ho
  `plinius-modem.service` vypíná.
