# Upstream-readiness portu Plinius (MT6878)

Cíl: dostat celý port do stavu, ve kterém by šel poslat do mainline — tzn.
rozdělený do logických patch sérií, checkpatch-clean, s `dt-bindings` YAML
pro každý nový `compatible`, s pokrytím v MAINTAINERS a s `Signed-off-by`.
**Tohle není o reálném rozeslání na LKML** (to je samostatný krok), ale o
přípravě k němu. Dokument je tracking napříč sessions — stav aktualizuj.

Větev: `plinius` v obou forcích (`karagos01/linux`, `karagos01/u-boot`).
Kernel port = **36 commitů** nad shallow baseline (7.2.0),
root `12f2e9f04`. U-Boot port je samostatný strom (viz níže).

## Dílčí nálezy (mimo compatible/checkpatch)

- `phy/mediatek,tphy.yaml`: port commit `c83b414` přidal do DTS property
  `mediatek,software-role-switch`, ale binding pro ni nemá type definici →
  `dt_binding_check` hlásí „missing type definition". Doplnit do tphy YAML
  (`$ref: /schemas/types.yaml#/definitions/flag` nebo bool).
- **dt tooling**: `dt_binding_check`/`dtbs_check` potřebují `dtschema` +
  `yamllint`. Na tomhle PC je venv ve scratchpadu
  (`dtschema-venv`, dtschema 2026.9). Pro čistý PC: `python3 -m venv v &&
  v/bin/pip install dtschema yamllint`, pak venv `bin` na PATH.

## Globální úkoly (platí pro všechny commity)

- [ ] **maintainer v bindings** — zatím `Kara <karagos01@gmail.com>` (bez
  příjmení). Před submission doplnit celé jméno.
- [ ] **`Signed-off-by:`** — mainline ho vyžaduje na každém commitu. Teď
  commity mají jen `Co-Authored-By:`. Před odesláním přidat SoB autora
  (`git rebase --signoff` nebo za každou zprávu).
- [ ] **`Co-authored-by:` casing** — checkpatch hlásí „Non-standard
  signature". Mainline chce malé `Co-authored-by:`. (Náš harness generuje
  `Co-Authored-By:`; při přípravě k submission přepsat na malé.)
- [ ] **checkpatch `--strict`** — projet znovu po úklidu nitů (viz tabulka).
  Stav teď: **0 errors** napříč všemi 36 commity; warnings jsou z větší
  části jen ty dvě atribuční (= fakticky čisté).
- [ ] **`dtbs_check` / `dt_binding_check`** — spustit po dopsání bindings:
  `make -C <linux> O=<build> ARCH=arm64 LLVM=1 dt_binding_check dtbs_check`.
- [ ] **MAINTAINERS** — nové `.c` spadají pod existující subsystémové sekce
  (MediaTek SoC, Bluetooth, regulator, mfd…), takže nové sekce skoro
  netřeba. Zvážit `F:` řádky pro nové soubory a board entry pro Vollu.

## Patch série (doporučené pořadí dle závislostí)

Port se dá rozdělit do těchto sérií. Každou lze poslat samostatně (pozdější
závisí na dřívějších přes DTS).

