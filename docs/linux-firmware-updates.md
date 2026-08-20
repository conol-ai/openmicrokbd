# Linux firmware updates

This document separates the STM32 ROM-DFU transport failure observed on Linux
from the Linux packaging and permissions work that the project still needs. The
two problems affect the same screen, but they have different causes and fixes.

## Confirmed failure signature

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

Use this recovery sequence on an affected Linux host:

1. If the updater has already reported failure, close it. Do not interrupt a
   command that is still actively programming.
2. Power-cycle the pad and confirm that it returns as `1209:0001`. If it remains
   in ROM DFU, recovery can continue from that mode.
3. Connect the pad through a USB 2.0 hub. A monitor hub or dock may also work;
   a hub with a distinct USB 2.0 transaction path is preferred.
4. Start the app, enter DFU, and install the firmware. Alternatively, after
   confirming the exact target with `dfu-util -l`, use:

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
5. Keep the pad powered until the download completes and it re-enumerates as
   `1209:0001`. Verify the reported firmware version in the app.

Changing udev permissions, running the updater as root, or changing the DfuSe
poll timeout does not correct this host-controller timing defect. A short delay
between separate control requests may be tested as a mitigation, but user-space
software cannot control packet spacing inside a USB control transfer; it must
not replace the documented hub workaround.

## Linux USB permissions

Linux installations need permission for both identities used during an update:

```udev
# /etc/udev/rules.d/70-openmicro.rules
KERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1209", ATTRS{idProduct}=="0001", MODE="0660", TAG+="uaccess"
SUBSYSTEM=="usb", ATTR{idVendor}=="0483", ATTR{idProduct}=="df11", MODE="0660", TAG+="uaccess"
```

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

### P2: remove dependence on the affected ROM updater

For future hardware/software revisions, consider a small project-controlled
USB bootloader in user flash. It can implement pacing and recovery appropriate
for OpenMicro and avoid the immutable STM32 ROM USB limitation. This changes
the flash layout and recovery model and therefore needs its own design review.
The initial installation still requires one successful ROM-DFU or SWD write.

## Release gate

Before publishing a release, test the exact production image on Linux and
macOS. Linux testing must cover direct xHCI attachment and attachment through a
USB 2.0 hub. Record the USB topology, verify that the reserved keymap page is
not touched, and confirm that the pad returns with the expected firmware and
saved profile. A GitHub-hosted runner cannot replace this physical-device gate.
