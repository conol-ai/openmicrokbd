# Production files

Everything a fab/assembly house needs to build and populate the board. Exported
from the routed board at [`../pcb/openmicro.kicad_pcb`](../pcb/) with KiCad 10.0.4.

| File | What it is |
|---|---|
| `openmicro-gerbers.zip` | Gerbers + Excellon drill, 4-layer — upload this zip as-is |
| `gerbers/` | The same files unzipped, for review and diffing |
| `openmicro-bom.csv` | Assembly BOM — `Manufacturer,Comment,Designator,Footprint`, where **Comment is the MPN** (copy of [`../out/openmicro-bom.csv`](../out/openmicro-bom.csv), emitted by `cohdl build`) |
| `openmicro-smt.csv` | SMT pick-and-place (CPL), JLC/Altium-style columns — mm units, board lower-left origin, Y-up (copy of [`../out/openmicro-smt.csv`](../out/openmicro-smt.csv)) |
| `openmicro-pos-all.csv` | Position file for **every** footprint — same columns as the CPL, nothing excluded (through-hole, mount holes, copper-only features) — for fixtures and inspection |

The CPL deliberately lists **SMD parts only**: the Choc keyswitches (SW3–SW15),
EC11 encoder (SW1), RKJXV joystick (J1), mount holes (H1–H4) and the bare-copper
touch pad (TP1) are through-hole or copper-only features and are hand-populated
or nothing at all. They still appear in the BOM for sourcing. Pick-and-place
rotations follow KiCad orientation; assembly houses apply their own per-part
tape-orientation corrections (JLC shows a placement preview — check polarized
parts there).

## Board status (DRC, KiCad 10.0.4)

Zones refilled; **0 unconnected items**. Remaining flags, all reviewed and
accepted:

- 63× pad-to-pad clearance 0.2000 mm vs the board's 0.2032 mm (8 mil) rule —
  3.2 µm under, and every pair is *within* a single package (the STM32's
  0.5 mm-pitch QFP-48 pads, U3, and the USB-C receptacle's pin grid, J3):
  fixed package geometry, far above JLC's 0.127 mm capability.
- 89× copper near Edge.Cuts — the 8 perimeter underglow LEDs are reverse-mount
  parts whose footprints carry their own board cutout, so their pads border an
  edge by construction; the rest is the edge-mount USB-C (J3).
- Silkscreen warnings (over copper / near edge / overlaps) — cosmetic.

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
cohdl build                       # -> out/openmicro-bom.csv
smt_pos.py pcb/openmicro.kicad_pcb out/openmicro-smt.csv
smt_pos.py --all pcb/openmicro.kicad_pcb fab/openmicro-pos-all.csv
```

The BOM comes from the CoHDL source (`src/`); the CPL and gerbers from the
routed board. `smt_pos.py` currently lives in the CoHDL repository
(`tools/smt_pos.py`) pending the compiler's open-source release.
