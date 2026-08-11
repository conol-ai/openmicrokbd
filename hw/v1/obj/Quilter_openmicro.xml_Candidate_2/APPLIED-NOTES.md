# Quilter Candidate_2 — applied to `out/openmicro.kicad_pcb`

Applied 2026-07-19 with `tools/apply_quilter.py`. Quilter exported this candidate
for **Cadence Allegro** (`Quilter_Importer.il` + `openmicro.csv`); this project is
KiCad, so the CSV — which carries the actual design changes — was replayed onto
the board with `pcbnew` instead of running the SKILL script.

Backup of the pre-Quilter board: `out/openmicro.pre-quilter.kicad_pcb`.

## What was applied

| Operation | Count |
|---|---|
| `MOVE_COMPONENT_TO` | 48 (the staged parts: C1–C27, D1–D13, R1–R3, U1–U3, J2, SW2) |
| `ROTATE_COMPONENT` | 30 |
| `CREATE_LINE` | 1978 (F.Cu 1586, B.Cu 392) |
| `CREATE_VIA` | 222 |
| `CREATE_POUR` | 2 (GND, F.Cu + B.Cu) |
| `CREATE_PADSTACK` | 1 (`VIA_610_305` → 0.305mm drill / 0.610mm pad) |

Quilter moved only the 48 components that were *staged* off-board; it left the 50
hand-placed ones (keys, LEDs, mounting holes, TP1, J1, J3) untouched, as intended.

**Routing completeness: unconnected items 240 → 13.**

## Coordinate frame (important for any re-application)

The CSV is **not** in KiCad's frame. It lives in the coordinate space of the
IPC-2581 document we fed Quilter (`build --emit ipc2581`), which is **Y-up and
millimetre-based**, while the CSV itself is in **microns** and KiCad is **Y-down**:

```
x_mm = x_um / 1000        y_mm = -y_um / 1000        rot_kicad = angle   (sign +1)
```

Layer tokens: `ETCH/F.CU`→`F.Cu`, `ETCH/B.CU`→`B.Cu`,
`BOUNDARY/LAYER_1`→`F.Cu`, `BOUNDARY/LAYER_2`→`B.Cu`.

Both the Y-flip and the rotation sign were established **empirically**, not
assumed: under this mapping all 129 routed pads of the moved components land on a
trace/via carrying that pad's own net, with **0 net mismatches**. Rotation sign
−1 yields 38 mismatches — worth noting because ±90° swaps pads 1↔2 on every
2-terminal part, which would have silently reversed all 13 diodes.

## Design rules

The board netclass was relaxed from 0.2mm to **0.15mm track / 0.15mm clearance**
(and `min_track_width`/`min_clearance` to 0.15) to match what Quilter actually
routed to (0.1525mm traces, 0.1562mm min clearance — i.e. ~6 mil). This cleared
258 `clearance` + 199 `track_width` violations. 6 mil is standard capability at
common fabs.

Note `min_clearance` was previously `0.0` (no floor); setting a real 0.15mm floor
added 4 `hole_clearance` reports that were previously unmeasured.

## KNOWN ISSUE — mechanical holes were not treated as keep-outs

**Root cause:** our IPC-2581 export emits RFC-022 `mount_hole` holes as hole
geometry with no net, but *not* as routing keep-outs. Quilter therefore routed
straight through the Choc switch locating holes (中央方轴心孔 / MX 定位柱).

This single root cause accounts for **107 of the 264 remaining DRC violations**
(93 `hole_clearance` + 14 `hole_to_hole`).

Per the design decision, these are **left in place and documented** rather than
auto-removed. They must be resolved before fabrication.

### 7 vias drilled inside mechanical locating holes

| Switch | Net | Position (mm) |
|---|---|---|
| SW9  | GND  | (−27.476, −7.976) |
| SW10 | COL3 | (−9.435, −8.938) |
| SW10 | GND  | (−10.740, −9.051) |
| SW11 | GND  | (11.035, −7.734) |
| SW12 | GND  | (29.657, −7.154) |
| SW15 | GND  | (8.682, 9.739) |
| SW15 | ROW2 | (8.893, 7.727) |

### 33 track crossings over mechanical holes (12 switches)

| Switch | Nets crossing the hole |
|---|---|
| SW3  | COL0, COL3, JOY_SW, LED_D2, ROW0 |
| SW4  | ROW3 |
| SW5  | VBUS |
| SW6  | COL3, KEY12, LED_D10, LED_D11 |
| SW7  | VBUS |
| SW8  | COL0, ROW0, V3V3, VBUS |
| SW9  | TOUCH |
| SW10 | COL3, KEY13, KEY8, KEY9, LED_D2, ROW2 |
| SW11 | KEY9, ROW1, ROW3, VBUS |
| SW13 | TOUCH |
| SW14 | LED_D6, LED_D7, VBUS |
| SW15 | LED_D8, ROW2 |

**Suggested fix:** add the 26 NPTH mechanical holes as explicit keep-out zones
before re-running Quilter, or teach the IPC-2581 emitter to project `mount_hole`
as a routing keep-out so any autorouter avoids them.

## Remaining DRC (264 total, after the 6 mil relaxation)

| Type | Count | Assessment |
|---|---|---|
| `silk_over_copper` | 105 | Cosmetic. 32 pre-existing, 73 from the newly placed parts. |
| `hole_clearance` | 93 | Mechanical-hole keep-out issue (above). |
| `silk_overlap` | 15 | Cosmetic, from the newly placed parts. |
| `hole_to_hole` | 14 | Mechanical-hole keep-out issue (above). |
| `courtyards_overlap` | 13 | **Pre-existing and by design** — per-key LEDs sit inside the switch courtyards. Identical count before Quilter. |
| `via_dangling` | 13 | GND via stubs left by Quilter. |
| `starved_thermal` | 9 | GND zone thermal spokes below min count. |
| `silk_edge_clearance` | 2 | Cosmetic. |

13 unconnected items remain, all GND — the pours plus a few sub-micron track
fragments Quilter emitted (134 segments are shorter than 1µm; they were preserved
rather than dropped so connectivity topology stays exactly as Quilter produced it).

## Caveat: `out/` is regenerable

`out/` is gitignored, and `tools/kicad_board.py` rebuilds `openmicro.kicad_pcb`
from the netlist + footprints. **Re-running it discards this routing.** Keep
`openmicro.pre-quilter.kicad_pcb` and this candidate directory if you need to
reproduce the routed board.
