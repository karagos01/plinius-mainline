# Upstream-readiness of the Plinius port (MT6878)

Goal: get the whole port into a state where it could be submitted to mainline —
i.e. split into logical patch series, checkpatch-clean, with a `dt-bindings` YAML
for every new `compatible`, with MAINTAINERS coverage and `Signed-off-by`.
**This is not about actually sending it to LKML** (that is a separate step), but
about preparing for it. This document is a tracker across sessions — keep the
status updated.

Branch: `plinius` in both forks (`karagos01/linux`, `karagos01/u-boot`).
Kernel port = **36 commits** on top of a shallow baseline (7.2.0),
root `12f2e9f04`. The U-Boot port is a separate tree (see below).

## Partial findings (beyond compatible/checkpatch)

- `phy/mediatek,tphy.yaml`: port commit `c83b414` added the property
  `mediatek,software-role-switch` to the DTS, but the binding has no type
  definition for it → `dt_binding_check` reports "missing type definition". Add
  it to the tphy YAML (`$ref: /schemas/types.yaml#/definitions/flag` or bool).
- **dt tooling**: `dt_binding_check`/`dtbs_check` need `dtschema` +
  `yamllint`. On this PC there is a venv in the scratchpad
  (`dtschema-venv`, dtschema 2026.9). For a clean PC: `python3 -m venv v &&
  v/bin/pip install dtschema yamllint`, then put the venv `bin` on PATH.

## Global tasks (apply to all commits)

- [x] **maintainer in bindings + MAINTAINERS** ✅ (2026-10-30, kernel commit
  `72bcbbbec`) — placeholder `Kara` → `Jan Benedikt <karagos01@gmail.com>` in
  all 12 new bindings `maintainers:` + 4 MAINTAINERS `M:` lines (13 files).
  NOTE: `Signed-off-by:`/git-author in the commits was still `kara <karagos01@gmail.com>`
  — for a real DCO submission these also need to be rewritten to `Jan Benedikt`
  (120-commit filter-branch + force-push); done later (see diary 2026-10-30).
- [x] **`Signed-off-by:`** ✅ (2026-10-28, global finale) — added to all
  **96** port commits (`12f2e9f04..plinius`) via `git filter-branch --msg-filter`:
  `Signed-off-by: kara <karagos01@gmail.com>` (from git config; the name was a
  placeholder, like the maintainer line — later finalized). Tree unchanged (only
  messages). Backup branch `plinius-pre-sob` (local). Force-pushed to fork/plinius.
- [x] **`Co-authored-by:` casing** ✅ (same) — `Co-Authored-By:`→`Co-authored-by:`
  on all 96. checkpatch confirms this as the "preferred signature form". NOTE:
  `Co-authored-by` is NOT a kernel-standard tag (the kernel has `Co-developed-by:`),
  so checkpatch ALWAYS leaves 1 warning "Non-standard signature" — that is
  unavoidable, the attribution line is required by the harness. Lowercasing does
  remove the extra "preferred form" warning + SoB removes the error. Per commit:
  **1 err/3 warn → 0 err/1–2 warn**.
- [x] **checkpatch** ✅ — after the finale, **0 errors** per commit; the remaining
  1 warning = the unavoidable non-standard `Co-authored-by` (attribution).
  Effectively clean.
- [ ] **`dtbs_check` / `dt_binding_check`** — run after finishing the bindings:
  `make -C <linux> O=<build> ARCH=arm64 LLVM=1 dt_binding_check dtbs_check`.
- [x] **MAINTAINERS** ✅ (2026-10-30) — 4 dedicated sections added (commit
  `0ca475646`): MT6363/MT6685 SPMI PMIC + RTC, MT6369 regulator, MT6375 SubPMIC
  (mfd/adc/charger/gauge), and MT6878 SoC + Volla Phone Plinius (connsys/ccci/gnss/
  disp-refresh/scp + board DTS). `F:` lines sorted, sections in alphabetical
  position (MT6363<6369<6375<6735<6878<76). Verified with `get_maintainer.pl`:
  all new files now resolve to the author (previously only to generic subsystem
  maintainers). checkpatch 0 err / only the attribution warn. Name = same
  placeholder `Kara <karagos01@gmail.com>` as in the bindings — later finalized.

