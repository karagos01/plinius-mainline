# GNSS/GPS na mainline (MT6878 connsys)

Stav: **funguje — reálný 3D fix na mainline.** Tento dokument shrnuje, jak
to běží, a kde je strop (TTFF) a proč.

## Architektura

GPS je 3. connsys subsystém (vedle WiFi a BT, sdílený A-die MT6637). Řetězec:

```
/dev/gpsdl0 (mtk-connsys-gnss, char dev)
   └─ stock gpsdl ioctl ABI  ← drží ji libmnl (MTK PVT engine) z userspace
        └─ connsys GPS L1 DSP (RAM image přes MVCD/SMC do TF-A)
```

- **Kernel driver**: `drivers/soc/mediatek/mtk-connsys-gnss.c` (větev
  `plinius-gps-mt`). `open()` vyveze link0 na RESET_DONE, nahodí reaktivní
  D2A drain a IRQ; **MVCD download RAM image nechává na userspace** (stock
  `mnld`/`libmnl`), což je cesta, která dala první fix. In-kernel boot
  (MVCD přes SMC/TF-A) je zachován za modul paramem `inkernel_boot=1`.
- **Userspace**: MediaTek `libmnl.so` (PVT engine) spuštěná přes Android
  NDK `linker64` tenkým hostem `mnl_host`. Host registruje callback
  tabulku (NMEA ven) a volá `mtk_gps_mnl_run()`. Config + NVRAM v
  `/data/vendor/gps/` (`MNL_Config.xml`, `mtkgps.dat`).
- **eLNA**: externí GPS L1/L5 eLNA na GPIO186/187. Permanentně nahozené
  přes **pinctrl v DT** (`gps_elna_pins`, `output-high` na gnss uzlu).
  Bez nich DSP viděl málo/žádné satelity (AGC na max zisku).

## Výsledek

Otevřená obloha: **$GNGSA,A,3** (3D fix), ~5 SV použito, HDOP ~1.3–1.5,
horizontální přesnost **~4–5 m**, plný 1 Hz NMEA stream
($GPGGA/RMC/GSA/GSV + PMTK diagnostika).

## TTFF a aiding — co jde a co ne

Vendor stack má v `MNL_Config.xml` zapnuté všechny aiding featury
(MDTime, Time_Source, CoTMS, MD-TSX). Náš host je ale záměrně tenký a
krmí jen NMEA callback. Rozbor (REA nad vendor `libmnl.so`) ukázal dvě
oddělené větve aidingu:

1. **Čas (coarse UTC) — řešeno zadarmo.** Engine čte systémový čas přímo:
   `hal_sys_time_get` → `mnl_sys_time_read` → libc `time()` + `gmtime()`.
   Telefon je NTP-synced (systemd-timesyncd), takže engine dostane přesný
   čas bez modemu. To je důvod, proč **warm start funguje**: persistovaná
   ephemeris v `mtkgps.dat` + správný systémový čas → okamžitě použitelná.
   (`"RTC NoRead"` v logu = nenaplněný **hardwarový RTC GNSS čipu**, ne
   Linux hodiny — neškodné.) Push API `mtk_gps_set_time()` existuje, ale
   je redundantní.

2. **Frekvence (MD-TSX) — jediná zbývající páka, blokovaná.** To, co by
   zkrátilo TTFF k vendor úrovni (~44 s), je kontinuální TCXO S-křivka z
   **modemu** (`md_tsx_req` / callback `MTK_AGPS_CB_MD_TSX_DATA_SYNC_REQ`
   / `mtk_gps_tsx_get_md_scurve`). Modem má TCXO disciplinovaný buňkovou
   sítí; sdílením téhle frekvenční reference engine udrží nosnou
   stabilnější → ephemeris subrámce dekódují bez bitových chyb → kratší
   TTFF. Bez toho GPS běží na vlastním connsys krystalu (funguje, jen s
   větší frekvenční nejistotou → pomalejší dekód ephemeris).

### Proč MD-TSX teď nejde

- Data tečou přes **connsys↔modem (MD) hardwarové IPC**, které na mainline
  **neexistuje** — byl by to samostatný modem↔GPS IPC driver (velký).
- Přístup do paměti modemu je u tohoto portu **zakázaný** (zamrzá/resetuje
  telefon) — bezpečnostní strop bring-upu.
- Frekvenční offset navíc neumíme sami změřit; engine si ho učí sám
  (TSXPoly) a persistuje v `mtkgps.dat`, což přesně dělá warm start.

### Shrnutí

| Lever | Stav |
|-------|------|
| eLNA L1/L5 (GPIO186/187) | ✅ v DT (pinctrl output-high) |
| Warm/hot start (persist `mtkgps.dat`) | ✅ ~95 s, 3× rychlejší než cold |
| Čas (systémový NTP → engine) | ✅ automaticky |
| EPO (predikované dráhy) | ❌ download 403 (chce MTK partner auth) |
| MD-TSX frekvenční aiding | ⛔ chce modem↔GPS IPC bridge (budoucí, velký) + naráží na zákaz modem-mem |

**Závěr:** ~95 s warm start je praktické dno mainline userspace stacku.
Uzavření mezery k vendor ~44 s vyžaduje modem↔GPS IPC bridge — samostatný
budoucí projekt, mimo rozsah bezpečného bring-upu.
