# Linux firmware updates

This document separates the STM32 ROM-DFU transport failure observed on Linux
from the Linux packaging and permissions work that the project still needs. The
two problems affect the same screen, but they have different causes and fixes.

## Firmware 0.10.0 and later: the resident bootloader

Since firmware 0.10.0 the pad carries its own bootloader in the first 24 KiB of
flash (`boot/`). Normal updates no longer touch the STM32 ROM DFU at all: the
app sends `ENTER_BOOT`, the pad re-enumerates as a plain HID device
(`1209:0002` "OpenMicro Bootloader"), the application image is sent in 64-byte
reports, and the bootloader checks its length and CRC-32 before it runs it. An
interrupted update leaves the pad in bootloader mode, never dead. No DFU
control transfer is involved on that path, so the xHCI `dfuDNBUSY` defect
described below cannot occur there; it only matters for the one-time migration
of a pad still on 0.9.0 or older (which the app performs over ROM DFU with the
combined image) and for recovery.

What the published files are, and where they go:

| file | contents | flash it with |
|---|---|---|
| `openmicro-fw-<ver>.bin` | bootloader + application, one image | ROM DFU / SWD at `0x08000000` (migration, recovery, factory); the app's normal update reads the application part out of it |
| `openmicro-fw-<ver>.hex` | the same bytes with the load address embedded | SWD / gang programmers |
| `openmicro-fw-<ver>-app-slot-0x08006000.bin` | application only, cut out locally with `python3 scripts/fw-image.py slice` (never published) | `dfu-util -s 0x08006000` only (no `:leave`; replug afterwards so the bootloader validates and starts it), or `scripts/test-bootloader.py upload` |

Never write an application-only image (a slice, or an `.elf` converted by
hand) at `0x08000000`: it overwrites the bootloader and the pad no longer
starts (SWD recovery through J2). The app refuses to do this; `dfu-util` does
not.

Manual update over the bootloader without the app (also the release gate's
round trip; see `RELEASING.md`):

```sh
python3 -m pip install hidapi
python3 scripts/test-bootloader.py cycle openmicro-fw-<version>.bin
```

## Confirmed failure signature (ROM DFU path)

The affected OpenMicro pad has completed a firmware update on macOS through the
app's USB DfuSe backend. It was not programmed through SWD, J2, ST-Link, or
`probe-rs`. On the affected Linux host, the same physical pad behaves as
follows:

- normal mode `1209:0001` enumerates and accepts the command to enter DFU;
- ROM DFU `0483:df11` enumerates and is accessible to the desktop user;
- `GETSTATUS`, `GETCOMMANDS`, and `UPLOAD` control requests work;
- both the OpenMicro updater and `dfu-util` accept a `DNLOAD` request, then stay
  in `dfuDNBUSY` indefinitely;
- even the non-destructive DfuSe Set Address command exhibits the failure;
- after a failed attempt and a power cycle, the existing application can still
  enumerate as `1209:0001` when its flash image remains intact.

This evidence rules out a missing Linux DFU implementation, a corrupt firmware
image, and ordinary USB permission failure as the direct cause of this incident.
The v1 board does have a documented `JOY_SW`/PA15 footprint defect, and PA15 is
used as USART2_RX by the STM32F071/072 ROM bootloader. That defect is worth
fixing, but it is not sufficient to explain this failure: the same board has
completed the same ROM-DFU update on macOS.

## Primary cause and field workaround

ST documents a USB limitation in the STM32F071/072 V10.1 system-memory
bootloader. On some machines with a high-speed USB host controller, the device
is detected but data transactions fail because the controller's inter-packet
delay is too short for the interrupt-driven ROM bootloader. The documented
workaround is to place a USB hub between the host and MCU so that transaction
timing is relaxed.