| # | Série | Commity | Poznámka |
|---|---|---|---|
| A | SPMI + PMIC základ | `c83b414` phy tphy fix, `7447ea1` + `c61f5a2` pinctrl mt6878, `d54ab73` + `3a2cfd8` spmi pmif, `f640ad4` mfd MT6363, `b9bf837` input MT6363 keys | základ pro vše na SPMI |
| B | PMIC sekundární + regulátory | `2549b86` mfd MT6685, `e3f37a1` mt6685 chip-id fix, `12d7ecd` regulator MT6369, `4448aa6` MT6369 SD LDO | |
| C | Clock + power domény | `90db7ad` clk topckgen, `379a3d8` pmdomain + CONN | |
| D | Display refresh (boot panel) | `c91d646` disp-refresh, `0ce6405` backlight | nice-to-have, lze odložit |
| E | microSD | `06e71f6` mmc mtk-sd MT6878 | binding ✅ hotový |
| F | connsys (sdílí WiFi+BT) | `cdcea87` soc connsys power control | závisí na C |
| G | WiFi (mt76/mt7921) | `7badf53` mt792x fw/dma, `9cda963` mt7921 tx_prepare move, `94b6250` mt7921 MT6878, `19cbe90` AP mode | závisí na F |
| H | Bluetooth | `2a7035c` driver, `a836b57` BD addr, `518cc39` IRQ/fixup, `d7c9e03` PSM | závisí na F |
| I | Modem (CCCI) | `562522` soc mtk-ccci | největší, nejvíc nitů |
| K | MT6375 baterie (charger/gauge/adc) | `b5f7616` mfd core, `17ae8b2` iio adc, `70ee1c6` charger, `f16e849` gauge, `19fccd5` dts, `6450927`+`e9bf234` bindings | nezávislá, na i2c5; bindings ✅, device-tested ✅ |
| L | Stabilita/QA | `48c4d32` mtk-ccci drop trace_printk; config (cpuidle PSCI, FUNCTION_TRACER off); `6f12477` thermal lvts mt6878 + `627d8b5` dts (throttling) + `37500d0` dt-bindings lvts mt6878; **RTC MT6685**: `0c5f904` rtc driver + `9c19c7c` dts + `644b429` mfd SCK_TOP irq-domain + `e939268` rtc alarm + `969f1a9` dts alarm + `8b5797d` dt-bindings | device-tested ✅ (RTC time+alarm, persistence přes power-cycle); thermal nekalibrovaný (efuse=devinfo, dead end); RTC binding ✅; **lvts binding ✅** (mt6878-lvts-ap/-mcu, clocks/resets/nvmem optional pro mt6878) |
| J | DTS integrace | všechny `arch/arm64/.../mt6878*` a `volla-plinius*` | poslední, váže A–I |

U-Boot: samostatná série do u-boot upstreamu (board `mt6878_plinius`) —
zatím mimo rozsah tohoto auditu.

## Stav dt-bindings podle compatible

✅ hotovo · 🟡 rozšířit existující · ❌ napsat nový YAML

| compatible | binding | cesta | stav |
|---|---|---|---|
| `mediatek,mt6878-mmc` | mtk-sd (upraveno) | `mmc/mtk-sd.yaml` | ✅ |
| `mediatek,mt6363` | nový (v portu) | `mfd/mediatek,mt6363.yaml` | ⚠️ (viz níže) |
| `mediatek,mt6363-keys` | rozšířeno | `input/mediatek,pmic-keys.yaml` + keys v mt6363.yaml | ✅ |
| `mediatek,mt6363-regulator` | existuje | `regulator/mediatek,mt6363-regulator.yaml` | ✅ |
| `mediatek,mt6369-regulator` | nový | `regulator/mediatek,mt6369-regulator.yaml` | ✅ |
| `mediatek,mt6685` | nový (+ RTC child + interrupt-controller) | `mfd/mediatek,mt6685.yaml` | ✅ |
| `mediatek,mt6685-rtc` | nový (child v mt6685 mfd) | `mfd/mediatek,mt6685.yaml` | ✅ |
| `mediatek,mt6878-spmi` | rozšířeno (+ interrupty) | `spmi/mtk,spmi-mtk-pmif.yaml` | ✅ |
| `mediatek,mt6878-power-controller` | rozšířeno | `power/mediatek,power-controller.yaml` | ✅ |
| `mediatek,mt6878-infracfg-ao` | rozšířit | `clock/mediatek,infracfg.yaml` (syscon) | ❌ (viz syscon rodina) |
| `mediatek,mt6878-connsys` | nový | `soc/mediatek/mediatek,mt6878-connsys.yaml` | ✅ |
| `mediatek,mt6878-wifi` | rozšířeno | `net/wireless/mediatek,mt76.yaml` | ✅ |
| `mediatek,mt6878-bluetooth` | nový | `net/bluetooth/mediatek,mt6878-bluetooth.yaml` | ✅ |
| `mediatek,mt6878-ccci` | nový | `soc/mediatek/mediatek,mt6878-ccci.yaml` | ✅ |
| `mediatek,mt6878-disp-refresh` | nový | `soc/mediatek/mediatek,mt6878-disp-refresh.yaml` | ✅ |
| `volla,plinius` + vendor `volla` | rozšířeno (+ nothing,tetris) | `arm/mediatek.yaml` + `vendor-prefixes.yaml` | ✅ |
| `mediatek,mt6375` | nový | `mfd/mediatek,mt6375.yaml` (adc inline) | ✅ |
| `mediatek,mt6375-charger` | nový | `power/supply/mediatek,mt6375-charger.yaml` | ✅ |
| `mediatek,mt6375-gauge` | nový | `power/supply/mediatek,mt6375-gauge.yaml` | ✅ |
| `mediatek,mt6375-adc` | nový (inline v MFD) | `mfd/mediatek,mt6375.yaml` | ✅ |

