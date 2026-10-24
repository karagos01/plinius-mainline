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
| `mediatek,mt6685` | nový | `mfd/mediatek,mt6685.yaml` | ✅ |
| `mediatek,mt6878-spmi` | rozšířeno (+ interrupty) | `spmi/mtk,spmi-mtk-pmif.yaml` | ✅ |
| `mediatek,mt6878-power-controller` | rozšířeno | `power/mediatek,power-controller.yaml` | ✅ |
| `mediatek,mt6878-infracfg-ao` | rozšířit | `clock/mediatek,infracfg.yaml` (syscon) | ❌ (viz syscon rodina) |
| `mediatek,mt6878-connsys` | nový | `soc/mediatek/mediatek,mt6878-connsys.yaml` | ✅ |
| `mediatek,mt6878-wifi` | rozšířeno | `net/wireless/mediatek,mt76.yaml` | ✅ |
| `mediatek,mt6878-bluetooth` | nový | `net/bluetooth/mediatek,mt6878-bluetooth.yaml` | ✅ |
| `mediatek,mt6878-ccci` | nový | `soc/mediatek/mediatek,mt6878-ccci.yaml` | ✅ |
| `mediatek,mt6878-disp-refresh` | nový | `soc/mediatek/mediatek,mt6878-disp-refresh.yaml` | ✅ |
| `volla,plinius` + vendor `volla` | rozšířeno (+ nothing,tetris) | `arm/mediatek.yaml` + `vendor-prefixes.yaml` | ✅ |

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
- **UFS** `ufshci@112b0000` (`mediatek,mt8183-ufshci`): `clocks`/`clock-names`
  too long (13 vs schema), chybí `vcc-supply`, `resets`/`reset-names` navíc.
  Buď per-SoC compatible, nebo rozšířit mt8183-ufshci binding.
- **USB** `xhci0@...` (`mediatek,mtk-xhci`) a `usb0@...` (`mediatek,mtu3`):
  generické compatibly nejsou v enumu + nodename/`clock-names` neodpovídá.
  Potřebují per-SoC compatible (`mediatek,mt6878-xhci`/`-mtu3`) nebo doplnění.
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

- **`mediatek,mt6878-pinctrl` (12)** — NOVÝ compatible, chybí binding
  (checkpatch ho nechytil, je ze shallow baseline). Velký: pinmux subnody.
  Vzor: `pinctrl/mediatek,mt6893-pinctrl.yaml` ap. **Priorita.**
- **`mediatek,mt6878-i2c` (3)** — NOVÝ compatible, rozšířit
  `i2c/mediatek,mt65xx-i2c.yaml` (nebo přidat). **Priorita.**
- **`mediatek,mt6878-hwv` + spm `syscon@1c001000`** — pure syscon, přidat do
  `mfd/syscon.yaml` (jako infracfg-ao).
- **ufscfg-ao/pdn** (`syscon@112b8000/112bb000`, simple-mfd + `ti,syscon-reset`
  reset-controller child, #clock-cells) — nový clock+reset binding; child
  `ti,syscon-reset` taky nematchuje (reset/ti-syscon-reset.txt je starý .txt).
- **UFS**: `ufshci@112b0000` (mt8183-ufshci: clocks/clock-names too long,
  chybí vcc-supply, resets navíc) + `ufsphy@112a0000` (nodename). Per-SoC
  compatible nebo rozšířit mt8183 binding.
- **USB**: `usb0@…` (mtu3) + `xhci0@…` (mtk-xhci) — generické compatibly,
  nodename `^usb(@.*)?`, clock-names. Per-SoC compatible.
- **tphy** `t-phy@11f40000` (mt6785-tphy) + `mediatek,software-role-switch`
  missing type v tphy.yaml.
- **watchdog** (mt6878-wdt): 3-compatible (+ "syscon") a `reboot-mode` child
  (syscon-reboot-mode) — chce úpravu mtk-wdt.yaml (driver+binding dialog).
- **mt6363.yaml**: viselý mt6373-regulator ref + adc příklad.
- drobné (1 chyba): `idle-states`, `mmc`, `keys`, `framebuffer`, `soc@0` —
  pravděpodobně example/okolní, prověřit jednotlivě.

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
  nechytil (baseline). Přesný zbytek viz backlog sekce. Další na řadě:
  pinctrl binding, pak i2c, pak hwv/spm do syscon.yaml.
