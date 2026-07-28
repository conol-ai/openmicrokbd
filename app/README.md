# OpenMicro companion app

Cross-platform desktop app for the OpenMicro macropad, built with
[makepad](https://makepad.dev) — the whole product stack (compiler-designed
board, embassy firmware, host app) stays in Rust.

```sh
cargo run --release
```

## The shape of it

A single surface, not tabs: **the pad is the home screen** and the only
permanent view. A slim profile strip on top, the grid drawn true to life in
the middle — encoder and joystick as dials, the touch pad as a disc, and all
**13 keys as independent 1U cells** — and a status line at the bottom
(connection dot, firmware + serial when connected, settings gear). Selecting
any input opens its editor beside the grid; macros, settings and firmware
updates are sheets over the pad; a menubar item carries profile switching and
connection status. Disconnected, the grid ghosts and a card explains —
profiles live in the app, so everything stays editable.

## How an input does something

Two layers, per the PRD's architecture decision:

1. **The pad emits a configurable HID code.** Every input slot (13 keys,
   encoder CW/CCW/press, joystick directions/press, touch tap) stores its
   emitted code — keyboard usage + modifier mask, or consumer usage — **in
   device flash**, written over the vendor HID interface (`1209:0001`, usage
   page `0xFF60`). Switching a profile writes its keymap to the pad, so the
   pad emits the right codes on any machine, app running or not. Factory
   defaults (F13–F20, Shift+F13…F17) are interceptable on all three OSes by
   construction — no macOS F21–F24 dead keys.
2. **The app optionally intercepts that code** OS-wide (`RegisterEventHotKey`
   on macOS — no accessibility permission needed for the grab itself) and
   runs the bound action instead of letting it type: **keystroke** (recorded
   chord), **macro** (ordered steps with delays: keystroke / delay / run /
   open / media), **run command**, **open app or URL**, **media control**, or
   **app settings**. Keystroke and media *synthesis* does need the
   Accessibility permission; the app asks with an explanation, shows a
   "not listening" state without it, and stays fully usable read-only.

Live press feedback needs neither layer: the firmware streams input events
over the vendor interface (`0x80` reports), so pressing a physical key lights
its on-screen cell — a built-in hardware test that works with zero OS
permissions.

## Profiles

Named profiles are first-class: full pad configuration (bindings, labels,
Lucide icons, emitted codes, joystick threshold) stored app-side in a
human-readable JSON under the OS config dir, exported/imported as one file
(merge or replace). Switching — strip chevrons, dropdown, or the menubar —
writes the keymap to the device and persists it to device flash. Ships with
one default profile, **Codex**, matching the Codex Micro keycap set (FAST /
APPR / REJ / SPLIT / NEW / TERM / PLAY / GIT / PR / DIFF / MIC / MIC / SETUP,
icons from the full bundled Lucide set).

## Firmware updates

The product's field-update path (no buttons, no probe): the sheet checks the
image (Cortex-M0 vector table for 128 K flash), sends `ENTER_DFU` over raw
HID, speaks DfuSe (AN3156) directly over libusb to the ROM bootloader
(`0483:df11`), and waits for the pad to come back. An update banner appears
on the home screen when a connected pad runs an older firmware than the app
ships against. A stranded bootloader (interrupted update) is picked up by
Install automatically. **Profiles and the on-device keymap survive updates**
— the keymap lives in a flash page updates never touch.

## Platform notes

- **macOS** — interception and live feedback need no permission; only
  keystroke/media *synthesis* (actions that type or press media keys for
  you) needs Accessibility, requested with a deep link. macOS has no
  virtual keycodes for F21–F24; the editor marks any code the OS cannot see.
- **Windows** — DFU needs a WinUSB driver bound to `0483:df11` once (Zadig).
  Interception uses `RegisterHotKey`; synthesis needs no special permission.
- **Linux** — udev rules needed for `1209:0001` (hidraw) and `0483:df11`
  (DFU); interception depends on the session (X11 grabs; Wayland varies).

## Known deferrals

- The menubar popover is a native menu (status, profiles, quick actions,
  firmware footer) — the PRD's mini pad mirror inside the popover needs a
  custom platform view and is deferred.
- The window is sized for grid + editor rather than growing/shrinking as the
  editor opens (makepad window resizing at runtime is not yet reliable).
- Touch swipe left/right slots exist end-to-end in config and protocol, but
  the current single-zone pad cannot detect swipe direction — hardware
  revision territory.
- Joystick key-repeat rate is not separately configurable: a held direction
  holds its keycode, so the OS's own key-repeat applies. A per-binding
  repeat-rate override is deferred.
- While a shortcut-record is armed, keys typed into a focused text field are
  captured as the shortcut *and* typed into the field — click Record before
  clicking into any field.
- Alternating one pad between machines with different active profiles writes
  the keymap flash page once per plug (the profile always wins). The page is
  rated for 10k erase cycles — years of plugging — but worth knowing.
- Per the PRD's out-of-scope list: no auto per-app switching, no layers, no
  lighting control, no snippets, no multi-device, no plugins.

This crate is standalone (like `../fw`) — the repository's CI does not build
it; it is part of the example product's deliverables, not the CoHDL compiler.