**Driver-bindings hotové: 12/14.** Zbývá syscon rodina (infracfg-ao) a oprava
mt6363.yaml — obojí v DTS-integračním backlogu níže.

## DTS-integrační backlog (série J) — z plného `dtbs_check` boardu

Reálné `dtbs_check` na `mt6878-volla-plinius.dtb` ukázalo, že kromě nových
driverů je potřeba doladit DTS uzly, které znovupoužívají existující IP
bloky s jinými/navíc vlastnostmi, nebo mají jen „syscon" fallback:

- **`mt6363.yaml` (přidáno portem) je rozbité:** visí na chybějícím
  `regulator/mediatek,mt6373-regulator.yaml` (unresolvable $ref) a jeho vlastní
  příklad padá (`adc@1000: reg` nematchuje). Opravit: buď dopsat mt6373
  regulator binding, nebo z `oneOf` odstranit mt6373 větev (board ho nepoužívá);
  a spravit adc příklad.
- **syscon rodina `mediatek,mt6878-*cfg-ao`** — „failed to match any schema":
  `infracfg-ao`, `pericfg-ao`, `ufscfg-ao`, `ufscfg-pdn`,
  `imp_iic_wrap-cs/ws/ess`. Každý má specifický compatible + `"syscon"` fallback.
  Doplnit je do příslušných mediatek syscon/infracfg bindings (nebo založit).
- **UFS** ✅ HOTOVO (2026-10-28, per-SoC cestou): nový `mediatek,mt6878-ufshci`
  compatible — binding `ufs/mediatek,ufs.yaml` rozšířen (13-clock větev, resets,
  vcc-supply optional pro mt6878), driver `ufs-mediatek.c` match entry (bez dat,
  nevětví se podle compatible), DTS compatible + `ufsphy@`→`ufs-phy@` rename.
  dt_binding_check + dtbs_check čisté; behavior-neutral (driver bere clk/reset
  podle jména). Commity `8dec57b`(binding) `83b5f52`(driver) `653d2f4`(dts).
- **USB** ✅ HOTOVO (2026-10-28): `mediatek,mt6878-mtu3` + `-xhci` compatibly
  (s generickým fallbackem, který driver matchuje → BEZ driver patche), bindingy
  `usb/mediatek,mtu3.yaml` (clock-names rozvolněno na unordered enum — driver bere
  clk podle jména) + `mtk-xhci.yaml`, DTS nodename `usb@`, odstraněn bogus
  `phy-cells`, `host_ck`→`xhci_ck` (downstream jméno co mainline driver nebere).
  dt_binding_check + dtbs_check čisté. Commity `4b1c1e6`(binding) `2215f26`(dts).
  POZN: změna živého RNDIS uzlu → vhodné ověřit reflashem (behavior: driver nově
  spravuje xhci_ck, sys_ck/frmcnt_ck beze změny).
- **tphy** `t-phy@11f40000` (`mediatek,mt6785-tphy`) nematchuje; souvisí
  s nálezem `mediatek,software-role-switch` missing type (viz Dílčí nálezy).
- **topckgen** `syscon@10000000`: `hw-voter-regmap` není v bindingu.
- **watchdog** `watchdog@1c00a000` (`mediatek,mt6878-wdt`): `reboot-mode`
  navíc + compatible oneOf; doplnit mt6878-wdt do mtk-wdt bindingu.

