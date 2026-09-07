# OpenMicro firmware

Rust/[embassy](https://embassy.dev) firmware for the OpenMicro macropad
(STM32F072CBT6). Async, no RTOS, no unsafe outside the vendored HAL.

Build stats (fw 0.10.0, release, LTO, `opt-level = "s"`): 68.1 KiB flash
with `DEFMT_LOG=off` (the production build), 83.3 KiB with the default
bring-up logging (`warn,openmicro_fw=info`) compiled in — of the 84 KiB
application slot behind the resident bootloader (see [Flash
map](#flash-map-header-and-boot-handoff)); ~9.6 KiB static RAM of 16 KiB.
The saved keymap and the Codex file slots live in their own pages above the
image.

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
  neither firmware updates nor probe-rs flashing touches — so the keymap
  follows the pad across machines and firmware updates.
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
- **Codex Micro compat mode** (opt-in, off by default, fw 0.8.0+,
  `src/codex/`): the pad can boot with the USB identity of OpenAI's Codex
  Micro plus a fifth HID interface speaking ChatGPT Desktop's device
  protocol, so the desktop app drives it natively — see
  [Codex Micro compat mode](#codex-micro-compat-mode).

## Codex Micro compat mode

Off by default. When switched on, the pad boots as a **Codex Micro** — VID
`0x303A` / PID `0x8360`, manufacturer "Work Louder", product "Codex Micro" —
and adds a vendor-defined HID interface (usage page `0xFF00`, report ID 6,
64-byte reports) carrying the JSON-RPC-style protocol ChatGPT Desktop uses
to talk to that device. The desktop app then treats the pad as the real
thing: keys, dial and stick arrive as Codex Micro controls, and the app
pushes its six agent status lights and its lighting configuration back to
the LEDs. **Confirmed 2026-09-02 with Codex desktop 26.825 on macOS**: the
app auto-detects the pad over USB the moment it enumerates (no pairing
step), and runs its `device.status` / `v.oai.thstatus` / `v.oai.rgbcfg`
traffic against this firmware without errors. The app opens the interface
**exclusively**, so any other tool holding it (the probe scripts, the
reference Swift probes) makes the app's connection fail with "exclusive
access and device already open" until that tool exits — quit one before
running the other. The normal keymap is bypassed in this mode; the OpenMicro vendor
interface (`0xFF60`) stays, so the companion app, firmware updates and the
mode switch keep working.

**Switching.** Hold a key while plugging the pad in:

| Held at power-up | Mode |
|---|---|
| The **second** key of the second row (KEY 04 in the app; slot 3, matrix row 1 / col 1) | Codex Micro compat |
| The **first** key of the second row (KEY 03 in the app; slot 2, matrix row 1 / col 0) | OpenMicro (the default) |

The choice is saved with the keymap (it takes the reserved byte of the v4
blob, so older firmware ignores it and a downgrade falls back to OpenMicro
mode — until that older firmware SAVEs, re-flashing 0.8.0+ restores the
saved mode) and applies to every later boot. The underglow shows the mode for a
second at power-up — amber for OpenMicro, white for Codex — blinking when a
chord has just changed it. The app's Settings sheet has the same switch
(`GET_MODE` / `SET_MODE` below); a change resets the pad so it re-enumerates.

**Mapping.** The official key numbering runs in the same reading order as
ours, so it is position for position:

| Pad | Codex Micro | Wire id |
|---|---|---|
| keys p0–p5 (top row + second row) | Agent Keys 1–6, with the six status lights | `AG00`–`AG05` (`ag` 0–5) |
| p6 / p7 / p8 / p9 | Command Keys (defaults Fast / Approve / Decline / Fork) | `ACT06`–`ACT09` |
| p10 / p11 | the two switches under the wide Mic key (push-to-talk if the host assigns it) | `ACT10` / `ACT11` — `ACT11` is inferred from the numbering; only `ACT10` has been observed |
| p12 | Send | `ACT12` |
| encoder turn / press | dial step / dial press (hold ≥ 500 ms = settings) | `ENC_CW` `ENC_CC` / `ENC` |
| joystick | analog stick, four directions | `v.oai.rad` |
| touch pad | — (no known message; app event only) | |

What each key *does* is configured in ChatGPT Desktop → Settings → Codex
Micro, not on the pad, and gestures (double-press, holds) are timed by the
host. Host → pad: `sys.version`, `device.status`, `v.oai.thstatus` (agent
lights: colour, brightness, effect, speed), `v.oai.rgbcfg` (ambient =
underglow, keys = command-key backlight, same fields), `lights.preview` and
`host.focused_app` (acknowledged). Effects are the device kit's numbers —
0 off, 1 solid, 2 snake (a segment running along the strip; the app uses it
on the ring while the selected thread works or the mic records), 3 rainbow,
4 breath (the selected thread), 5 gradient, 6 shallow breath — each at its
own speed; the pad animates all of them (fw 0.8.1+; 0.8.0 only knew the
names the Bluetooth emulators use and showed the ring as breathing). Message shapes are documented in
`src/codex/mod.rs`; the codec is unit-tested on the host against the request
shapes the reference projects' probe scripts and protocol notes use
(`scripts/test-codex-wire.sh`), and `scripts/test-codex-compat.py` drives a
pad in compat mode end to end.

**Work Louder's Input app.** Input (Work Louder's configurator) talks to
the same interface, but through a device file system: on connect it lists
files, reads `keymap.json` and, if present, `smart_actions.json`, and saves
edits back as base64 chunks (`fs.list` / `fs.readbin` / `fs.writebin` /
`fs.read` / `fs.write` / `fs.delete`, SHA-1 checksums). The pad keeps those
two files in flash slots above the image (`src/codex/files.rs`: 12 KiB for
the keymap at `0x1B000`, 6 KiB for smart actions at `0x1E000` — see the
[flash map](#flash-map-header-and-boot-handoff)) and serves a built-in default keymap
— the ChatGPT layer above — until one is written. Chunks stream straight
into flash, so a 4 KB write never needs RAM.

The pad then *runs* the keymap (`src/codex/layout.rs`): the active profile
(`activeProfileId`, or a `KI_PS<n>` profile key) and layer decide what each
key, the dial, the touch pad (`buttons`) and the stick do.

| Keycode in Input | On the pad |
|---|---|
| `KV_OAI_AG00`–`AG05`, `ACT06`–`ACT12`, `ENC_CC`/`ENC_CW`/`ENC_CLK` | the Codex Micro controls (`v.oai.hid`) |
| `KC_*` (letters, numbers, glyphs, F1–F24, navigation, numpad, media, `KC_LCTL`… modifiers) | USB keyboard / consumer usages |
| `KI_LS<n>` / `KI_LM<n>` / `KI_PS<n>` | toggle layer n / layer n while held / profile n |
| Actions (`KA_A<n>`) | the macro's press / release / click steps, with delays, on a background task |
| Multi-actions (`KA_M<n>`) | the tap keycode (hold / double-tap variants not implemented) |
| Smart actions (`SA_<n>`) | `kb.sa.inserttext` / `exec` / `openapp` / `openurl` notifications, which the Input app executes on the host |
| `KI_CS_SHOW` / `HIDE` / `TOGGLE` / `SHOW_TMP` | `kb.cs.*` notifications (Input's cheat sheet) |
| `KI_BLUP` / `KI_BLDW` | backlight brightness |
| joystick `VENDOR` | the Codex Micro analog stick (`v.oai.rad`) |
| joystick `RADIAL` / `JOYSTICK` sectors | the sector's keycode for the four stick directions, plus `kb.radial` so Input can draw the menu |
| `KI_FP`, Bluetooth keys, `KI_X` | ignored |

A layer's `lights` (backlight / underglow) apply when it becomes active;
`lights.preview` from Input's lighting editor applies live. Not implemented:
Work Louder firmware updates through Input (`sys.bootloader` is refused),
the app manager / media player / wallpapers of screen-equipped models, and
per-app layer switching (`linkedApps`).

**Provenance and disclaimer.** The protocol is undocumented. This is an
independent Rust re-implementation of the behaviour documented by two
MIT-licensed projects that emulate the device over Bluetooth,
[`imliubo/codex-micro-4-core2`](https://github.com/imliubo/codex-micro-4-core2)
(Copyright (c) 2026 imliubo) and
[`digitsisyph/codex-micro-stopwatch`](https://github.com/digitsisyph/codex-micro-stopwatch).
Neither validated USB; that part is now confirmed against the real app,
whose bundled Work Louder device kit filters on VID `0x303A`, PIDs `0x8360`
(Codex Micro) / `0x8297` / `0x8298` (Creator Micro V2), usage page `0xFF00`,
and calls the link USB when the HID transport says so or, failing that, when
`bcdDevice % 4 == 0` (`codex::DEVICE_RELEASE`). It ignores the pad's other
HID interfaces, polls `device.status` about once a minute for battery, and
sends its lighting config with the command-key backlight off. Still
inferred, not observed: the `ACT11` id above. OpenAI, ChatGPT and Codex are
trademarks of OpenAI; Codex Micro and Work Louder belong to their owners.
The identifiers are emitted only so a compatible host recognises the device
and imply no affiliation, endorsement or official status — the mode is off
unless the owner turns it on, and a ChatGPT Desktop update can change the
protocol without notice.

## Building

```sh
rustup target add thumbv6m-none-eabi
cargo build --release
```

The image is linked at `0x08006000`, behind the resident bootloader
(`memory.x`; the 32-byte `.app_header` section and the `_stext` override
there are what put the header at `+0xC0` and `Reset` at `+0xE0`, which the
bootloader insists on). Release images are produced by
`scripts/build-firmware.sh`, which also stamps the header and prepends the
bootloader. Use the release profile: the unoptimised dev profile does not
fit the 84 KiB slot.

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
`warn,openmicro_fw=info` — this crate's boot/keymap/USB messages at `info`,
embassy at `warn`. Filtering is compile-time, so **`DEFMT_LOG=off` strips
logging out entirely** and is what production builds should use. The
per-tick diagnostics (joystick raw counts, touch charge time, executor
heartbeat) are `debug` and have to be opted into per invocation:

```sh
DEFMT_LOG=info,openmicro_fw=debug cargo run --release
```

Flash footprint of 0.10.0 by filter, against the 86,016-byte slot:

| `DEFMT_LOG` | image | headroom |
|---|---|---|
| `off` (releases) | 69,696 B | 16,320 B |
| `warn` | 84,112 B | 1,904 B |
| `warn,openmicro_fw=info` (default) | 85,264 B | 752 B |
| `info` | 85,424 B | 592 B |
| `info,openmicro_fw=debug` | 85,936 B | 80 B |

The debug stream is at the edge of the slot: a new `info!` on a busy path
may not link with it, which is why it is no longer the default.

## Flash map, header and boot handoff

Since fw 0.10.0 the pad boots through a **resident bootloader** (`../boot`,
24 KiB at the bottom of flash) and this firmware is linked behind it, at
`0x08006000`. The bootloader validates the application before every start
and stays in a driverless USB HID update mode whenever there is nothing
valid to run, so an interrupted update leaves a pad that still updates
instead of a dead one. Every number below is defined once, in
[`../layout/src/lib.rs`](../layout/src/lib.rs) (the `openmicro-layout`
crate shared by the bootloader, this firmware, the host app and the build
scripts) and mirrored in `memory.x`.

| Flash | Size | Contents |
|---|---|---|
| `0x08000000` | 24 KiB (12 pages) | bootloader; never erased by an update; its info block (magic `"OMKB"`, protocol, `app_base`, `app_size`, version) at `+0xC0` |
| `0x08006000` | 84 KiB (42 pages) | **application — this crate**: vector table, 32-byte header at `+0xC0`, `.text` from `+0xE0` |
| `0x0801B000` | 12 KiB | `keymap.json` slot (Work Louder files, `src/codex/files.rs`) |
| `0x0801E000` | 6 KiB | `smart_actions.json` slot |
| `0x0801F800` | 2 KiB | keymap/config page (`src/keymap.rs`) |

| RAM | Contents |
|---|---|
| `0x20000000`–`0x200000C0` | copy of the application's vector table: the Cortex-M0 has no VTOR, so SYSCFG remaps SRAM to address 0 instead (`src/boot.rs`) |
| `0x200000C0`–`0x20003FF0` | `.data` / `.bss` / stack of whichever image is running |
| `0x20003FF0`–`0x20004000` | the handoff words, below |

**The application header** (`src/boot.rs` `APP_HEADER`, 32 bytes at
`0x080060C0`, little-endian):

| Offset | Field |
|---|---|
| 0 | magic `"OMKA"` (`0x414B4D4F`) |
| 4 | image length in bytes from `0x08006000`, a multiple of 4 |
| 8 | CRC-32 (ISO-HDLC — `zlib.crc32`, `crc32fast`) over the image with this field taken as zero |
| 12 | header version `1` (u16), flags (u16): bit 0 set = built with `--features proto` (`openmicro_layout::FLAG_PROTO`); the bootloader refuses to install an image whose bit disagrees with its own info-block `variant`, so a `proto` build cannot land on a production pad or vice versa |
| 16 | `CARGO_PKG_VERSION`, NUL padded to 16 bytes |

Straight out of the linker the length and CRC are **zero** — an *unstamped*
image. `scripts/build-firmware.sh` runs `scripts/fw-image.py patch` on the
`.bin`, which fills them in; that *stamped* image is what releases carry and
what the bootloader CRC-checks before every start. An unstamped image — what
`cargo run` / probe-rs flashes from the ELF — is accepted on the strength of
its vector table alone (initial SP inside RAM, reset vector exactly
`0x080060E1`), so development flashing needs no extra step. `BOOT_INFO`
reports which kind is running (`app_valid` 1 stamped, 2 unstamped). The
bootloader's HID update protocol, on the other hand, refuses unstamped
uploads outright (`UPDATE_BEGIN` with `crc == 0`, or a header chunk whose
length/crc words differ from what BEGIN announced) so that an interrupted
upload can never leave a vectors-only image the boot path would accept.

**Handoff words** (the top 16 bytes of RAM; SRAM survives a system reset but
not a power cycle, and both images keep the words outside their RAM
regions):

| Address | Word | Meaning |
|---|---|---|
| `0x20003FF0` | request | `"BOOT"` `0x424F4F54`: stay in the bootloader's update mode · `0xB00710AD`: jump to the ROM DFU · `"RUN!"` `0x214E5552` (bootloader-internal, after `BOOT_RUN`): start the app without the switch check |
| `0x20003FF4` | !request | bitwise complement of the request; both must match, so power-up garbage never counts |
| `0x20003FF8` | reason | written by the bootloader before it starts the app — 1 normal, 2 first boot after a verified upload, 3 the pad had been put into update mode by holding the encoder switch (both 2 and 3 are what update mode left behind and are carried through only across its own `BOOT_RUN`); logged at startup |
| `0x20003FFC` | faults | the bootloader's fault counter (`0x5A5A00nn`: panic, HardFault, or any unbound exception). A fault in this firmware's init window — before `remap_vectors_to_sram` has run, exceptions still vector through the bootloader's table — counts here, and the next boot stays in update mode instead of starting us again. `main` clears it with `boot::clear_faults()` right after the remap; the bootloader clears it once update mode has answered a `BOOT_INFO` |

The bootloader clears the request pair before acting on it, so a request
never replays. The application side is `src/boot.rs`: `request(Bootloader |
RomDfu)` writes the pair and resets; `remap_vectors_to_sram()` re-applies
the SRAM remap after `embassy_stm32::init` (which resets SYSCFG and would
otherwise leave the bootloader's vectors at address 0 — `main` keeps
interrupts masked across both); `clear_faults()` follows it; `boot_info()`
reads the bootloader's info block for `BOOT_INFO`. Until 0.9.0 the firmware
itself jumped into the ROM DFU from a magic word in `.uninit` RAM at a
linker-chosen address; the handoff words replace that.

## Flashing and field updates

The board has **no BOOT0 button** (BOOT0 is strapped low through `rboot`)
and no reset line on the debug socket, so everything below is either SWD or
software-triggered over USB.

**Development: SWD (J2 socket)** — with a probe (ST-Link, CMSIS-DAP, …)
attached to J2:

```sh
cargo install probe-rs-tools
cargo run --release        # runner = probe-rs run --chip STM32F072CBTx
```

This flashes the ELF at its link address, `0x08006000`; the bootloader must
already be in the 24 KiB below it (`cd ../boot && cargo run --release` once,
or any combined release image via the ROM DFU). The bootloader starts the
unstamped image as it is.

**The vendor HID interface** (usage page `0xFF60`, 32-byte reports, no
report IDs) carries the whole app protocol — replies echo the command byte;
any IN report starting `0x80` is an unsolicited input event, not a reply:

| OUT report | Effect |
|---|---|
| `[0x01, …]` | Replies `[0x01, len, "0.10.0"…]` — running firmware version |
| `[0x02, 'D','F','U','!']` | Acks `[0x02, 0x01]`, then resets with the ROM-DFU request; the *bootloader* jumps into the ST ROM DFU (`0483:df11`) from reset state. **Only for reinstalling the bootloader** with a combined image (below) |
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
| `[0x10]` | Replies `[0x10, mode]` — running device mode (0 OpenMicro / 1 Codex Micro compat, fw 0.8.0+) |
| `[0x11, mode, 'M','O','D','E']` | Acks `[0x11, ok]`; a changed mode is saved (the whole RAM configuration, like SAVE) and the pad resets to re-enumerate in it |
| `[0x12, 'B','O','O','T']` | Acks `[0x12, 0x01]`, then (50 ms later) resets into the bootloader's HID update mode — the pad re-enumerates as `1209:0002` "OpenMicro Bootloader". A wrong key is refused with `[0x12, 0x00]` (fw 0.10.0+) |
| `[0x20]` | Replies `[0x20, status, proto, boot_major, boot_minor, boot_patch, app_base u32, app_size u32, app_valid, app_version[16]]` — 31 bytes, little-endian (`openmicro_layout::BootInfoReply`): status 0 with the bootloader's protocol (1), version and layout (`0x08006000`, `0x15000`), `app_valid` 1 stamped / 2 unstamped, and this firmware's version NUL-padded; status 1 with the rest zeroed if there is no bootloader info block (fw 0.10.0+). The bootloader answers the same opcode on its own interface with `page u16, max_chunk u8` appended |

The Codex Micro compat interface (usage page `0xFF00`) is documented in
[`src/codex/mod.rs`](src/codex/mod.rs); its keymap file format in
[`src/codex/layout.rs`](src/codex/layout.rs).

Event reports (`[0x80, src, a, b]`, best-effort, dropped when no host reads):
src 0 = key (a = position 0–12, b = pressed), 1 = encoder rotate (a = 1 CW),
2 = encoder button, 3 = joystick (a = dir 0 up / 1 down / 2 left / 3 right /
4 press, b = active), 4 = touch tap. These give the companion app live press
feedback — and a hardware test — with no OS input-monitoring permission.

**Field updates: the bootloader's HID update mode — no driver, no buttons.**

The host app sends `ENTER_BOOT`; the pad re-enumerates as `1209:0002`
"OpenMicro Bootloader" (keys breathing amber), a raw-HID device on the same
usage page with 64-byte reports that speaks `BOOT_INFO` / `UPDATE_BEGIN` /
`UPDATE_DATA` / `UPDATE_END` / `BOOT_RUN` — see
[`../boot/README.md`](../boot/README.md). The bootloader erases and programs
only the application slot, checks the header's CRC over flash at
`UPDATE_END`, and re-validates before every start; the config page and the
file slots are never touched, so the keymap and the Codex-mode setting
survive. If an upload is interrupted the pad simply comes back in update
mode with `app_valid = 0` and the upload is repeated; an image built for
the other board variant (`proto` vs production, header flags bit 0) is
refused at `UPDATE_END` and the old image stays. Holding the encoder switch
while plugging in enters update mode too (the app then logs reason 3 after
`BOOT_RUN`), should the application ever be unable to answer — as does the
boot after a firmware that faulted during its own start-up, so a bad
release can be replaced rather than reset-looping. Raw HID needs no driver
on any OS (Linux wants a udev rule for `1209:0002`,
`docs/linux-firmware-updates.md`).

Release images are **combined**: `dist/openmicro-fw-<version>.bin` is the
bootloader padded to 24 KiB followed by the stamped application, so its
first words are the bootloader's vectors and it flashes at `0x08000000`.
The host app reads `app_base` from the bootloader's info block (never
hard-coded) and slices the application out for the HID path.

**The ROM DFU** (`ENTER_DFU`, `0x02`) remains for exactly two cases: the
one-time migration of a pad running fw ≤ 0.9.0 (which has no bootloader —
the host app detects that and uses the old DfuSe path with the *combined*
image, installing the bootloader), and reinstalling the bootloader itself.
It writes from `0x08000000`, so it must only ever be given a combined
image; an application-only image there would overwrite the bootloader with
code linked for the wrong address. On Windows it needs the WinUSB (Zadig)
binding; on some Linux xHCI hosts the ROM's DNLOAD bug bites
(`docs/linux-firmware-updates.md`).

```sh
dfu-util -a 0 -s 0x08000000:leave -D dist/openmicro-fw-0.10.0.bin   # combined image only
```

`bcdDevice` in the USB device descriptor carries the semver from
`Cargo.toml` (`version_bcd`) except in Codex Micro compat mode, where it is
fixed at `0x0100` (`codex::DEVICE_RELEASE`); `BOOT_INFO` and the `0x01`
VERSION command are the version sources to trust. Minimal host flow
(Python + `hidapi`):

```python
import hid
dev = next(d for d in hid.enumerate(0x1209, 0x0001) if d["usage_page"] == 0xFF60)
h = hid.device(); h.open_path(dev["path"])
h.write(bytes([0x00, 0x20]))               # leading 0x00 = report ID; BOOT_INFO
print(h.read(32, 500))                     # [0x20, 0, 1, boot maj, min, patch, ...]
h.write(bytes([0x00, 0x12]) + b"BOOT")     # ENTER_BOOT -> [0x12, 0x01]
# device drops off and re-enumerates as 1209:0002 -> the app (or ../boot's
# protocol) uploads the application slot and sends BOOT_RUN
```

**Failure model.** An interrupted *application* update can no longer brick
the pad: the bootloader never erases its own pages and validates before
every jump, so the worst case is a pad waiting in update mode. A firmware
that HardFaults before `main` has re-taken the vector table is caught by
the bootloader's fault counter and also ends in update mode on the next
boot (see `../boot/README.md`, Faults). What is left is the bootloader
itself — only a ROM-DFU write of the combined image (or SWD) touches it,
and if *that* is interrupted the recovery is the SWD port on J2 (no BOOT0
button, no reset line: the probe attaches to the running core).
