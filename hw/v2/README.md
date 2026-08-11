# OpenMicro2

OpenMicro2 is an original wireless macropad design — the second-generation
successor to the retired `openmicro-kbd` example, and **no longer a clone of
any commercial keyboard**. It is built around a SiFli SF32LB52 (BLE 5.3,
Cortex-M33) and adds a display, audio, and battery power:

- a 2.16" 480x480 AMOLED touch module (Huaxia RGB H0216F002AM: CO5300 DDIC
  over LCDC quad-SPI, CST9220 touch over I2C) held OFF-BOARD on a
  3D-printed mount at the assembly's left, its flex tail plugged into an
  OCN OK-F302-31115 0.3mm FPC receptacle on the board's left edge
  (`docs/README.md` records how that connector was identified) — taking
  the 41mm-tall module off the PCB is what lets the board be only 92mm
  tall;
- an on-board control band above the keys: the rotary encoder sitting
  immediately left of the Alps planar joystick on a shared centre line
  (both pushes are scanned as matrix row 3, so neither costs a GPIO);
- a 3 x 5 field of 15 Kailh Choc V2 switches on the 19.05mm grid, one
  reverse-mount SK6812MINI-E per key, plus a 10-LED perimeter ambient ring
  (3 top, 3 bottom, 2 per side) on the top layer, firing upward, EVENLY
  spaced — quarters of the long edges, thirds of the short ones — and
  chained after the key LEDs on the same gated rail, so one data pin
  (PA40) drives all 25;
- an analog MEMS microphone module on a 2-pin connector, running into the
  SF32's dedicated audio ADC (`ADCP`) with its supply served from `MIC_BIAS`
  — zero GPIOs;
- Bluetooth LE via a Johanson 2450AT18B100E chip antenna on the top edge
  between the second and third underglow LEDs (50-ohm feed, tee matching
  slots reserved, ground-clearance zone with open air above it now that
  the display is off-board; off-corner placement still means the matching
  gets characterized on the finished board);
- USB-C for charging and native full-speed USB data; an SGM41562B power-path
  charger and a JST-XH 2-pin battery connector (J4, back side, bottom-left)
  for a 1-cell Li-ion pack — this is what makes the keyboard fully wireless.
  The CELL is a plug-in purchase item, not a BOM line: any 1S LiPo with a
  JST-XH plug fits (e.g. a 503450-class ~1000mAh pouch adhered to the back
  under the key field, clear of the service band and the underglow
  windows). The display's SIBO rail is specified 3.7-4.5V (abs. max 4.6V),
  so it runs from the power-path SYS rail, never from 5V VBUS;
- an SGM2554 load switch that gates the LED rail off in BLE sleep.

41 of the SF32's 45 PA GPIOs are committed (PA01/PA09/PA30/PA37 are free
expansion headroom; PA30 keeps a GPADC channel) — the pin map in
`src/main.cohdl` is a solved packing around the fixed pinmux positions
(LCDC QSPI on PA00/PA02..PA08, MPI2 flash on PA12..PA17, the 32.768kHz
BLE-sleep crystal on PA22/PA23, GPADC channels only on PA28..PA34, USB on
PA35/PA36, I2C1 on PA41/PA42), and every FLEXIBLE signal is additionally
chosen by package geometry: the MCU sits WEST, between the USB entry and
the display bundle, its RF/USB pin edge toward the board's top edge — the
twelve-line display bundle runs ~23mm off the chip's west flank to the
left-edge receptacle, the USB pair exits ~10mm from its ESD array, the
matrix escapes south and east, the encoder takes the top row's east-end
pins beside its knob, and the accepted trade is a ~35mm 50-ohm antenna
feed to the user-placed top-edge antenna slot (~38mm).

The PCB outline is a 115 x 92 mm rounded rectangle; the side margins carry
the display, battery, and debug connectors on the left edge and the
microphone connector on the right, and USB-C sits on the top edge between
the first two underglow LEDs with the antenna between the last two. User-facing controls,
the key field, LEDs, matrix diodes, and the service connectors are fully
placed; back-side electronics are pre-route seeds (unlike `openmicro`, whose
placement was re-derived from routing runs), and bypass capacitors carry
`#[bypass]` attributes instead of coordinates so the auto-placer positions
them from the fanout.

Build it from the repository root:

```sh
cargo run -- check examples/openmicro2
cargo run -- fmt examples/openmicro2 --check
cargo run -- build examples/openmicro2 --emit ipc2581
```

Notes and open obligations for the layout stage:

- the antenna's 6.5 x 4 mm zone must stay copper-free on all layers,
  and the tee matching network's two shunt lands are reserved unpopulated
  beside the fitted 0R series slot (CoHDL has no do-not-populate construct);
- SiFli specifies no fitted matching capacitors for crystals below 12pF CL;
  reserve unpopulated lands;
- the SGM41562B power path limits USB input current, so firmware must cap
  simultaneous LED brightness + display load + charge current — the LED rail
  gate (PA44) is also the budget lever;
- the H0216F002AM module itself is a plug-in assembly (it appears in the BOM
  only as its board-side receptacle); the 3D-printed display mount must
  hold the panel within its flex tail's reach of the left-edge receptacle
  (the module spec does not dimension the free tail length — verify on
  hardware), and all enclosure CAD, keycaps, and firmware are outside this
  example's scope.

Component provenance: `docs/README.md` here (OCN connector series drawing),
the library packages' own `docs/` collections (SF32, display module, charger,
antenna, connectors, flash, passives), and the audited `../openmicro/docs/`
collection for the Alps and Kailh parts.