Tyhle jsou „série J" a je jich dost — menší než nové drivery, ale rozeseté
po mnoha existujících bindingách. Dělat postupně, každý s `dtbs_check`.

### Stav backlogu po první dávce (2026-10-24)

Hotovo z backlogu: **pericfg-ao** (clock/pericfg.yaml), **infracfg-ao**
(mfd/syscon.yaml), **topckgen + apmixedsys + imp_iic_wrap×4**
(nový clock/mediatek,mt6878-clock.yaml). Zbývající uzly z plného
`dtbs_check` boardu (počet = chyb na uzlu):

- ~~`mediatek,mt6878-pinctrl`~~ ✅ — binding UŽ existoval (Collabora, z baseline),
  ale náš DTS/driver mu neodpovídal. Srovnáno: reg-names base/bl/.../rt (driver
  base_names stejně, mapuje se byname), odstraněny 4 duplicitní EINT regy,
  pin-group uzly `*-state`→`*-pins`, a do bindingu doplněny R1R0 bias hodnoty
  (MSDC1 je používá). Commity 5a357ca97/b2a2bb347/84a1a4c27. **Ověřeno na čipu**
  (reflash update-debian.sh): pinctrl, microSD, WiFi, BT OK, dtbs_check čistý.
- ~~`mediatek,mt6878-i2c`~~ ✅ — compatible už v `i2c/i2c-mt65xx.yaml` byl;
  chyby byly downstream props `scl-gpio-id`/`sda-gpio-id` na i2c7, odstraněny
  z DTS (driver je nepoužívá). Commit `9a8aa6729`.
- ~~`mediatek,mt6878-hwv`~~ ✅ → mfd/syscon.yaml. ~~`mediatek,mt6878-scpsys`~~ ✅
  → přidáno do `mfd/mediatek,mt8195-scpsys.yaml` (sdílený scpsys binding; power-
  controller child validuje). Commity 07bfaedf3.
- ~~**ufscfg-ao/pdn** (parent)~~ ✅ — nový `clock/mediatek,mt6878-ufscfg.yaml`
  (clock + syscon + simple-mfd + reset child). Commit 60fbb3abd. POZOR: child
  `ti,syscon-reset` pořád hlásí „no schema" — `reset/ti-syscon-reset.txt` je
  starý .txt (není YAML). **Upstream gap, ne náš port** — postihuje všechny
  uživatele ti,syscon-reset. (Fix = konverze ti-syscon-reset.txt→yaml, samostatný
  patch TI maintainerům.)
- ~~**mmc** reg~~ ✅ — mt6878-mmc přidán k 2-reg (top base) enumu v mtk-sd.yaml.
  Commit 384aaece8.

**Zbývá — vyžaduje ROZHODNUTÍ / dialog s maintainery (ne čisté binding fixy):**
- **UFS**: `ufshci@112b0000` používá `mediatek,mt8183-ufshci`, ale má navíc 13
  clocks (binding čeká méně), chybí vcc-supply, resets/reset-names navíc;
  `ufsphy@112a0000` nodename nesedí. → buď zavést `mediatek,mt6878-ufshci/-ufsphy`
  (nový compatible + driver data), nebo rozšířit mt8183 binding. Rozhodnutí.
- **USB**: `usb0` (`mediatek,mtu3`) + `xhci0` (`mediatek,mtk-xhci`) používají
  GENERICKÉ compatibly (nejsou v enumu), nodename `^usb(@.*)?`, clock-names
  nesedí. → zavést per-SoC `mediatek,mt6878-mtu3/-xhci`. Rozhodnutí + driver.
- **keys** (`mt6363-keys`): náš DTS + port-driver čtou `interrupts`/`interrupt-names`
  z PARENT uzlu (`platform_get_irq_byname`, pinctrl-mtk-pmic-keys.c:398), ale
  pmic-keys.yaml čeká interrupty na CHILD key uzlech. **Designová divergence**
  zavedená portem vs. mainline konvence. Buď (a) přepsat driver+DTS na child-model
  (deployed power/volume tlačítka → test), nebo (b) obhájit parent-model u
  maintainerů a rozšířit binding. Rozhodnutí.