## Patch series (recommended order by dependencies)

The port can be split into these series. Each can be sent on its own (later ones
depend on earlier ones via the DTS).

| # | Series | Commits | Note |
|---|---|---|---|
| A | SPMI + PMIC base | `c83b414` phy tphy fix, `7447ea1` + `c61f5a2` pinctrl mt6878, `d54ab73` + `3a2cfd8` spmi pmif, `f640ad4` mfd MT6363, `b9bf837` input MT6363 keys | base for everything on SPMI |
| B | PMIC secondary + regulators | `2549b86` mfd MT6685, `e3f37a1` mt6685 chip-id fix, `12d7ecd` regulator MT6369, `4448aa6` MT6369 SD LDO | |
| C | Clock + power domains | `90db7ad` clk topckgen, `379a3d8` pmdomain + CONN | |
| D | Display refresh (boot panel) | `c91d646` disp-refresh, `0ce6405` backlight | nice-to-have, can be deferred |
| E | microSD | `06e71f6` mmc mtk-sd MT6878 | binding ✅ done |
| F | connsys (shared WiFi+BT) | `cdcea87` soc connsys power control | depends on C |
| G | WiFi (mt76/mt7921) | `7badf53` mt792x fw/dma, `9cda963` mt7921 tx_prepare move, `94b6250` mt7921 MT6878, `19cbe90` AP mode | depends on F |
| H | Bluetooth | `2a7035c` driver, `a836b57` BD addr, `518cc39` IRQ/fixup, `d7c9e03` PSM | depends on F |
| I | Modem (CCCI) | `562522` soc mtk-ccci | largest, most nits |
| K | MT6375 battery (charger/gauge/adc) | `b5f7616` mfd core, `17ae8b2` iio adc, `70ee1c6` charger, `f16e849` gauge, `19fccd5` dts, `6450927`+`e9bf234` bindings | independent, on i2c5; bindings ✅, device-tested ✅ |
| L | Stability/QA | `48c4d32` mtk-ccci drop trace_printk; config (cpuidle PSCI, FUNCTION_TRACER off); `6f12477` thermal lvts mt6878 + `627d8b5` dts (throttling) + `37500d0` dt-bindings lvts mt6878; **RTC MT6685**: `0c5f904` rtc driver + `9c19c7c` dts + `644b429` mfd SCK_TOP irq-domain + `e939268` rtc alarm + `969f1a9` dts alarm + `8b5797d` dt-bindings | device-tested ✅ (RTC time+alarm, persistence across power-cycle); thermal uncalibrated (efuse=devinfo, dead end); RTC binding ✅; **lvts binding ✅** (mt6878-lvts-ap/-mcu, clocks/resets/nvmem optional for mt6878) |
| J | DTS integration | all `arch/arm64/.../mt6878*` and `volla-plinius*` | last, ties A–I together |

U-Boot: a separate series for u-boot upstream (board `mt6878_plinius`) — out of
scope for this audit for now.

## dt-bindings status by compatible

✅ done · 🟡 extend existing · ❌ write new YAML

