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
| `mediatek,mt6363` | nový (v portu) | `mfd/mediatek,mt6363.yaml` | ✅ |
| `mediatek,mt6363-keys` | rozšířit | `input/mediatek,pmic-keys.yaml` | 🟡 |
| `mediatek,mt6363-regulator` | rozšířit/nový | `regulator/mediatek,mt6363-regulator.yaml` | ❌ |
| `mediatek,mt6369-regulator` | nový | `regulator/mediatek,mt6369-regulator.yaml` | ❌ |
| `mediatek,mt6685` | nový | `mfd/mediatek,mt6685.yaml` | ❌ |
| `mediatek,mt6878-spmi` | rozšířit | `spmi/mtk,spmi-mtk-pmif.yaml` | 🟡 |
| `mediatek,mt6878-power-controller` | rozšířit | `power/mediatek,power-controller.yaml` | 🟡 |
| `mediatek,mt6878-infracfg-ao` | rozšířit | `…/mediatek,infracfg.yaml` (syscon) | 🟡 |
| `mediatek,mt6878-connsys` | nový | `soc/mediatek/mediatek,mt6878-connsys.yaml` | ❌ |
| `mediatek,mt6878-wifi` | rozšířit | `net/wireless/mediatek,mt76.yaml` | 🟡 |
| `mediatek,mt6878-bluetooth` | nový (vzor: mt7622-bluetooth) | `net/bluetooth/mediatek,mt6878-bluetooth.yaml` | ✅ |
| `mediatek,mt6878-ccci` | nový | `soc/mediatek/mediatek,mt6878-ccci.yaml` | ❌ |
| `mediatek,mt6878-disp-refresh` | nový | `soc/mediatek/mediatek,mt6878-disp-refresh.yaml` | ❌ |
| `volla,plinius` + vendor `volla` | rozšířit | `arm/mediatek.yaml` + `vendor-prefixes.yaml` | 🟡 |

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
  Další krok: série F/G/H bindings — connsys (`mediatek,mt6878-connsys`,
  nový YAML pod soc/mediatek), pak WiFi (rozšířit mt76.yaml).