- **watchdog** (`mt6878-wdt`): 3-compatible (+ "syscon") a `reboot-mode` child
  (syscon-reboot-mode) — mtk-wdt.yaml to nemodeluje. Driver+binding dialog.
- **tphy** `t-phy@11f40000` (`mediatek,mt6785-tphy`,`generic-tphy-v2`): oneOf
  fail + `usb-phy@0` `port` prop. (~~`software-role-switch` missing type~~ ✅
  opraveno — `type: boolean` do tphy.yaml, commit 22f793e31.) Zbytek je reused
  mt6785-tphy — ověřit per-SoC compatible.
- ~~**framebuffer** `fb-panel`~~ ✅ — odstraněn `panel`/`fb-panel` child z DTS
  (driver je nečte), commit e06b48c4e.
- **idle-states**: uzly `clusteroff-b`/`cpuoff-l`/… nesedí `^(cpu|cluster)-`,
  `entry-method` má být `psci`, + nestandardní MTK stavy (mcusysoff/s2idle/
  system-vcore). **Přesunuto do bodu 4 (stabilita)** — mění CPU idle/power.
- **mt6363.yaml** (Collabora, ne-náš): viselý `mt6373-regulator` $ref + adc
  příklad. Buď dopsat mt6373 binding, nebo odstranit mt6373 oneOf větev.

**Pozn.:** nothing-tetris (cizí board) má vlastní `key-default` pin uzel — mimo
rozsah Plinius portu.

Precedens k použití: `net/bluetooth/mediatek,mt7622-bluetooth.yaml` (SoC
integrované BT), `power/mediatek,power-controller.yaml`,
`net/wireless/mediatek,mt76.yaml`.

## checkpatch nity k úklidu (reálné = total − 2 atribuční)

| commit | reálných warnings | priorita |
|---|---|---|
| `562522` CCCI | ~10 (+ delka) | vysoká — největší driver |
| `94b6250` mt7921 MT6878 | ~6 warn + 13 checks | vysoká |
| `cdcea87` connsys, `2a7035c` BT, `12d7ecd` MT6369, `f640ad4` MT6363, `379a3d8` pmdomain, `4248fec`/`3067f54`/`67ee229`/`5431b9a` DTS | 3–4 | střední |
| zbytek | 0–1 | nízká |

Konkrétní nity vytáhne `./scripts/checkpatch.pl --strict -g <commit>`.

## Postup (pořadí práce)

1. **Bindings zdola nahoru** (série A→J), vždy hned `dt_binding_check` na nový YAML.
2. Po každé sérii `dtbs_check` proti `mt6878-volla-plinius.dtb`.
3. checkpatch nity v rámci dané série (ne hromadně — ať zůstane bisektovatelné).
4. Nakonec globální úkoly (SoB, casing) jako finální rebase před submission.

## Deník

- 2026-10-24: audit hotový (36 commitů, 0 checkpatch errors). Dokument
  založen.
- 2026-10-24: binding `mediatek,mt6878-bluetooth` napsán (vzor
  mt7622-bluetooth) a ověřen — `dt_binding_check` i `dtbs_check` čisté.
  Commit v kernel repu. Nalezen tphy type-definition problém (viz výše).
- 2026-10-24: bindings connsys + WiFi (mt76.yaml) hotové a ověřené.
- 2026-10-24: dávka bindings dokončena — **12/14 driver-bindings hotovo**:
  ccci (nový), mt6369-regulator (nový), mt6685 (nový), mt6363-keys (pmic-keys +
  mt6363.yaml), spmi (+interrupty, opravuje i mt8186), power-controller,
  disp-refresh (nový), board volla,plinius+nothing,tetris, vendor volla
  (Hallo Welt Systeme UG). Vše dt_binding_check + dtbs_check čisté, commity
  pushnuté na karagos01/linux plinius.
  Zbývá: syscon rodina (`mt6878-*cfg-ao`) + mt6363.yaml oprava + celý
  DTS-integrační backlog (UFS/USB/tphy/topckgen/wdt — viz sekce výše).
  Pak checkpatch nity (série I/CCCI nejvíc) a globální SoB/casing.