Source: [AN2606, STM32 microcontroller system memory boot mode](https://www.st.com/resource/en/application_note/an2606-stm32microcontroller-system-memory-boot-mode-stmicroelectronics.pdf), section "STM32F071xx/072xx devices", bootloader version V10.1 known limitations.

Use this recovery sequence on an affected Linux host. It only applies while
the ROM DFU is in use: migrating a pad from firmware 0.9.0 or older, or
recovering one whose bootloader is gone. A pad on 0.10.0 or later updates over
its own bootloader and never enters ROM DFU unless asked (`ENTER_DFU`).

1. If the updater has already reported failure, close it. Do not interrupt a
   command that is still actively programming.
2. Power-cycle the pad and confirm that it returns as `1209:0001` (or, on
   0.10.0+, as `1209:0002` if only the application is missing — then the
   bootloader is intact and a plain HID update repairs it; no DFU needed). If
   it remains in ROM DFU, recovery can continue from that mode.
3. Connect the pad through a USB 2.0 hub. A monitor hub or dock may also work;
   a hub with a distinct USB 2.0 transaction path is preferred.
4. Start the app, enter DFU, and install the firmware. Alternatively, after
   confirming the exact target with `dfu-util -l`, flash the **combined**
   image (bootloader + application) at the start of flash:

   ```sh
   dfu-util \
     -d 0483:df11 \
     -p <bus-port-path> \
     -S <dfu-serial> \
     -a 0 \
     -s 0x08000000:leave \
     -D openmicro-fw-<version>.bin
   ```

   `-a 0` selects internal flash. Never write alt setting 1 (Option Bytes).
   To reprogram only the application slot (leaving the bootloader alone), cut
   the slice out first and address it explicitly — the slice is never valid
   at `0x08000000` — and do **not** add `:leave` to a slice:

   ```sh
   python3 scripts/fw-image.py slice openmicro-fw-<version>.bin \
     --out /tmp/openmicro-fw-<version>-app-slot-0x08006000.bin
   dfu-util -d 0483:df11 -a 0 -s 0x08006000 \
     -D /tmp/openmicro-fw-<version>-app-slot-0x08006000.bin
   ```

   then unplug and replug the pad: the bootloader validates the slot and
   starts it. With `:leave` the ROM would jump straight to `0x08006000`,
   skipping the bootloader's validation and its reset hygiene (memory remap,
   SysTick), and the freshly written application can appear not to start
   until the next power cycle anyway. `:leave` is only right for the
   combined image at `0x08000000`, where the jump lands in the bootloader.
5. Keep the pad powered until the download completes and it re-enumerates as
   `1209:0001`. Verify the reported firmware version in the app.

Changing udev permissions, running the updater as root, or changing the DfuSe
poll timeout does not correct this host-controller timing defect. A short delay
between separate control requests may be tested as a mitigation, but user-space
software cannot control packet spacing inside a USB control transfer; it must
not replace the documented hub workaround.

## Linux USB permissions

Linux installations need permission for every identity the pad can take during
an update:

```udev
# /etc/udev/rules.d/70-openmicro.rules
KERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1209", ATTRS{idProduct}=="0001", MODE="0660", TAG+="uaccess"
# Codex Micro compat mode (fw 0.8.0+): the same pad under its other identity
KERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{idVendor}=="303a", ATTRS{idProduct}=="8360", MODE="0660", TAG+="uaccess"
# Bootloader mode (fw 0.10.0+): every normal firmware update goes through this device
KERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1209", ATTRS{idProduct}=="0002", MODE="0660", TAG+="uaccess"
# STM32 ROM DFU: only the one-time bootloader install (pads on fw <= 0.9.0) and recovery
SUBSYSTEM=="usb", ATTR{idVendor}=="0483", ATTR{idProduct}=="df11", MODE="0660", TAG+="uaccess"
```

Without the `1209:0002` rule the bootloader enumerates but cannot be opened;
the app and `scripts/test-bootloader.py` then report the open error (and this
file) rather than waiting for a device that is already there.

After installing or changing the file:

```sh
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Reconnect the pad. Permission failures normally appear while opening hidraw or
claiming the DFU interface; they are different from a command that was accepted
and then remains in `dfuDNBUSY`.

## Project implementation plan

### P0: make DFU failures safe and diagnosable

- Preserve the actual HID open error instead of converting
  `api.open_path(&path)` to `None`. Distinguish not found, permission denied,
  busy, and disconnect/re-enumeration errors in the UI and logs.
- Log every relevant DFU transition: request/phase, `bStatus`, `bState`, and
  `bwPollTimeout`.
- Before erasing, perform a non-destructive Set Address preflight. If it does
  not leave `dfuDNBUSY` within a short deadline, stop without touching flash and
  recommend a USB 2.0 hub.
- Treat recovery and an active operation separately. `ABORT` is not a valid
  escape from every state. Do not repeatedly issue recovery requests while a
  legitimate erase or program operation may still be running.
- Remove the unconditional `recovery: SWD on J2` suffix. Recommend SWD only
  when the application no longer starts and ROM DFU cannot be recovered.

Acceptance criteria:

- a permission failure is never displayed as `device not found`;
- the Linux timing failure is detected before the first erase;
- logs contain enough state to distinguish `dfuIDLE`, `dfuDNBUSY`, and
  `dfuERROR` without an external diagnostic program;
- the UI gives the USB 2.0 hub workaround for the known STM32 failure.

### P1: ship a supported Linux application

- Add `packaging/linux/70-openmicro.rules` and install it from Linux packages.
- Add a Linux CI job that produces at least an x86-64 tar archive containing
  the GPUI binary, desktop metadata/icon, udev rules, firmware image, and
  adjacent firmware manifest. AppImage or distro-native packages can follow.
- Generalize `release::bundled_firmware()` to search, in order:
  1. an explicit `OPENMICRO_FIRMWARE_DIR` override;
  2. macOS `Contents/Resources/firmware`;
  3. a Linux executable-relative `../share/openmicro/firmware` directory;
  4. `/usr/share/openmicro/firmware`;
  5. the verified release catalog download.
- Test that the packaged firmware bytes match the version, size, and SHA-256 in
  the adjacent manifest.

Acceptance criteria:

- a fresh Linux desktop installation can open both normal HID and ROM DFU as a
  logged-in user without running the app as root;
- the installed app can offer its bundled firmware while offline;
- CI installs the archive in a clean environment and verifies that all runtime
  resources resolve from their installed locations.

### P1: publish a coherent firmware release

- Publish firmware 0.7.0 (or the next intentionally selected version) in the
  release catalog instead of leaving the public manifest at 0.6.0.
- Decide which GitHub repository owns official releases. The app currently
  reads the manifest from `conol-ai/openmicrokbd`, while active development may
  be published from another fork. Keep the workflow asset URLs and
  `MANIFEST_URL` on the same authority.
- Include Linux artifacts in the release manifest without changing firmware
  board/protocol validation.

Acceptance criteria:

- the tagged firmware version, firmware binary, manifest version, checksum,
  and App UI agree;
- macOS and Linux builds consume the same immutable firmware artifact produced
  by the firmware CI job.

### P2: remove dependence on the affected ROM updater — shipped in firmware 0.10.0

The resident bootloader in `boot/` (see `fw/README.md` and `boot/README.md`)
replaces the ROM updater for every normal update: HID reports the bootloader
paces itself, a length + CRC-32 check before every start, and an interrupted
update that ends in bootloader mode instead of a dead pad. What remains on ROM
DFU is the one-time migration of pads on 0.9.0 or older and the recovery
path, both with the combined image at `0x08000000`. The P0 items above still
apply to that path; they are no longer on the critical path of ordinary
updates.

## Release gate

Before publishing a release, test the exact production image on Linux and
macOS: the bootloader round trip (`python3 scripts/test-bootloader.py cycle`,
see `RELEASING.md`) with the udev rules above installed and nothing running as
root, and — for any release that changes the bootloader — the one-time ROM DFU
migration from 0.9.0 on Linux both directly on xHCI and through a USB 2.0 hub.
Record the USB topology, verify that the reserved keymap page is not touched,
and confirm that the pad returns with the expected firmware and saved profile.
A GitHub-hosted runner cannot replace this physical-device gate.
