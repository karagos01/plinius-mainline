# Volla Phone Plinius — mainline Linux port

Port mainline Linuxu (a U-Bootu) na **Volla Phone Plinius**, telefon s SoC
**MediaTek MT6878**. Telefon sám nabootuje Debian z microSD karty a funguje
na něm WiFi, mobilní data, SMS i Bluetooth — všechno přes mainline ovladače,
bez downstream kernelu.

Tenhle repozitář (`plinius-mainline`) drží **buildovací skripty, kernel
konfiguraci a runtime nástroje**. Samotné zdrojáky kernelu a U-Bootu jsou
ve dvou samostatných forcích (viz níže).

## Repozitáře

| Repo | Větev | Obsah |
|---|---|---|
| **[karagos01/linux](https://github.com/karagos01/linux)** | `plinius` | kernel — DTS, ovladače (connsys WiFi/BT, CCCI modem, MSDC) |
| **[karagos01/u-boot](https://github.com/karagos01/u-boot)** | `plinius` | bootloader — board support Plinius, boot z `vendor_boot_b` |
| **karagos01/plinius-mainline** (tento) | `main` | `build.sh`, `configs/linux.config`, balení/flashování boot image, runtime (modem/data/SMS/hotspot) |

Oba kernely jsou forky příslušných `MT6878-Mainline/*` repů (zachovaná vazba
na upstream); výchozí větev je u obou nastavená na `plinius`.

## Co funguje

| Subsystém | Stav | Poznámka |
|---|---|---|
| Boot chain | ✅ | stock LK → U-Boot (`boot_a`) → Linux (`vendor_boot_b`) → `switch_root` na microSD |
| microSD (MSDC1) | ✅ | SDR104; klíčový fix `no_src_clk_gate` |
| Debian | ✅ | Debian trixie z microSD (`root=LABEL=plinius`) |
| USB gadget | ✅ | NCM síť `172.16.42.1` + ACM konzole z initramfs |
| WiFi STA | ✅ | mainline mt76 / mt7921 nad connsys |
| WiFi AP / hotspot | ✅ | `hotspot.sh` + hostapd |
| Modem — SIM/radio | ✅ | CCCI ovladač, SIM, registrace do sítě |
| Mobilní data | ✅ | LTE, NetAgent EIF (`mobile-data.py`) |
| SMS | ✅ | příjem i odesílání (`sms.py`) |
| Bluetooth BR/EDR + LE | ✅ | vlastní `btmtk_connsys` ovladač (BGF subsystém, BEIF EMI-ring, WMT); scan, párování, A2DP audio, IRQ-driven RX |

## Jak telefon bootuje

```
preloader → stock LK → boot_a = U-Boot → vendor_boot_b = Linux + initramfs
                                          → switch_root na microSD (LABEL=plinius)
```

- **LK zůstává stock.** U-Boot je zabalený jako boot image v `boot_a`.
- U-Boot po krátkém odpočtu načte Linux z `vendor_boot_b`; při neúspěchu jde
  do fastboot (Volume Up = menu).
- Initramfs (Alpine) nahodí USB gadget a přepne na Debian z karty.
- **Slot B je prázdný** — `vendor_boot_b` slouží jen jako úložiště boot image.

## Build a nasazení

Podrobný postup od čistého PC až po běžící telefon je v **[BUILD.md](BUILD.md)**.
Ve zkratce (z kořene projektu, tj. adresáře nad `scripts/`):

```sh
# kompletní build (kernel + U-Boot + initramfs + boot image + rootfs)
WIFI_SSID=mojesit WIFI_PSK=heslo scripts/build.sh

# první flash přes fastboot (U-Boot do boot_a, Linux do vendor_boot_b)
#   viz BUILD.md

# update běžícího systému (rebuild kernelu + modulů + boot image, reflash)
scripts/update-debian.sh
```

Pro změnu jen v modulech (bez změny DTB) stačí přebalit moduly na telefon a
rebootovat — bez UART reflashe. Změna DTB vyžaduje `update-debian.sh`.

## Důležitá pravidla

- Kernel stavět **jen s `LLVM=1`** (GCC přepíše `.config` bez CFI/LTO → moduly padají).
- **Nikdy `fastboot set_active b`** (slot B je prázdný) a **neflashovat LK**.
- Přes `/dev/mem` nečíst paměť modemu, EMI MPU ani SMPU — telefon zamrzne.
- Do gitu nepatří tajné hodnoty: WiFi PSK, PIN/PUK SIM, `out/modem-nvram`
  (IMEI) ani `backup/`.

Víc v [BUILD.md](BUILD.md).