- 2026-10-24: backlog dávka 1 — pericfg-ao → pericfg.yaml, infracfg-ao →
  syscon.yaml, a nový **clock/mediatek,mt6878-clock.yaml** (topckgen,
  apmixedsys, imp_iic_wrap ×4). Vše dt_binding_check + dtbs_check čisté,
  pushnuto. Plný board dtbs_check odhalil další NOVÉ compatibly bez bindingu:
  **mt6878-pinctrl** (velký, priorita) a **mt6878-i2c** — checkpatch je
  nechytil (baseline). Přesný zbytek viz backlog sekce.
- 2026-10-24: **pinctrl hotový + ověřený na čipu** — binding existoval, DTS/driver
  k němu srovnány (reg-names, dup regy, *-pins uzly, R1R0 bias). i2c vyřešeno
  (downstream props z DTS).
- 2026-10-25: dávka 2 backlogu — **hwv** + **scpsys** (syscon), **ufscfg** parent
  (nový clock+reset binding), **mmc** reg (2-reg enum). Všechny dt_binding_check
  + dtbs_check čisté, pushnuto. Vyšetřeny a zařazeny k ROZHODNUTÍ (ne čisté fixy):
  UFS/USB (reused generic compatibly → per-SoC), keys (parent vs child interrupt
  model — portem zavedená divergence), watchdog (reboot-mode child), tphy, idle-
  states, framebuffer fb-panel, mt6363.yaml. Upstream gap: ti,syscon-reset bez
  YAML. **Všechny čisté binding fixy portu jsou hotové** — zbytek chce tvoje
  rozhodnutí o per-SoC compatible / restrukturalizaci deployed uzlů, nebo dialog
  s upstream maintainery. Pak checkpatch nity (CCCI) + SoB/casing.
- 2026-10-28: **GPS funguje na mainline** — reálný 3D fix (otevřená obloha,
  ~5 SV, ~4–5 m). Checkpoint commitnut (local-only): eLNA L1/L5 pinctrl v DTS
  (`plinius`) + R1 driver default na userspace-driven MVCD, in-kernel boot za
  modul paramem (`plinius-gps-mt`). TTFF lever rozebrán (REA nad vendor
  `libmnl.so`): čas aiding běží zadarmo (engine čte systémový NTP čas), jediná
  zbývající páka = MD-TSX frekvenční aiding přes modem↔GPS IPC bridge
  (nedosažitelné bez velkého driveru + naráží na zákaz modem-mem). ~95 s warm
  start = praktické dno. Detaily viz `GPS.md`.
- 2026-10-28 (bod 2 další dávka): tři nové bindingy, všechny dt_binding_check +
  dtbs_check čisté, pushnuto na `plinius`: **mt6685 RTC** child + interrupt-ctrl
  (`8b5797d`), **lvts mt6878-ap/-mcu** (`37500d0`, clocks/resets/nvmem optional),
  **mt6878-gps** connsys GNSS (`3c7d264`). Plný board dtbs_check: **33 → 30**
  warningů. Driver-bindings 14/14 (všechny naše `.c` compatibly mají binding).
  Zbývajících 30 = jen ROZHODOVACÍ/dialogové, potvrzeno živostí na čipu:
  **UFS živé** (`/dev/sdc` 238 GB interní + host0) → VYŘEŠENO per-SoC compatible
  `mediatek,mt6878-ufshci` (binding+driver+dts, commity 8dec57b/83b5f52/653d2f4);
  **USB mtu3/xhci živé** (RNDIS UDC) → VYŘEŠENO `mt6878-mtu3`/`-xhci` s fallbackem
  bez driver patche (4b1c1e6/2215f26); board 33→22→**13**,
  **USB mtu3 živé** (`11201000.usb0` = náš RNDIS UDC, generic `mediatek,mtu3` +
  `mtk-xhci` → chce `mt6878-mtu3`/`-xhci` + nodename usb@/usb0@), watchdog
  (syscon+reboot-mode, žádný mainline precedent), tphy (mt6785-tphy + port graph),
  idle-states (custom názvy + entry-method ≠ psci), mt6363-keys (parent-vs-child
  irq), soc/pinctrl ranges. Upstream gap (ne naše): ti,syscon-reset bez YAML (2).