| compatible | binding | path | status |
|---|---|---|---|
| `mediatek,mt6878-mmc` | mtk-sd (modified) | `mmc/mtk-sd.yaml` | ✅ |
| `mediatek,mt6363` | new (in port) | `mfd/mediatek,mt6363.yaml` | ⚠️ (see below) |
| `mediatek,mt6363-keys` | extended | `input/mediatek,pmic-keys.yaml` + keys in mt6363.yaml | ✅ |
| `mediatek,mt6363-regulator` | exists | `regulator/mediatek,mt6363-regulator.yaml` | ✅ |
| `mediatek,mt6369-regulator` | new | `regulator/mediatek,mt6369-regulator.yaml` | ✅ |
| `mediatek,mt6685` | new (+ RTC child + interrupt-controller) | `mfd/mediatek,mt6685.yaml` | ✅ |
| `mediatek,mt6685-rtc` | new (child in mt6685 mfd) | `mfd/mediatek,mt6685.yaml` | ✅ |
| `mediatek,mt6878-spmi` | extended (+ interrupts) | `spmi/mtk,spmi-mtk-pmif.yaml` | ✅ |
| `mediatek,mt6878-power-controller` | extended | `power/mediatek,power-controller.yaml` | ✅ |
| `mediatek,mt6878-infracfg-ao` | extend | `clock/mediatek,infracfg.yaml` (syscon) | ❌ (see syscon family) |
| `mediatek,mt6878-connsys` | new | `soc/mediatek/mediatek,mt6878-connsys.yaml` | ✅ |
| `mediatek,mt6878-wifi` | extended | `net/wireless/mediatek,mt76.yaml` | ✅ |
| `mediatek,mt6878-bluetooth` | new | `net/bluetooth/mediatek,mt6878-bluetooth.yaml` | ✅ |
| `mediatek,mt6878-ccci` | new | `soc/mediatek/mediatek,mt6878-ccci.yaml` | ✅ |
| `mediatek,mt6878-disp-refresh` | new | `soc/mediatek/mediatek,mt6878-disp-refresh.yaml` | ✅ |
| `volla,plinius` + vendor `volla` | extended (+ nothing,tetris) | `arm/mediatek.yaml` + `vendor-prefixes.yaml` | ✅ |
| `mediatek,mt6375` | new | `mfd/mediatek,mt6375.yaml` (adc inline) | ✅ |
| `mediatek,mt6375-charger` | new | `power/supply/mediatek,mt6375-charger.yaml` | ✅ |
| `mediatek,mt6375-gauge` | new | `power/supply/mediatek,mt6375-gauge.yaml` | ✅ |
| `mediatek,mt6375-adc` | new (inline in MFD) | `mfd/mediatek,mt6375.yaml` | ✅ |

**Driver-bindings done: 12/14.** Remaining: the syscon family (infracfg-ao) and
the mt6363.yaml fix — both in the DTS-integration backlog below.

## DTS-integration backlog (series J) — from the full board `dtbs_check`

A real `dtbs_check` on `mt6878-volla-plinius.dtb` showed that, besides the new
drivers, the DTS nodes that reuse existing IP blocks with different/extra
properties, or that only have a "syscon" fallback, need tuning:

- **`mt6363.yaml` (added by the port) is broken:** it hangs on a missing
  `regulator/mediatek,mt6373-regulator.yaml` (unresolvable $ref) and its own
  example fails (`adc@1000: reg` doesn't match). Fix: either write the mt6373
  regulator binding, or remove the mt6373 branch from `oneOf` (the board doesn't
  use it); and fix the adc example.
- **syscon family `mediatek,mt6878-*cfg-ao`** — "failed to match any schema":
  `infracfg-ao`, `pericfg-ao`, `ufscfg-ao`, `ufscfg-pdn`,
  `imp_iic_wrap-cs/ws/ess`. Each has a specific compatible + `"syscon"` fallback.
  Add them to the appropriate mediatek syscon/infracfg bindings (or create one).
- **UFS** ✅ DONE (2026-10-28, per-SoC route): new `mediatek,mt6878-ufshci`
  compatible — binding `ufs/mediatek,ufs.yaml` extended (13-clock branch, resets,
  vcc-supply optional for mt6878), driver `ufs-mediatek.c` match entry (no data,
  doesn't branch on compatible), DTS compatible + `ufsphy@`→`ufs-phy@` rename.
  dt_binding_check + dtbs_check clean; behavior-neutral (driver takes clk/reset
  by name). Commits `8dec57b`(binding) `83b5f52`(driver) `653d2f4`(dts).
- **USB** ✅ DONE (2026-10-28): `mediatek,mt6878-mtu3` + `-xhci` compatibles
  (with a generic fallback that the driver matches → NO driver patch), bindings
  `usb/mediatek,mtu3.yaml` (clock-names relaxed to an unordered enum — driver
  takes clk by name) + `mtk-xhci.yaml`, DTS nodename `usb@`, removed a bogus
  `phy-cells`, `host_ck`→`xhci_ck` (downstream name the mainline driver ignores).
  dt_binding_check + dtbs_check clean. Commits `4b1c1e6`(binding) `2215f26`(dts).
  **DEVICE-VERIFIED 2026-10-28 via reflash**: taint=0, RNDIS gadget works (ssh
  over it), mtu3 bound as `11201000.usb`, compatible `mt6878-mtu3`. UFS also
  verified: `/dev/sdc` 238 GB + host0 with `mt6878-ufshci`. The only log
  `ufshci: Failed to get reset control mphy_rst: -2` = a benign pre-existing one
  (the optional 4th reset is not in DT; we didn't touch resets).
- **tphy** ✅ DONE (2026-10-28): `mediatek,mt6878-tphy` (generic-tphy-v2
  instance) instead of the placeholder `mt6785-tphy`; the binding got a `port`
  graph in the usb-phy child (software-role-switch requires it, but it was
  missing). The driver matches the generic-tphy-v2 fallback → no patch,
  behavior-neutral. dt_binding_check + dtbs_check clean. Commits `3f3709f`(binding)
  `5349100`(dts). board 13→10.
- **topckgen** `syscon@10000000`: `hw-voter-regmap` is not in the binding.
- **watchdog** `watchdog@1c00a000` (`mediatek,mt6878-wdt`): extra `reboot-mode`
  + compatible oneOf; add mt6878-wdt to the mtk-wdt binding.

These are "series J" and there are quite a few — smaller than the new drivers,
but scattered across many existing bindings. Do them gradually, each with
`dtbs_check`.

### Backlog status after the first batch (2026-10-24)

Done from the backlog: **pericfg-ao** (clock/pericfg.yaml), **infracfg-ao**
(mfd/syscon.yaml), **topckgen + apmixedsys + imp_iic_wrap×4**
(new clock/mediatek,mt6878-clock.yaml). Remaining nodes from the full board
`dtbs_check` (count = errors per node):

- ~~`mediatek,mt6878-pinctrl`~~ ✅ — the binding ALREADY existed (Collabora, from
  baseline), but our DTS/driver didn't match it. Reconciled: reg-names
  base/bl/.../rt (driver base_names the same, mapped by name), removed 4
  duplicate EINT regs, pin-group nodes `*-state`→`*-pins`, and added R1R0 bias
  values to the binding (MSDC1 uses them). Commits 5a357ca97/b2a2bb347/84a1a4c27.
  **Verified on the chip** (reflash update-debian.sh): pinctrl, microSD, WiFi, BT
  OK, dtbs_check clean.
- ~~`mediatek,mt6878-i2c`~~ ✅ — the compatible was already in `i2c/i2c-mt65xx.yaml`;
  the errors were the downstream props `scl-gpio-id`/`sda-gpio-id` on i2c7,
  removed from the DTS (the driver doesn't use them). Commit `9a8aa6729`.
- ~~`mediatek,mt6878-hwv`~~ ✅ → mfd/syscon.yaml. ~~`mediatek,mt6878-scpsys`~~ ✅
  → added to `mfd/mediatek,mt8195-scpsys.yaml` (shared scpsys binding; the
  power-controller child validates). Commits 07bfaedf3.
- ~~**ufscfg-ao/pdn** (parent)~~ ✅ — new `clock/mediatek,mt6878-ufscfg.yaml`
  (clock + syscon + simple-mfd + reset child). Commit 60fbb3abd. WATCH OUT: the
  `ti,syscon-reset` child still reports "no schema" — `reset/ti-syscon-reset.txt`
  is an old .txt (not YAML). **Upstream gap, not our port** — it affects all
  ti,syscon-reset users. (Fix = convert ti-syscon-reset.txt→yaml, a separate
  patch to the TI maintainers.)
- ~~**mmc** reg~~ ✅ — mt6878-mmc added to the 2-reg (top base) enum in mtk-sd.yaml.
  Commit 384aaece8.

**Remaining — requires a DECISION / dialogue with maintainers (not clean binding fixes):**
- **UFS**: `ufshci@112b0000` uses `mediatek,mt8183-ufshci`, but has an extra 13
  clocks (the binding expects fewer), is missing vcc-supply, extra resets/reset-names;
  the `ufsphy@112a0000` nodename doesn't match. → either introduce
  `mediatek,mt6878-ufshci/-ufsphy` (new compatible + driver data), or extend the
  mt8183 binding. Decision.
- **USB**: `usb0` (`mediatek,mtu3`) + `xhci0` (`mediatek,mtk-xhci`) use GENERIC
  compatibles (not in the enum), nodename `^usb(@.*)?`, clock-names don't match.
  → introduce per-SoC `mediatek,mt6878-mtu3/-xhci`. Decision + driver.
- **keys** (`mt6363-keys`): our DTS + port-driver read `interrupts`/`interrupt-names`
  from the PARENT node (`platform_get_irq_byname`, pinctrl-mtk-pmic-keys.c:398),
  but pmic-keys.yaml expects the interrupts on the CHILD key nodes. **Design
  divergence** introduced by the port vs the mainline convention. Either (a)
  rewrite driver+DTS to the child model (deployed power/volume buttons → test), or
  (b) justify the parent model with maintainers and extend the binding. Decision.
- **watchdog** (`mt6878-wdt`): 3-compatible (+ "syscon") and a `reboot-mode` child
  (syscon-reboot-mode) — mtk-wdt.yaml doesn't model it. Driver+binding dialogue.
- **tphy** `t-phy@11f40000` (`mediatek,mt6785-tphy`,`generic-tphy-v2`): oneOf
  fail + `usb-phy@0` `port` prop. (~~`software-role-switch` missing type~~ ✅
  fixed — `type: boolean` into tphy.yaml, commit 22f793e31.) The rest is reused
  mt6785-tphy — verify the per-SoC compatible.
- ~~**framebuffer** `fb-panel`~~ ✅ — removed the `panel`/`fb-panel` child from the
  DTS (the driver doesn't read it), commit e06b48c4e.
- **idle-states**: nodes `clusteroff-b`/`cpuoff-l`/… don't match `^(cpu|cluster)-`,
  `entry-method` should be `psci`, + non-standard MTK states (mcusysoff/s2idle/
  system-vcore). **Moved to item 4 (stability)** — it changes CPU idle/power.
- **mt6363.yaml** (Collabora, not ours): a dangling `mt6373-regulator` $ref + the
  adc example. Either write the mt6373 binding, or remove the mt6373 oneOf branch.

**Note:** nothing-tetris (a foreign board) has its own `key-default` pin node —
out of scope for the Plinius port.

Precedent to use: `net/bluetooth/mediatek,mt7622-bluetooth.yaml` (SoC-integrated
BT), `power/mediatek,power-controller.yaml`,
`net/wireless/mediatek,mt76.yaml`.

## checkpatch nits to clean up (real = total − 2 attribution)

| commit | real warnings | priority |
|---|---|---|
| `562522` CCCI | ~10 (+ length) | high — largest driver |
| `94b6250` mt7921 MT6878 | ~6 warn + 13 checks | high |
| `cdcea87` connsys, `2a7035c` BT, `12d7ecd` MT6369, `f640ad4` MT6363, `379a3d8` pmdomain, `4248fec`/`3067f54`/`67ee229`/`5431b9a` DTS | 3–4 | medium |
| the rest | 0–1 | low |

Specific nits are produced by `./scripts/checkpatch.pl --strict -g <commit>`.

## Procedure (order of work)

1. **Bindings bottom-up** (series A→J), always run `dt_binding_check` on the new
   YAML immediately.
2. After each series, `dtbs_check` against `mt6878-volla-plinius.dtb`.
3. checkpatch nits within the given series (not in bulk — keep it bisectable).
4. Finally the global tasks (SoB, casing) as a final rebase before submission.

## Diary

- 2026-10-24: audit done (36 commits, 0 checkpatch errors). Document created.
- 2026-10-24: binding `mediatek,mt6878-bluetooth` written (modeled on
  mt7622-bluetooth) and verified — `dt_binding_check` and `dtbs_check` clean.
  Commit in the kernel repo. Found the tphy type-definition problem (see above).
- 2026-10-24: connsys + WiFi (mt76.yaml) bindings done and verified.
- 2026-10-24: binding batch finished — **12/14 driver-bindings done**:
  ccci (new), mt6369-regulator (new), mt6685 (new), mt6363-keys (pmic-keys +
  mt6363.yaml), spmi (+interrupts, also fixes mt8186), power-controller,
  disp-refresh (new), board volla,plinius+nothing,tetris, vendor volla
  (Hallo Welt Systeme UG). All dt_binding_check + dtbs_check clean, commits
  pushed to karagos01/linux plinius.
  Remaining: syscon family (`mt6878-*cfg-ao`) + mt6363.yaml fix + the whole
  DTS-integration backlog (UFS/USB/tphy/topckgen/wdt — see the section above).
  Then checkpatch nits (series I/CCCI the most) and the global SoB/casing.
- 2026-10-24: backlog batch 1 — pericfg-ao → pericfg.yaml, infracfg-ao →
  syscon.yaml, and a new **clock/mediatek,mt6878-clock.yaml** (topckgen,
  apmixedsys, imp_iic_wrap ×4). All dt_binding_check + dtbs_check clean,
  pushed. The full board dtbs_check revealed further NEW compatibles without a
  binding: **mt6878-pinctrl** (large, priority) and **mt6878-i2c** — checkpatch
  didn't catch them (baseline). Exact remainder see the backlog section.
- 2026-10-24: **pinctrl done + verified on the chip** — the binding existed, the
  DTS/driver reconciled to it (reg-names, dup regs, *-pins nodes, R1R0 bias).
  i2c resolved (downstream props out of the DTS).
- 2026-10-25: backlog batch 2 — **hwv** + **scpsys** (syscon), **ufscfg** parent
  (new clock+reset binding), **mmc** reg (2-reg enum). All dt_binding_check +
  dtbs_check clean, pushed. Investigated and classified as DECISION items (not
  clean fixes): UFS/USB (reused generic compatibles → per-SoC), keys (parent vs
  child interrupt model — a divergence introduced by the port), watchdog
  (reboot-mode child), tphy, idle-states, framebuffer fb-panel, mt6363.yaml.
  Upstream gap: ti,syscon-reset without YAML. **All clean binding fixes of the
  port are done** — the rest needs your decision on per-SoC compatibles /
  restructuring deployed nodes, or dialogue with upstream maintainers. Then
  checkpatch nits (CCCI) + SoB/casing.
- 2026-10-28: **GPS works on mainline** — a real 3D fix (open sky, ~5 SV,
  ~4–5 m). Checkpoint committed (local-only): eLNA L1/L5 pinctrl in the DTS
  (`plinius`) + R1 driver defaulting to userspace-driven MVCD, in-kernel boot
  behind a module param (`plinius-gps-mt`). TTFF lever analyzed (REA over the
  vendor `libmnl.so`): time aiding runs for free (the engine reads the system NTP
  time), the only remaining lever = MD-TSX frequency aiding over the modem↔GPS
  IPC bridge (unreachable without a large driver + it hits the modem-mem ban).
  ~95 s warm start = the practical floor. Details see `GPS.md`.
- 2026-10-28 (item 2, further batch): three new bindings, all dt_binding_check +
  dtbs_check clean, pushed to `plinius`: **mt6685 RTC** child + interrupt-ctrl
  (`8b5797d`), **lvts mt6878-ap/-mcu** (`37500d0`, clocks/resets/nvmem optional),
  **mt6878-gps** connsys GNSS (`3c7d264`). Full board dtbs_check: **33 → 30**
  warnings. Driver-bindings 14/14 (all our `.c` compatibles have a binding).
  The remaining 30 = only DECISION/dialogue items, confirmed live on the chip:
  **UFS live** (`/dev/sdc` 238 GB internal + host0) → RESOLVED per-SoC compatible
  `mediatek,mt6878-ufshci` (binding+driver+dts, commits 8dec57b/83b5f52/653d2f4);
  **USB mtu3/xhci live** (RNDIS UDC) → RESOLVED `mt6878-mtu3`/`-xhci` with a
  fallback, no driver patch (4b1c1e6/2215f26); board 33→22→**13**,
  **USB mtu3 live** (`11201000.usb0` = our RNDIS UDC, generic `mediatek,mtu3` +
  `mtk-xhci` → wants `mt6878-mtu3`/`-xhci` + nodename usb@/usb0@), watchdog
  (syscon+reboot-mode, no mainline precedent), tphy (mt6785-tphy + port graph),
  idle-states (custom names + entry-method ≠ psci), mt6363-keys (parent-vs-child
  irq), soc/pinctrl ranges. Upstream gap (not ours): ti,syscon-reset without YAML (2).
- 2026-10-28 (cont.): **tphy** ✅ (`mt6878-tphy` + port graph, see above) and
  **soc/pinctrl ranges** ✅ — the pinctrl node got a unit-address `pinctrl@10005000`
  (it had reg without an @address → simple-bus treated it as a sub-bus and wanted
  ranges); behavior-neutral, commit `e3fec00`. Board **13→10→9**. Remaining 9:
  watchdog (3), idle-states (2), mt6363-keys (2) = decision items; ti,syscon-reset
  (2) = upstream gap, unsolvable cleanly from our side.
- 2026-10-28 (decision batch, user): **watchdog** → DROP reboot-mode +
  "syscon" (no mainline precedent; the mtk_wdt restart stays). `mt6878-wdt` into
  the mtk-wdt.yaml enum + DTS cleanup + `#reset-cells`. Commits `205fd10`/`43f21fc`.
  **mt6363-keys** → the investigation corrected the premise: the driver reads the
  irq `platform_get_irq_byname` from the PARENT = UPSTREAM BASELINE (not a port
  divergence; memory had it wrong). Upstream mt6358 keys has no irq in the DTS
  (the MFD supplies it); mt6363 SPMI needs them explicitly on the parent, where
  the driver reads them. So NOT a restructure (that would steer the driver away
  from baseline + risk the power key), but a binding FIX: `interrupts`/`interrupt-names`
  allowed on the parent in pmic-keys.yaml. Only the binding, no DTS/driver/reflash.
  Commit `7283330`. **idle-states** → KEEP (2 warnings): the deep states are
  device-measured and heavily used (usage system-vcore 15851, mcusysoff 8261);
  dropping them = a power regression; a clean fix = extend the core ARM
  idle-states.yaml = out of scope. **FINAL STATE board dtbs_check: 33 → 4.** The
  remaining 4 = idle-states (2, intentionally kept) + ti,syscon-reset (2, upstream
  gap without YAML). **Everything in our remit is done.** Driver-bindings 14/14 +
  6 existing ones extended (ufs/mtu3/xhci/tphy/wdt/pmic-keys) + lvts + gps.
  UFS/USB device-verified via reflash.
- 2026-10-30: **MAINTAINERS entries** added (commit `0ca475646`, see the global
  tasks above) — 4 new sections, all new files resolve to the author. checkpatch
  audit across all 120 commits: **0 errors**, the rest = ~100 warn / ~81 check,
  but the type histogram shows it's mostly noise/CHECK-level/false-positive
  (pointer `*` alignment, macro-arg reuse, usleep_range CHECK in places where part
  of the udelay is in atomic context, commit-message line-wrap) + 29× exactly that
  MAINTAINERS reminder (now addressed). **Real remainder of item 2 = (a) your full
  name** (blocks the maintainer line in bindings and in MAINTAINERS), **(b)** the
  optional micro-cleanup of checkpatch nits (bisect-risky rebase across 65 commits,
  low value), **(c)** the intentionally-kept idle-states (2) + the upstream
  ti,syscon-reset gap (2). Clean binding fixes, SoB, casing, dtbs_check 33→4 = done.
- 2026-10-28: **watchdog cleanup device-verified** via reflash (`out/boot-debian.img`
  → `vendor_boot_b`). Boot OK on the first try, taint=0; live DTB `watchdog@1c00a000`
  = `mt6878-wdt`+`mt6589-wdt`, `#reset-cells=1`, reboot-mode child gone; `mtk-wdt`
  probed ("Watchdog enabled, timeout=31 sec"), restart handler from mtk_wdt;
  cpuidle deep states (mcusysoff/system-vcore) still used, no regression. No new
  dmesg errors from that change.
- 2026-10-30: **name finalized + SoB/author rewrite** — the maintainer-line/binding
  placeholder `Kara` → `Jan Benedikt <karagos01@gmail.com>` (kernel commit
  `72bcbbbec`), and a `git filter-branch` over `12f2e9f04..plinius` rewrote
  author+committer `kara`→`Jan Benedikt` and SoB `kara`→`Jan Benedikt` across all
  123 commits (Co-authored-by Claude lines untouched; tree identical). Force-pushed
  to fork/plinius. Backups: `plinius-pre-name` + the older `plinius-pre-sob`.
  Item 2 brought to a clean stop; the remainder is either the low-value checkpatch
  micro-nit cleanup or intentionally-kept items.
- 2026-10-31: **USB-C OTG / host (DRD) fully works on mainline — functional
  bring-up, not just the dtbs/binding work above.** The USB-C port is dual-role:
  PD sink (charging) + data device (RNDIS gadget) + **host (OTG)** with runtime
  role switching driven by the MT6375 TCPM/TCPC. Device-verified on a reflash:
  a USB flash drive enumerates (`/dev/sdd`), vfat+iso9660 mount read-only, and
  **runtime hot-plug works** — plugging the drive at runtime (incl. right after
  the phone had been charging from a PC as a sink) switches to host and
  enumerates; detach returns to gadget. The functional series (11 commits pushed
  to fork/plinius, on top of the binding/dtbs work):
  `a58b04c9` tcpci: core helper to report self-sourced VBUS present,
  `20d0339f` tcpci_mt6375: report VBUS present while sourcing OTG,
  `eeb5812a` mtu3: set PHY mode on the host/device role switch,
  `a034d1ff` dts: source OTG VBUS from the xhci host node,
  `dfaca16c` pmdomain mt6878: add the SSUSB power domain,
  `def2cf8d` mtu3: bring up the MT6878 host resources on the role switch
  (adds `ssusb_hwrscs_req()` gated by DT `mediatek,hwrscs-vers`, exports
  `ssusb_ip_sw_reset()`, per-switch IP reset + xHCI teardown/rebuild),
  `7009da91` + `c8acc948` dts/dtsi: wire + enable the host path,
  `c04c16092` dts: gate OTG VBUS via GPIO 110 (`otg_vbus_en` regulator-fixed
  fed from `mt6375_otg_vbus`),
  `25fc5708` tcpci_mt6375: program the hardware Alert mask at init (fixes the
  edge-latched PD_EVT that wedged TCPM on hot-plug),
  `c15c9116` mtu3: power-gate the IP across a device->host role switch (the final
  hot-plug fix — power-gates the SSUSB IP to its sleep state between roles and
  enables VBUS before the root hub scan, so a switch taken after the port was a
  sink no longer comes up PORTSC.CCS=0; mirrors the stock `switch_port_to_off`,
  found by RE of the vendor `mtu3.ko`/`phy-mtk-xsphy.ko`; exports
  `ssusb_host_enable()`/`ssusb_host_disable()`).
  Also needs CONFIG_USB_STORAGE=y + USB_UAS=y (+ MODULE_ALLOW_BTF_MISMATCH=y so
  the =m fs/storage modules load). **Upstream-readiness TODO for this series
  (new work beyond the 33→4 dtbs audit):** (1) add the new DT property
  `mediatek,hwrscs-vers` to `usb/mediatek,mtu3.yaml`; (2) the two tcpci commits
  touch shared core (`drivers/usb/typec/tcpm/tcpci.c` + `include/linux/usb/tcpci.h`,
  a new `->get_vbus`-style helper) — needs justification / maintainer dialogue,
  not a Plinius-only change; (3) the `mtu3_dr.c` role-switch rework
  (ip_sw_reset/hwrscs/power-gate/VBUS-ordering) is driver-internal — checkpatch +
  a cover-letter explaining the MT6878 quirk; (4) DTS: the `otg_vbus_en`
  regulator-fixed, the `&ssusb vbus-supply`, and the FSA4480 typec-mux +
  usb-c-connector OF-graph (needs `fcs,fsa4480` = CONFIG_TYPEC_MUX_FSA4480, already
  an upstream binding). **TEST-METHOD NOTE for anyone re-verifying:** the first
  boot right after a `fastboot flash` is flaky for USB host (pre-existing
  first-boot-after-flash issue) — always re-test hot-plug on a second, plain
  `reboot -f` boot.
