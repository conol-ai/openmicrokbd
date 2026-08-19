# OpenMicro firmware

Rust/[embassy](https://embassy.dev) firmware for the OpenMicro macropad
(STM32F072CBT6). Async, no RTOS, no unsafe outside the vendored HAL.

Build stats (release, LTO, `opt-level = "s"`): 37.9 KiB flash of 128 KiB with
bring-up logging compiled in, 25.8 KiB with `DEFMT_LOG=off`; ~5 KiB static RAM
of 16 KiB. The last 2 KiB flash page is reserved for the saved keymap.

## Pin map — where it comes from

The pin assignment is **generated from the board design**, not chosen here:
`../src/openmicro_parts.cohdl` (the `STM32F072CBT6` device block) is the
single source of truth — it is the position-aware GPIO map that made the
board routable. If the `.cohdl` changes, update the table at the top of
`src/main.rs` to match.

| Function | Pins |
|---|---|
| Matrix rows (out, drive-high) | ROW0 `PA9` · ROW1 `PA10` · ROW2 `PB3` · ROW3 `PB8` |
| Matrix cols (in, pull-down) | COL0 `PB4` · COL1 `PB5` · COL2 `PC14` · COL3 `PC13` |
| Rotary encoder | A `PB12` · B `PB13` · push `PB15` |
| Joystick | X `PB1`/ADC_IN9 · Y `PA0`/ADC_IN0 · push `PA15` |
| Touch pad | `PB9` (RC charge-time sensing) |
| RGB data | per-key chain (13× SK6812MINI-E) `PA8` · underglow ring (8×) `PB14` |
| USB FS | DM `PA11` · DP `PA12` |
| SWD (J2) | SWDIO `PA13` · SWCLK `PA14` |

Clocking: HSI48 with CRS sync from USB SOF drives both the core (48 MHz —
the WS2812 bit-bang cycle counts assume it) and the USB peripheral. The
8 MHz HSE crystal on the board is fitted belt-and-braces but not required.

### Prototype boards (`--features proto`)

Boards fabbed before the 2026-07-28 GPIO re-derivation use the older pin map
and a 16-LED underglow ring. Build with the `proto` feature to target them —
default builds are for the current (v23) board, and the two are **not**
interchangeable:

| Function | Pins (prototype) |
|---|---|
| Matrix rows (out, drive-high) | ROW0 `PA9` · ROW1 `PB3` · ROW2 `PB6` · ROW3 `PB5` |
| Matrix cols (in, pull-down) | COL0 `PB8` · COL1 `PB7` · COL2 `PA15` · COL3 `PA10` |
| Rotary encoder | A `PC13` · B `PC14` · push `PC15` |
| Joystick | X `PB1`/ADC_IN9 · Y `PB0`/ADC_IN8 · push `PA8` |
| RGB data | per-key chain (13×) `PB4` · underglow ring (16×) `PA0` |

Touch, USB, and SWD are identical on both revisions.

```sh
cargo build --release --features proto            # from fw/, or:
FW_FEATURES=proto scripts/build-firmware.sh dist  # -> dist/openmicro-fw-<version>-proto.bin
```

## What it does

A composite USB HID device (VID `0x1209` pid.codes, keyboard + consumer
control), with **every input's emitted code configurable and stored on the
device** (`keymap.rs`):

- **24 keymap slots** — the 13 keys (all independent positions, including the
  pair under the 2U keycap), encoder CW/CCW/press, joystick up/down/left/
  right/press, touch tap (+ two reserved swipe slots for a future multi-zone
  pad). Each slot emits a keyboard usage (with a modifier mask — `Shift+F13`
  style) or a consumer usage, or nothing. The companion app writes slots over
  the vendor HID interface; SAVE persists them to the last flash page, which
  neither DFU updates nor probe-rs flashing touches — so the keymap follows
  the pad across machines and firmware updates.
- **Factory defaults chosen to be interceptable on every OS**: keys emit
  F13–F20 and Shift+F13…F17 (macOS has no virtual keycodes for F21–F24, so
  those never appear as defaults), encoder volume/mute, joystick arrows and
  Enter, touch play/pause.
- **13 keys** — 1 kHz matrix scan, 5 ms debounce, COL2ROW diodes. Held slots
  feed one shared 6KRO report builder, so a joystick move can no longer drop
  a held key from the host's view.
- **Encoder** — A/B are decoded from a full
  quadrature transition table on their own EXTI-driven task: polling them
  from the 1 kHz scan aliased away transitions during a fast spin, and the
  WS2812 critical section (below) blanks interrupts long enough to lose
  states outright. EXTI latches its pending bit, so nothing is dropped.
- **Touch pad.** The RC rise on `PB9` is only ~20 CPU cycles, so
  the sense loop configures PUPDR/ODR once and flips *only* MODER via raw
  register writes (`unstable-pac`) — going through `Flex::set_as_input()` per
  cycle costs longer than the rise being measured, and reads a constant zero.
  Each tick sums 64 charge cycles for SNR: on hardware that puts an untouched
  pad at exactly 192 with no jitter, a finger at 242–1015, and the trigger at
  25% over a self-calibrating baseline.
- **Joystick** — 50 Hz ADC poll with an app-tunable deflection threshold
  (`SET_ANALOG`, persisted with the keymap). Three modes (`SET_JOYMODE`,
  persisted): **keys** holds the direction slots, **mouse** moves a dedicated
  HID pointer proportionally (push = left click), and **grade** is the mouse
  with the speed applied squared (sub-pixel fine at 1, brisk at 10) and the
  left button auto-held while deflected — park the pointer over a DaVinci
  Resolve colour wheel and the stick grabs and drags it like a panel
  trackball, releasing at centre (with hysteresis, so dead-zone jitter can't
  machine-gun clicks).
- **LEDs**: pressed keys light white over an idle rainbow; the underglow
  ring rotates hue. Brightness is capped in `ws2812.rs` (`scaled(n/64)`)
  to keep all 21 LEDs inside the 500 mA VBUS budget.

## Building

```sh
rustup target add thumbv6m-none-eabi
cargo build --release
```

The workspace/CI at the repo root does not build this crate (it needs the
thumb target); it lives here as part of the example board's deliverables.

## Logging (bring-up)

`defmt` over RTT, carried by the same SWD probe used for flashing, with panics
printed on the channel via `panic-probe`. `cargo run --release` flashes and
then streams:

```
[INFO ] OpenMicro fw v0.1.0: clocks up (HSI48 -> 48 MHz core, CRS synced from USB SOF)
[INFO ] matrix r2 c1 DOWN kc=0x6f
[INFO ] encoder CW -> vol+
[DEBUG] touch charge t=48 baseline=48
```

The level is set by `DEFMT_LOG` in `.cargo/config.toml`, defaulting to
`info,openmicro_fw=debug` — this crate's per-tick diagnostics (joystick raw
counts, touch charge time, executor heartbeat) at `debug`, embassy at `info`.
Filtering is compile-time, so **`DEFMT_LOG=off` strips logging out entirely**
and is what production builds should use; it also drops the flash footprint
from 34.5 KiB back to 23.5 KiB.

