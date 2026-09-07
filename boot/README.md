# OpenMicro v1 resident bootloader (`openmicro-boot`)

The bootloader owns the first 24 KiB of the STM32F072CB's flash and is the
only thing that ever writes the application slot. Before every start it
checks the application's header and CRC-32; whenever the application is
missing, invalid, asked for it, or the encoder switch is held at power-up, it
stays in a **driverless USB HID update mode** instead of running anything.
An interrupted update therefore leaves the pad in update mode, never dead —
the board has no BOOT0 button and no NRST pin, so without this an erased or
half-written flash could only be recovered with an SWD probe.

Shared constants (flash/RAM map, header, handoff words, protocol, CRC) live
in [`../layout`](../layout/src/lib.rs) and are used unchanged by the
firmware, the host app and the scripts.

## Flash and RAM map

| region | address | size | notes |
|---|---|---|---|
| bootloader | `0x08000000` | `0x6000` (12 pages) | never erased by updates; info block at `+0xC0` |
| application | `0x08006000` | `0x15000` (42 pages) | header at `+0xC0`, `Reset` at `+0xE0` |
| keymap.json / smart_actions.json / config | `0x0801B000`.. | 20 KiB | untouched (`DATA_BASE`) |

| RAM | use |
|---|---|
| `0x20000000..0x200000C0` | copy of the application's vector table (Cortex-M0 has no VTOR: the bootloader copies the table here and remaps SRAM to address 0 just before the jump) |
| `0x200000C0..0x20003FF0` | `.data/.bss/stack` of whichever image runs (both `memory.x` files start RAM here) |
| `0x20003FF0..0x20004000` | handoff words: `REQUEST`, `!REQUEST`, `REASON`, `FAULTS` |

The 32-byte **info block** at `0x080000C0` (`"OMKB"`, protocol 1, variant
0 prod / 1 proto, `app_base`, `app_size`, version string) is what the
application answers `BOOT_INFO` from and what the host uses to slice a
combined image; nothing else hard-codes the layout.

## Boot decision (`src/main.rs`, before any HAL init, on the 8 MHz HSI)

