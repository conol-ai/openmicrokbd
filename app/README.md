# OpenMicro companion app

Cross-platform desktop app for the OpenMicro macropad, built with
[makepad](https://makepad.dev) — the whole product stack (compiler-designed
board, embassy firmware, host app) stays in Rust.

```sh
cargo run --release
```

## What it does

**Device dashboard** — finds the pad over its vendor HID interface
(`1209:0001`, usage page `0xFF60`), shows live connection state, running
firmware version (in-band version query) and serial.

**Firmware updates** — the product's field-update path (no buttons, no
probe): pick a `.bin`, press Install, and the app

1. sanity-checks the image (Cortex-M0 vector table for 128K flash / 16K RAM
   — a wrong file is refused before anything is touched),
2. sends the `ENTER_DFU` command over raw HID; the firmware reboots into
   the STM32F072's ROM bootloader (`0483:df11`, same USB-C port),
3. speaks DfuSe (AN3156) directly over libusb: erase the covered 2 KiB
   pages, program in `wTransferSize` blocks, set the address pointer and
   manifest — the pad boots the new firmware,
4. waits for the pad to re-enumerate and confirms the new version.

A previously interrupted update is picked up automatically: if a bare
`0483:df11` bootloader is present, Install skips step 2 and just flashes.
(If power was lost mid-flash *and* the pad was unplugged, the app is gone
and BOOT0 is strapped low — recovery is SWD on J2, by design.)

To produce an image from the firmware crate:

```sh
cd ../fw
cargo objcopy --release -- -O binary openmicro.bin
```

**Key actions** — the pad's 13 keys arrive as F13..F24 (the 2U cap's two
switches share F23). Each can be bound to a host-side action, saved to
`<config-dir>/OpenMicro/config.json` and registered as global hotkeys:

- *Run command* — `sh -c` / `cmd /C`
- *Open URL / file* — OS default handler (URL, file, or app)

Encoder (volume/mute), touch bar (play/pause) and joystick (arrows/enter)
are ordinary media/arrow usages the OS handles directly — nothing to
configure.

## Platform notes

- **macOS** — no Input-Monitoring permission needed (the app only opens the
  vendor usage page, not the keyboard interface). macOS has no F21-F24
  virtual keycodes, so the last four F-keys can't trigger host actions
  there; the UI marks them. DFU works out of the box.
- **Windows** — driving the DFU device (`0483:df11`) needs a WinUSB driver
  bound to it once (Zadig, or libwdi in an installer).
- **Linux** — udev rules needed for unprivileged access to `1209:0001`
  (hidraw) and `0483:df11` (DFU).

This crate is standalone (like `../fw`) — the repository's CI does not
build it; it is part of the example product's deliverables, not the CoHDL
compiler.
