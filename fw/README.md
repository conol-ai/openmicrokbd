# OpenMicro firmware

Rust/[embassy](https://embassy.dev) firmware for the OpenMicro macropad
(STM32F072CBT6). Async, no RTOS, no unsafe outside the vendored HAL.

Build stats (release, LTO, `opt-level = "s"`): ~22 KiB flash of 128 KiB,
~5 KiB static RAM of 16 KiB.

## Pin map — where it comes from

The pin assignment is **generated from the board design**, not chosen here:
`../src/openmicro_parts.cohdl` (the `STM32F072CBT6` device block) is the
single source of truth — it is the position-aware GPIO map that made the
board routable. If the `.cohdl` changes, update the table at the top of
`src/main.rs` to match.

| Function | Pins |
|---|---|
| Matrix rows (out, drive-high) | ROW0 `PA9` · ROW1 `PB3` · ROW2 `PB6` · ROW3 `PB5` |
| Matrix cols (in, pull-down) | COL0 `PB8` · COL1 `PB7` · COL2 `PA15` · COL3 `PA10` |
| Rotary encoder | A `PC13` · B `PC14` · push `PC15` |
| Joystick | X `PB1`/ADC_IN9 · Y `PB0`/ADC_IN8 · push `PA8` |
| Touch pad | `PB9` (RC charge-time sensing) |
| RGB data | per-key chain (13× SK6812MINI-E) `PB4` · underglow ring (16×) `PA0` |
| USB FS | DM `PA11` · DP `PA12` |
| SWD (J2) | SWDIO `PA13` · SWCLK `PA14` |

Clocking: HSI48 with CRS sync from USB SOF drives both the core (48 MHz —
the WS2812 bit-bang cycle counts assume it) and the USB peripheral. The
8 MHz HSE crystal on the board is fitted belt-and-braces but not required.

## What it does

A composite USB HID device (VID `0x1209` pid.codes, keyboard + consumer
control):

- **13 keys → F13…F24** — 1 kHz matrix scan, 5 ms debounce, COL2ROW diodes.
  The two switches under the 2U keycap (sw10/sw11) both send F23.
- **Encoder → volume** up/down, push → mute.
- **Touch pad → play/pause.**
- **Joystick → arrow keys** (ADC thresholds), push → Enter.
- **LEDs**: pressed keys light white over an idle rainbow; the underglow
  ring rotates hue. Brightness is capped in `ws2812.rs` (`scaled(n/64)`)
  to keep all 29 LEDs inside the 500 mA VBUS budget.

## Building

```sh
rustup target add thumbv6m-none-eabi
cargo build --release
```

The workspace/CI at the repo root does not build this crate (it needs the
thumb target); it lives here as part of the example board's deliverables.

## Flashing and field updates

Two deliberate paths — the board has **no BOOT0 button** (BOOT0 is strapped
low through `rboot`), so bootloader entry in the field is software-only:

**Development / recovery: SWD (J2 header)** — with a probe (ST-Link,
CMSIS-DAP, …) attached to J2:

```sh
cargo install probe-rs-tools
cargo run --release        # runner = probe-rs run --chip STM32F072CBTx
```

**Field updates: app-triggered DFU, no probe, no buttons.** The firmware
exposes a vendor HID interface (usage page `0xFF60`, 32-byte reports, no
report IDs) that the host updater app drives:

| OUT report | Effect |
|---|---|
| `[0x01, …]` | Replies `[0x01, len, "0.1.0"…]` — running firmware version |
| `[0x02, 'D','F','U','!']` | Acks `[0x02, 0x01]`, then reboots into the ROM DFU bootloader |

On the DFU command the firmware stamps a magic word in noinit RAM and
resets; early boot (before any peripheral init) sees it and jumps into
system memory (`0x1FFF_C800`, AN2606). The chip re-enumerates on the same
USB-C port as ST DFU (`0483:df11`) and any standard DFU tool finishes the
job:

```sh
cargo install cargo-binutils && rustup component add llvm-tools
cargo objcopy --release -- -O binary openmicro.bin
dfu-util -a 0 -s 0x08000000:leave -D openmicro.bin   # :leave boots the new app
```

The updater app can decide *whether* to update without opening the device:
`bcdDevice` in the USB descriptor carries the semver from `Cargo.toml`
(`0.1.0` → `0x0110`-style encoding, see `version_bcd`). Minimal host flow
(Python + `hidapi` + `dfu-util`):

```python
import hid
dev = next(d for d in hid.enumerate(0x1209, 0x0001) if d["usage_page"] == 0xFF60)
h = hid.device(); h.open_path(dev["path"])
h.write(bytes([0x00, 0x02, ord('D'), ord('F'), ord('U'), ord('!')]))  # leading 0x00 = report ID
# device drops off, re-enumerates as 0483:df11 -> run dfu-util as above
```

Failure model, by design: if power is lost mid-download the app is gone and
the device does not auto-enter DFU (BOOT0 is low) — recovery is the SWD
port. The update window is a few seconds; the trade keeps the boot path
trivial (no resident bootloader to maintain).
