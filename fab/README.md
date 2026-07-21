# Production files

Everything a fab/assembly house needs to build and populate the board. Exported
from the routed board at [`../pcb/openmicro.kicad_pcb`](../pcb/) with KiCad 10.0.4.

| File | What it is |
|---|---|
| `openmicro-gerbers.zip` | Gerbers + Excellon drill, 4-layer — upload this zip as-is |
| `gerbers/` | The same files unzipped, for review and diffing |
| `openmicro-bom.csv` | Assembly BOM — `Manufacturer,Comment,Designator,Footprint`, where **Comment is the MPN** (copy of [`../out/openmicro-bom.csv`](../out/openmicro-bom.csv), emitted by `cohdl build`) |
| `openmicro-smt.csv` | SMT pick-and-place (CPL), JLC/Altium-style columns — mm units, board lower-left origin, Y-up (copy of [`../out/openmicro-smt.csv`](../out/openmicro-smt.csv)) |

The CPL deliberately lists **SMD parts only**: the Choc keyswitches (SW3–SW15),
EC11 encoder (SW1), SWD header (J2), mount holes (H1–H4) and the bare-copper
touch pad (TP1) are through-hole or copper-only features and are hand-populated
or nothing at all. They still appear in the BOM for sourcing. Pick-and-place
rotations follow KiCad orientation; assembly houses apply their own per-part
tape-orientation corrections (JLC shows a placement preview — check polarized
parts there).

## Board status (DRC, KiCad 10.0.4)

Zones refilled; **0 unconnected items, 0 copper-to-copper clearance
violations**. Remaining flags, all reviewed and accepted:

- 199× via annular width 0.0977 mm vs the board's 0.1 mm rule — 2.3 µm under
  our own conservative rule, comfortably above JLC's 0.075 mm capability.
- Tracks 0.17–0.5 mm from the Choc switches' locating holes (NPTH) — tight but
  standard for Choc boards; JLC's copper-to-NPTH minimum is 0.2 mm and only a
  handful of segments sit slightly inside that with drill-tolerance risk only.
- USB-C shield pads (J3) on the board edge — edge-mount connector, by design.
- 14× courtyard overlaps — the per-key LED/diode sit under the switches, by design.
- Silkscreen-over-mask warnings — cosmetic.

## Fab parameters

95 × 95 mm, **4 layers** (F.Cu signal / In1.Cu GND plane / In2.Cu GND plane /
B.Cu signal), 1.6 mm FR-4, 1 oz copper, dielectric 0.2 / 1.03 / 0.2 mm
(impedance-controlled — the USB differential pairs target 100 Ω differential /
50 Ω single-ended per the design constraints in
[`../out/differential_pairs.csv`](../out/differential_pairs.csv); on JLCPCB
pick the JLC04161H-7628 standard stackup), min track/space used 0.15/0.15 mm,
vias 0.495/0.3 mm, surface finish your choice (ENIG recommended for the
capacitive touch pad TP1).

## Regenerating

```sh
# Gerbers + drill (from the repo root)
kicad-cli pcb export gerbers -o fab/gerbers/ \
  --layers F.Cu,In1.Cu,In2.Cu,B.Cu,F.Paste,B.Paste,F.Silkscreen,B.Silkscreen,F.Mask,B.Mask,Edge.Cuts \
  pcb/openmicro.kicad_pcb
kicad-cli pcb export drill -o fab/gerbers/ --excellon-separate-th pcb/openmicro.kicad_pcb

# BOM (CoHDL compiler) and CPL (smt_pos.py, run with KiCad's bundled python)
cohdl build .                     # -> out/openmicro-bom.csv
smt_pos.py pcb/openmicro.kicad_pcb out/openmicro-smt.csv
```

The BOM comes from the CoHDL source (`src/`); the CPL and gerbers from the
routed board. `smt_pos.py` currently lives in the CoHDL repository
(`tools/smt_pos.py`) pending the compiler's open-source release.
