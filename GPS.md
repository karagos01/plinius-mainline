# GNSS/GPS on mainline (MT6878 connsys)

Status: **works — a real 3D fix on mainline.** This document summarizes how it
runs, and where the ceiling (TTFF) is and why.

## Architecture

GPS is the 3rd connsys subsystem (alongside WiFi and BT, sharing the A-die
MT6637). The chain:

```
/dev/gpsdl0 (mtk-connsys-gnss, char dev)
   └─ stock gpsdl ioctl ABI  ← held by libmnl (MTK PVT engine) from userspace
        └─ connsys GPS L1 DSP (RAM image via MVCD/SMC into TF-A)
```

- **Kernel driver**: `drivers/soc/mediatek/mtk-connsys-gnss.c` (branch
  `plinius-gps-mt`). `open()` brings link0 to RESET_DONE, sets up the reactive
  D2A drain and IRQ; **it leaves the MVCD RAM-image download to userspace** (stock
  `mnld`/`libmnl`), which is the path that produced the first fix. In-kernel boot
  (MVCD via SMC/TF-A) is kept behind the module param `inkernel_boot=1`.
- **Userspace**: MediaTek `libmnl.so` (PVT engine) run via the Android NDK
  `linker64` with a thin host `mnl_host`. The host registers a callback table
  (NMEA out) and calls `mtk_gps_mnl_run()`. Config + NVRAM in
  `/data/vendor/gps/` (`MNL_Config.xml`, `mtkgps.dat`).
- **eLNA**: an external GPS L1/L5 eLNA on GPIO186/187. Permanently raised via
  **pinctrl in DT** (`gps_elna_pins`, `output-high` on the gnss node). Without
  them the DSP saw few/no satellites (AGC at max gain).

## Result

Open sky: **$GNGSA,A,3** (3D fix), ~5 SV used, HDOP ~1.3–1.5, horizontal
accuracy **~4–5 m**, a full 1 Hz NMEA stream ($GPGGA/RMC/GSA/GSV + PMTK
diagnostics).

## TTFF and aiding — what works and what doesn't

The vendor stack has all aiding features enabled in `MNL_Config.xml` (MDTime,
Time_Source, CoTMS, MD-TSX). Our host, however, is deliberately thin and only
feeds the NMEA callback. Analysis (REA over the vendor `libmnl.so`) showed two
separate aiding branches:

1. **Time (coarse UTC) — handled for free.** The engine reads the system time
   directly: `hal_sys_time_get` → `mnl_sys_time_read` → libc `time()` +
   `gmtime()`. The phone is NTP-synced (systemd-timesyncd), so the engine gets an
   accurate time without the modem. That is why **warm start works**: persisted
   ephemeris in `mtkgps.dat` + the correct system time → immediately usable.
   (`"RTC NoRead"` in the log = the unpopulated **hardware RTC of the GNSS chip**,
   not the Linux clock — harmless.) The push API `mtk_gps_set_time()` exists, but
   is redundant.

2. **Frequency (MD-TSX) — the only remaining lever, blocked.** What would shorten
   TTFF to the vendor level (~44 s) is the continuous TCXO S-curve from the
   **modem** (`md_tsx_req` / callback `MTK_AGPS_CB_MD_TSX_DATA_SYNC_REQ` /
   `mtk_gps_tsx_get_md_scurve`). The modem has its TCXO disciplined by the
   cellular network; by sharing this frequency reference the engine keeps the
   carrier more stable → the ephemeris subframes decode without bit errors →
   shorter TTFF. Without it, GPS runs on its own connsys crystal (works, just with
   larger frequency uncertainty → slower ephemeris decode).

### Why MD-TSX doesn't work now

- The data flows over the **connsys↔modem (MD) hardware IPC**, which **does not
  exist** on mainline — it would be a separate modem↔GPS IPC driver (large).
- Access to modem memory is **forbidden** on this port (it freezes/resets the
  phone) — a safety ceiling of the bring-up.
- Besides, we can't measure the frequency offset ourselves; the engine learns it
  on its own (TSXPoly) and persists it in `mtkgps.dat`, which is exactly what
  warm start does.

### Summary

| Lever | Status |
|-------|--------|
| eLNA L1/L5 (GPIO186/187) | ✅ in DT (pinctrl output-high) |
| Warm/hot start (persisted `mtkgps.dat`) | ✅ ~95 s, 3× faster than cold |
| Time (system NTP → engine) | ✅ automatic |
| EPO (predicted orbits) | ❌ download 403 (needs MTK partner auth) |
| MD-TSX frequency aiding | ⛔ needs a modem↔GPS IPC bridge (future, large) + hits the modem-mem ban |

**Conclusion:** ~95 s warm start is the practical floor of the mainline userspace
stack. Closing the gap to the vendor's ~44 s requires a modem↔GPS IPC bridge — a
separate future project, out of scope for the safe bring-up.
