# OpenMicro companion app

Cross-platform desktop app for the OpenMicro macropad, built with
[GPUI](https://gpui.rs) and
[GPUI Component](https://github.com/longbridge/gpui-component). The native,
GPU-accelerated interface uses platform-quality text rendering and a compact
8-bit visual system; the whole product stack (compiler-designed board,
embassy firmware, host app) stays in Rust.

```sh
cargo run --release --locked --bin openmicro-app
```

## The shape of it

A single 8-bit-styled surface, not tabs: a product header keeps the active
profile, connection state, and settings close; beneath it, a board-like
hardware map and a structured input inspector share the workspace. Quiet
one-pixel containment, selective stepped shadows, paired light and dark
palettes, and compact display labels give the app a hardware-workstation
character without sacrificing native text clarity. Appearance follows the
operating system by default and can be pinned to Light or Dark in Settings.
The map is drawn true to life — encoder and joystick as dials, the touch pad
as a disc, and all **13 keys as independent 1U cells**. Selecting any input
opens its editor beside the grid; macros, settings and firmware updates are
focused sheets over the pad; a menubar item carries profile switching and
connection status. Disconnected, the board remains fully legible and an
offline-editing callout explains that the active profile will sync when the
pad returns.

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

## Joystick modes

The joystick editor offers four behaviors, stored on the pad: **arrow keys**
(direction slots under an optional shared modifier mask), **custom keys**
(every direction and the push fully configurable), **mouse pointer**
(proportional HID pointer motion, push = left click), and **color grading** —
pointer motion with the speed applied squared (the slider spans sub-pixel
nudges to brisk drags) and the left button auto-held while the stick is
deflected. In DaVinci Resolve's color page, park the
pointer over a color wheel and the stick grabs and nudges it like a panel
trackball, letting go when it returns to center. It is plain mouse input —
no scripting API, panel drivers, or Studio license involved — so it works in
the free Resolve (and any other app that drags, e.g. Lightroom sliders).
Grade mode needs firmware ≥ 0.6.0; on older firmware the app detects the
silent downgrade to keys mode and reports that the extras need a firmware
update.

## Profiles

Named profiles are first-class: full pad configuration (bindings, labels,
Lucide symbols or Simple Icons brand marks, emitted codes, joystick threshold)
stored app-side in a human-readable JSON under the OS config dir,
exported/imported as one file
(merge or replace). Switching — strip chevrons, dropdown, or the menubar —
writes the keymap to the device and persists it to device flash. Ships with
one default profile, **Codex**, matching the Codex Micro keycap set (FAST /
APPR / REJ / SPLIT / NEW / TERM / PLAY / GIT / PR / DIFF / MIC / MIC / SETUP,
icons from the full bundled Lucide set). The icon picker also includes the
bundled Simple Icons catalog for monochrome brand marks. Brand names and logos
remain trademarks of their respective owners.

## Firmware updates

The product's field-update path (no buttons, no probe): the sheet checks the
image, sends `ENTER_DFU` over raw HID, speaks DfuSe (AN3156) directly over
libusb to the ROM bootloader (`0483:df11`), and verifies the reported version
when the pad returns. Release builds bundle the exact production firmware; a
newer independent firmware release can also be downloaded from the GitHub
Release path and is checked for size, SHA-256, board, protocol, and
Cortex-M0 vectors before flashing. If the app stops while the powered device
remains in ROM DFU, Install can resume. Do not unplug during flashing: power
loss can leave recovery requiring SWD on J2. **Profiles and the on-device
keymap survive updates** — the keymap lives in a reserved flash page updates
never touch.

## App updates

Release builds check `release-manifest.json` at startup and every six hours.
When a newer host version exists, the banner downloads the DMG for the running
Mac architecture, verifies its declared size and SHA-256, and opens it so the
user can replace OpenMicro in Applications. See [`../RELEASING.md`](../RELEASING.md)
for packaging, Developer ID signing, notarization, and publishing.

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

This crate is standalone (like `../fw`), but the tag-triggered release workflow
builds and publishes it as native Apple Silicon and Intel DMGs.