1. **Hygiene**, in this order: SysTick off (`SYST_CSR = 0`), a pending
   SysTick/PendSV cleared (`SCB_ICSR` `PENDSTCLR | PENDSVCLR`), every NVIC
   line disabled and unpended (`ICER/ICPR = 0xFFFFFFFF`) — and only *then*
   SYSCFG clock on and `MEM_MODE = MAIN_FLASH` (the ROM DFU `leave` path
   arrives with system memory mapped at 0), so nothing inherited can be
   taken through our table while the core is half-configured. Then sysclk
   back on HSI and PRIMASK cleared (the ROM may hand over with interrupts
   masked; update mode's USB interrupt has to run). PA13/PA14 (SWD) are
   never touched anywhere.
2. Read and clear the **handoff request** (valid only if `!REQUEST` matches;
   power-up garbage never does):
   - `REQ_ROM_DFU` (`0xB00710AD`) → map system memory, bootstrap into the ST
     ROM DFU (0483:DF11) — exactly what `fw/src/dfu.rs` used to do;
   - `REQ_BOOTLOADER` (`"BOOT"`) → update mode;
   - `REQ_RUN` (`"RUN!"`, written by `BOOT_RUN`) → straight to step 4.
3. Otherwise, **a fault counted on the previous boot** (`FAULTS` ≥ 1, see
   [Faults](#faults)) → update mode: the application died in its init
   window (or the entry path did) and a second jump would only repeat it.
   Then **sample the encoder switch** (PB15, `proto`: PC15): pull-up on,
   1 ms settle, 20 samples 1 ms apart (busy loop calibrated for 8 MHz); all
   low → update mode with `REASON_SWITCH`. The pin and its clock go back to
   reset state.
4. **Validate** the slot with `openmicro_layout::validate_app`: magic,
   header version, stack pointer inside RAM, reset vector exactly
   `0x080060E1`, and for a stamped header `0xE0 <= length <= 0x15000`,
   `length % 4 == 0`, CRC-32 over `[0, length)` (the header's own crc field
   taken as 0). An **unstamped** header (`length == crc == 0`, what
   `cargo run` / probe-rs leaves) passes on the vector checks alone — the
   HID protocol never produces one (below).
5. Valid → write `REASON` (`1` normal; after a `BOOT_RUN` the value update
   mode left is carried through: `2` first boot after a verified upload,
   `3` update mode had been entered by the switch), copy the 0xC0-byte
   table to `0x20000000`, `MEM_MODE = SRAM`, SysTick off, PRIMASK clear,
   `bootload(0x08006000)`. `FAULTS` is *not* cleared here: the application
   clears it once it runs (`fw/src/boot.rs`). Invalid → update mode.

## Update mode

`embassy_stm32::init` with the application's clock configuration (HSI48
trimmed by CRS from USB SOF, 48 MHz core), then a hand-built
`embassy_executor::Executor` with one task. USB identity: **1209:0002**,
manufacturer `conol`, product `OpenMicro Bootloader`, serial = the MCU's
96-bit UID in hex, `bcdDevice` = the bootloader version. One HID interface:
usage page `0xFF60`, usage `0x61`, **64-byte** IN and OUT reports, no report
IDs, 1 ms polling. Update mode never times out; only `BOOT_RUN`,
`ENTER_DFU` or a power cycle leave it.

### LEDs

| keys (13) | meaning |
|---|---|
| amber breathing (2 s period, dim) | idle in update mode, waiting for a host |
| solid green for 1 s | `UPDATE_END` verified the new image |
| three red blinks | any error status (bad chunk, CRC mismatch, unknown opcode, ...) |
| solid red, no USB | third fault in a row: halted, SWD-attachable (see Recovery) |

The underglow ring is written dark once on entry so it does not keep the
application's last frame. Brightness is capped at 40/255 per channel.

### Protocol (`src/update.rs`)

64-byte reports; the host prepends report id 0 (65-byte writes). Byte 0 is
the opcode, replies echo it (bit 7 never set) and are always full 64-byte
reports, zero padded. Little-endian fields. Codes: `openmicro_layout::{op,
status}`.

| op | request | reply |
|---|---|---|
| `0x01` VERSION | `[0x01]` | `[0x01, len, "boot X.Y.Z"]` |
| `0x02` ENTER_DFU | `[0x02, 'D','F','U','!']` | `[0x02, 1]`, then after 50 ms arm `REQ_ROM_DFU` and reset (the boot path jumps into the ROM DFU from reset state). Without the key: `[0x02, 0]` |
| `0x20` BOOT_INFO | `[0x20]` | `[0x20, 0, proto=1, boot maj, min, patch, app_base u32, app_size u32, app_valid u8, app_version[16], page u16 = 2048, max_chunk u8 = 56]` (34 bytes). `app_valid`: 0 none, 1 valid (stamped, CRC ok), 2 unstamped; `app_version` from the header when valid, else zeros |
| `0x21` UPDATE_BEGIN | `[0x21, length u32, crc u32]` | erases `ceil(length / 2048)` pages from the slot start (may take ~1.5 s), then `[0x21, status]`: 0 ok, 1 bad length (must be `0xE0..=0x15000` and a multiple of 4), 4 `crc == 0` (an unstamped image; refused *without* erasing, the old image stays valid), 2 flash error. Always restarts an upload in progress |
| `0x22` UPDATE_DATA | `[0x22, offset u32, n u8, data[n]]` | programs immediately, `[0x22, status, next_offset u32]`: 0 ok, 3 out of order (resume from `next_offset`), 1 bad n (`1..=56`, a multiple of 4 unless `offset + n == length`, inside `length`), 4 the chunk carrying the header's length/crc words (slot offsets `0xC4..0xCC`) does not carry the values BEGIN announced — refused *before* it is programmed, upload closed, 2 flash error (upload closed), 5 no BEGIN |
| `0x23` UPDATE_END | `[0x23]` | `[0x23, status]`: 0 valid, 4 header invalid (also: the slot reads back unstamped, or the header's `flags` bit 0 disagrees with this bootloader's variant — a `proto` image on a production pad or vice versa), 6 CRC mismatch (also when the header's crc differs from the one announced in BEGIN), 3 not all bytes received yet (upload stays open), 2 flash error, 5 no BEGIN |
| `0x24` BOOT_RUN | `[0x24]` | `[0x24, 0]`, then after 50 ms arm `REQ_RUN` and reset; the boot path re-validates and starts the app, or comes back in update mode if it is invalid |
| other | | `[op, 0xFF]` |

Any status other than 0 leaves the slot as it is; the validation before
every boot is what keeps a half-written slot from ever running. A host that
loses a reply simply resends: a repeated chunk answers `3` with the offset
to continue from. Suggested host timeouts: BEGIN 8 s, DATA/END 2 s,
BOOT_INFO 1 s.

**Only stamped images get in over HID.** The boot path accepts an
unstamped header on its vectors alone (for probe-rs), so the protocol must
never leave one behind: BEGIN refuses `crc == 0`, and the DATA chunk that
carries the header's length and crc words is compared with BEGIN before it
is programmed — a hand-rolled client sending a raw `objcopy` image is
stopped there with the slot still lacking its magic, never with a
bootable-looking header over a partial body. The same check catches an
image/crc mix-up after ~200 bytes instead of after the whole upload. END
additionally refuses a slot that reads back unstamped (defence in depth)
and an image built for the other board variant (`flags & 1` must equal the
info block's `variant`, i.e. `proto` builds only install on a `proto`
bootloader). The variant rule is part of the slot validation itself
(`openmicro_layout::validate_app_for`), so the boot path and BOOT_INFO apply
it too: a wrong-variant image that a refused upload left in the slot is
reported as `app_valid 0` and is never started, not even when a debugger
flashed it. `scripts/build-firmware.sh` builds both crates with the same
`FW_FEATURES`, so release images always match.

## Faults

Panics, HardFaults and every exception or interrupt nothing else binds
(`DefaultHandler`: SysTick, NMI, a spurious IRQ line — cortex-m-rt's own
default would park the core in `loop {}`, dark until a power cycle) go
through one ladder and never produce a reset loop. `FAULTS`
(`0x20003FFC`, `0x5A5A0000 | count`) is incremented and:

| count | action |
|---|---|
| 1 | `sys_reset` — with `REQ_BOOTLOADER` armed first if the fault happened in update mode, so the retry lands in update mode again (a plain reset would find the valid application, boot it, and quietly leave a pad that can never be updated over HID). On the entry path: plain reset, and the next boot goes to update mode because it sees the count (step 3 above) |
| 2 | arm `REQ_ROM_DFU`, `sys_reset` → the pad appears as 0483:DF11; reinstall the **combined** image with dfu-util / the host app |
| 3+ | paint the keys red if the clocks are up, then spin on `nop` (never WFI) so an SWD probe can attach through J2 |

Who clears the word: **the application**, once it is running with its own
vector table (`fw/src/boot.rs` `clear_faults`, right after the remap in
`main`) — not the jump. Until the application re-remaps, its exceptions
still go through this bootloader's table, so a firmware that HardFaults in
its init window counts here: boot 1 faults → reset → boot 2 sees `FAULTS ==
1` and stays in update mode (amber, `1209:0002`) for the host to install a
fixed image, instead of an endless ~200 ms reset loop. **Update mode**
clears it as soon as it has answered its first `BOOT_INFO` (enumerated and
talking). A power cycle clears SRAM and starts the ladder over.

Walk-through of a repeatable fault in update mode: fault → count 1,
`REQ_BOOTLOADER` → update mode again → fault before any `BOOT_INFO` →
count 2, `REQ_ROM_DFU` → ROM DFU, recoverable with the combined image.

## Building

```sh
cd boot
cargo build --release --offline                    # production board
cargo build --release --offline --features proto   # pre-2026-07-28 prototypes
```

Dependencies are pinned with `=` to the versions in `fw/Cargo.lock`
(`boot/Cargo.lock` was seeded from it and pruned), so `cargo build --locked
--offline` works from the same registry cache. `.cargo/config.toml` sets
`thumbv6m-none-eabi` and `-Tlink.x` only (no defmt — that is why this crate
is not under `fw/`). Profile: `opt-level = "z"`, fat LTO, one codegen unit.

Size budget: the `.bin` must stay at or under `BOOT_SIZE - 512 = 24064`
bytes (`scripts/build-firmware.sh` asserts it). Measured for 1.0.0:
**22,864 bytes** production, 22,856 bytes `proto`; RAM `.data + .bss` =
4,696 bytes (executor arena 4 KiB included), leaving ~11.2 KiB of stack.

```sh
BIN="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin"
"$BIN/llvm-objcopy" -O binary target/thumbv6m-none-eabi/release/openmicro-boot boot.bin
xxd -l 0xE0 boot.bin     # word 0 = 0x20003FF0, word 1 = 0x080000E1, "OMKB" at 0xC0
```

Host tests (the protocol against a RAM model of the slot with F0 flash
rules — erase to 0xFF, no programming of a non-erased half-word):

```sh
cd boot/host-tests && cargo test --offline --target $(rustc -vV | sed -n 's/^host: //p')
```

## Flashing for bring-up

Over **SWD** (J2 on the bottom: P1/P2 GND, P3 SWCLK, P5 SWDIO; no reset
line, so the probe attaches to the running core):

```sh
cd boot && cargo run --release --offline        # probe-rs run --chip STM32F072CBTx
# or, without running:
probe-rs download --chip STM32F072CBTx --binary-format bin --base-address 0x08000000 boot.bin
```

Flashing only the bootloader leaves whatever is in the application slot;
with no valid header there the pad comes up in update mode (amber
breathing, 1209:0002) and the application is installed over HID.

The alternative is the **ROM DFU with the combined image**
(`dist/openmicro-fw-<ver>.bin` from `scripts/build-firmware.sh`: bootloader
padded to 24 KiB + stamped app): `ENTER_DFU` on the application's raw
interface, then `dfu-util -a 0 -s 0x08000000:leave -D <combined>.bin`. That
is the one-time migration path for pads running fw ≤ 0.9.0, and it carries
the old brick window once more — a resident bootloader that is already
installed makes it unnecessary. Never write an app-only slice through ROM
DFU at `0x08000000`; an app slice belongs at `-s 0x08006000`.

## Recovery

- **Application invalid or update interrupted**: nothing to do — the pad is
  in update mode; upload again.
- **Application faults during start-up** (before it owns its vector table):
  the next boot stays in update mode with the old image still reported by
  `BOOT_INFO`; install a fixed image, or power-cycle to try it once more.
- **Bootloader faulted once** in update mode: it reset straight back into
  update mode and carried on.
- **Bootloader faulted twice**: it degraded to the ROM DFU (0483:DF11).
  Reinstall the combined image with dfu-util or the host app.
- **Three faults**: keys solid red (if clocks were up), core halted in a
  `nop` loop with SWD alive. Attach probe-rs / a debugger through J2 and
  reflash; a power cycle clears the counter (SRAM) and tries again from 1.
- `ENTER_DFU` from either image still reaches the ROM DFU for bootloader
  self-update.

## Files

| file | role |
|---|---|
| `src/main.rs` | entry, hygiene, boot decision, jump, update mode, LED cue, fault escalation (panic, HardFault, DefaultHandler) |
| `src/update.rs` | protocol state machine over the `Programmer` trait — no embassy, host-testable; refuses unstamped and wrong-variant images |
| `src/flashprog.rs` | `Programmer` over `embassy_stm32::flash::Flash`, bounds-checked to the slot |
| `src/handoff.rs` | volatile access to the four handoff words |
| `src/ws2812.rs` | trimmed copy of `fw/src/ws2812.rs` (keep in sync) |
| `memory.x` | flash `0x08000000/0x6000`, RAM `0x200000C0/0x3F30`, `.boot_info` after the vector table |
| `host-tests/` | `cargo test` crate that includes `src/update.rs` via `#[path]` |
