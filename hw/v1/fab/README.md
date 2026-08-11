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

Zones refilled; **0 unconnected items**, and no schematic-parity errors.
Remaining flags, all reviewed and accepted:

- 236× clearance + 199× track width. The board file keeps a conservative
  0.2032 mm (8 mil) minimum clearance/width rule, while the autorouter works
  to a 0.15 mm constraint class: every flagged track is 0.153 mm wide and the
  smallest actual clearance is 0.154 mm. Included in that count are 63
  pad-to-pad pairs *within* a single package — the STM32's 0.5 mm-pitch
  QFP-48 (U3) and the USB-C receptacle's pin grid (J3) — which are fixed
  package geometry. Everything here is far above JLC's 0.127 mm capability.
- 89× copper near Edge.Cuts: 84 are the reverse-mount SK6812 LEDs, whose
  footprints carry their own board cutout, so their pads border that opening
  by construction. The remaining five are the edge-mount USB-C shield tabs
  (J3, flush with the board edge by design) and the three odd-row pads of the
  SWD socket J2, which end 0.18 mm from the top edge — the socket sits at
  y = 43.0 mm on a board that ends at 47.5 mm. Accepted: it is a hand-soldered
  through-board socket land, and 0.18 mm clears a routed outline comfortably.
- 6× hole clearance — tracks passing 0.16–0.18 mm from the NPTH locating
  holes of the keyswitches and USB-C. These take plastic bosses, not pins;
  there is no copper barrel to short to.
- 74× silkscreen warnings (over copper / near edge / overlaps) — cosmetic.

## Fab parameters

95 × 95 mm, **4 layers** (F.Cu signal / In1.Cu GND plane / In2.Cu GND plane /
B.Cu signal), 1.6 mm FR-4, 1 oz copper, dielectric 0.2 / 1.03 / 0.2 mm
(impedance-controlled — the USB differential pairs target 100 Ω differential /
50 Ω single-ended per the design constraints in
[`../out/differential_pairs.csv`](../out/differential_pairs.csv); on JLCPCB
pick the JLC04161H-7628 standard stackup), min track/space used
0.153/0.154 mm, vias 0.45 mm pad / 0.305 mm drill (216 of them), surface
finish your choice (ENIG recommended for the capacitive touch pad TP1).

## Regenerating

```sh
# Gerbers + drill (from hw/v1/)
kicad-cli pcb export gerbers -o fab/gerbers/ \
  --layers F.Cu,In1.Cu,In2.Cu,B.Cu,F.Paste,B.Paste,F.Silkscreen,B.Silkscreen,F.Mask,B.Mask,Edge.Cuts \
  pcb/openmicro.kicad_pcb
kicad-cli pcb export drill -o fab/gerbers/ --excellon-separate-th pcb/openmicro.kicad_pcb

# BOM (CoHDL compiler) and CPL (smt_pos.py, run with KiCad's bundled python)
cohdl build .                     # -> out/openmicro-bom.csv
smt_pos.py pcb/openmicro.kicad_pcb out/openmicro-smt.csv
smt_pos.py --all pcb/openmicro.kicad_pcb fab/openmicro-pos-all.csv
```

The BOM comes from the CoHDL source (`src/`); the CPL and gerbers from the
routed board. `smt_pos.py` currently lives in the CoHDL repository
(`tools/smt_pos.py`) pending the compiler's open-source release.

## Firmware programming

Program `openmicro-fw-<version>.hex` from the [GitHub release
assets](../RELEASING.md) over SWD at J2. The hex is Intel HEX with the
0x08000000 load address embedded — byte-identical to the released `.bin`, so
any STM32-capable programmer (STM32CubeProgrammer, J-Flash, gang programmers)
places it correctly with no address entry. Verify the file against the
release's `SHA256SUMS` before loading it into the fixture.

Full-chip erase + program + verify is the correct cycle on a fresh board. The
last 2 KiB flash page (0x0801F800) holds user settings and must simply be
**left erased** — the firmware detects the blank page and boots with factory
defaults. Do not program anything there.

J2 is a 2×3 2.54 mm socket (P1/P2 GND, P3 SWCLK, P5 SWDIO; P4/P6 are the
serial console). It carries **no 3.3 V pin**: power the board through USB-C
while programming, and configure the programmer accordingly if it expects a
target-voltage sense line.

Post-programming check: the board enumerates over USB as `1209:0001`
(OpenMicro). `scripts/bin2hex.py` regenerates the hex from a released `.bin`
if needed.
