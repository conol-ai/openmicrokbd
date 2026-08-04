# OpenMicroKbd

An open-source hardware recreation of OpenAI's **Codex Micro** macropad — 13 low-profile keys, a rotary encoder, an analog joystick, a capacitive touch pad, and per-key + underglow RGB on a wired USB-C board built around an STM32F072.

**The PCB is written, not drawn.** The entire schematic is source code in **CoHDL**, a new AI-native hardware description language (see below). The compiler type-checks the design — units, pin-connection obligations, power integrity — and emits the netlist, BOM, footprints and layout constraints you can inspect under [`out/`](out/).

🌐 **https://openmicrokbd.org** · License: [MIT](LICENSE)

## Designed in CoHDL

This board was designed in **CoHDL**, a hardware description language newly developed by [Tony Huang](https://github.com/conol-ai) that makes schematic (PCB) design AI-native: a board is a program — typed, checked, and compiled. In CoHDL, the design in [`src/`](src/) *is* the schematic; the compiler is the oracle that grades it.

**The CoHDL language and compiler will be open sourced soon.** Until then, this repository ships the complete compiler outputs under `out/`, so the design is fully inspectable and the firmware and app are buildable today.

## What's inside

| Path | Contents |
|---|---|
| [`src/`](src/) | The PCB as CoHDL source: `main.cohdl` (top-level design), `openmicro_parts.cohdl` (datasheet-verified part bindings), `footprints.cohdl`, `pads.cohdl` |
| [`out/`](out/) | Checked-in compiler output: KiCad netlist (`openmicro.net`), BOM (`openmicro-bom.csv`), SMT placement (`openmicro-smt.csv`), `.kicad_mod` footprints, layout constraints (`openmicro-layout.json`) |
| [`fw/`](fw/) | Firmware — Rust + [embassy](https://embassy.dev) on STM32F072CB: key matrix, twin WS2812 chains, USB HID + vendor interface, DFU reboot. See [`fw/README.md`](fw/README.md) |
| [`app/`](app/) | Companion desktop app — Rust + [makepad](https://makepad.dev): device dashboard, live connection state, and button-free firmware updates over USB. See [`app/README.md`](app/README.md) |
| [`pcb/`](pcb/) | The routed board — `openmicro.kicad_pcb` + KiCad project (design rules) |
| [`fab/`](fab/) | Manufacturing package: Gerber/drill zip, assembly BOM, SMT placement CSV — see [`fab/README.md`](fab/README.md) |
| [`mechanical/`](mechanical/) | Board outline (DXF) |
| [`docs/`](docs/) | Manufacturer datasheets for every active, connector, and electromechanical part in the BOM, with sourcing/verification notes — see [`docs/README.md`](docs/README.md) |

## Hardware

- **MCU:** STM32F072CBT6 (Cortex-M0), 8 MHz HSE crystal, 2×3 SWD debug socket
- **Keys:** 13× Kailh Choc V2 low-profile switches on a 19.05 mm grid (square 4×4-cell frame, matching the Codex Micro layout), 1N4148W per-key matrix diodes
- **Inputs:** EC11 rotary encoder, RKJXV analog joystick, capacitive touch pad
- **Lighting:** 13× per-key SK6812MINI-E (reverse-mount, one WS2812 chain) + 8× perimeter underglow SK6812MINI-E (second chain)
- **USB:** Type-C wired, USBLC6 ESD protection, AP2112 3.3 V LDO
- PCB laid out from the CoHDL-generated constraints; manufacturing handoff via IPC-2581

## Building

```sh
# Production firmware binary + debug ELF
scripts/build-firmware.sh dist

# Native macOS app bundle + DMG (the app is ad-hoc signed for local testing)
scripts/package-macos.sh dist dist/openmicro-fw-<firmware-version>.bin

# Or run the companion app from source
cd app && cargo run --release --locked
```

Rebuilding the PCB from `src/` requires the CoHDL compiler, which is being prepared for open source — the generated netlist/BOM/footprints are checked in under `out/` in the meantime.

Signed releases build both Intel and Apple Silicon DMGs plus firmware through
GitHub Actions. See [`RELEASING.md`](RELEASING.md) for versioning, Apple
signing/notarization secrets, the required device smoke test, and the tag flow.

## Production files

The final routed board lives in [`pcb/`](pcb/) (`openmicro.kicad_pcb`, opens in KiCad 9+). The manufacturing package — Gerber/drill zip, assembly BOM, and SMT pick-and-place CSV, ready to upload to a fab such as JLCPCB — is in [`fab/`](fab/), with DRC status and ordering notes in [`fab/README.md`](fab/README.md).

## License

[MIT](LICENSE) © 2026 Tony Huang. Build your own — attribution appreciated.

---

*OpenMicroKbd is an independent open-source project and is not affiliated with, endorsed by, or sponsored by OpenAI. "Codex Micro" refers to OpenAI's product; all trademarks belong to their respective owners.*