## Flashing and field updates

Two deliberate paths — the board has **no BOOT0 button** (BOOT0 is strapped
low through `rboot`), so bootloader entry in the field is software-only:

**Development / recovery: SWD (J2 header)** — with a probe (ST-Link,
CMSIS-DAP, …) attached to J2:

```sh
cargo install probe-rs-tools
cargo run --release        # runner = probe-rs run --chip STM32F072CBTx
```

**The vendor HID interface** (usage page `0xFF60`, 32-byte reports, no
report IDs) carries the whole app protocol — replies echo the command byte;
any IN report starting `0x80` is an unsolicited input event, not a reply:

| OUT report | Effect |
|---|---|
| `[0x01, …]` | Replies `[0x01, len, "0.2.1"…]` — running firmware version |
| `[0x02, 'D','F','U','!']` | Acks `[0x02, 0x01]`, then reboots into the ROM DFU bootloader |
| `[0x03, page]` | Replies `[0x03, page, count, count×4 slot bytes]` — read keymap (4 pages of ≤7 slots; slot = kind, mods, code LE; kind 0 none / 1 keyboard / 2 consumer) |
| `[0x04, page, count, slots…]` | Acks `[0x04, ok]` — write keymap page to RAM, live immediately |
| `[0x05, 'S','A','V','E']` | Acks `[0x05, ok]` — persist keymap + analog to the last flash page |
| `[0x06, 'R','S','T','!']` | Acks `[0x06, ok]` — factory defaults, saved config wiped |
| `[0x07]` | Replies `[0x07, thr_lo, thr_hi]` — joystick threshold (u16 LE) |
| `[0x08, thr_lo, thr_hi]` | Acks `[0x08, 0x01]` — set threshold in RAM (SAVE persists) |
| `[0x09]` | Replies `[0x09, mode, speed]` — joystick mode (0 keys / 1 mouse / 2 grade) + pointer speed 1–10 |
| `[0x0A, mode, speed]` | Acks `[0x0A, 0x01]` — set joystick mode/speed in RAM (SAVE persists) |
| `[0x0B]` | Replies `[0x0B, brightness]` — LED brightness 0–255 |
| `[0x0C, brightness]` | Acks `[0x0C, 0x01]` — set brightness in RAM, applied within one LED frame (SAVE persists) |
| `[0x0D]` | Replies `[0x0D, kmode,kr,kg,kb, umode,ur,ug,ub]` — per-chain LED pattern (0 rainbow, 1 solid RGB) |
| `[0x0E, kmode,kr,kg,kb, umode,ur,ug,ub]` | Acks `[0x0E, 0x01]` — set patterns in RAM (SAVE persists) |
| `[0x0F, index, enabled, r, g, b]` | Acks `[0x0F, 0x01]` — set or clear one key LED override in RAM (never persisted) |

Event reports (`[0x80, src, a, b]`, best-effort, dropped when no host reads):
src 0 = key (a = position 0–12, b = pressed), 1 = encoder rotate (a = 1 CW),
2 = encoder button, 3 = joystick (a = dir 0 up / 1 down / 2 left / 3 right /
4 press, b = active), 4 = touch tap. These give the companion app live press
feedback — and a hardware test — with no OS input-monitoring permission.

**Field updates: app-triggered DFU, no probe, no buttons.**

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
