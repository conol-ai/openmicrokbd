# Releasing OpenMicro

The release workflow publishes both parts of the product from one GitHub
Release:

- notarized macOS DMGs for Apple Silicon and Intel;
- signed Sparkle appcasts for Apple Silicon and Intel;
- portable Windows ZIPs for Arm64 and x64;
- the production STM32F072 firmware image `openmicro-fw-<ver>.bin` — the
  resident bootloader (first 24 KiB) and the application (from 0x08006000)
  in one file, flashable at 0x08000000 — with the factory programming `.hex`
  (same bytes with the load address embedded, for SWD/gang programmers at
  manufacturing), the application's debug `openmicro-fw-<ver>.elf`, and the
  bootloader's debug `openmicro-boot-<bootver>.elf` (neither ELF boots on
  its own: the app is linked behind the bootloader, the bootloader has no app);
- `release-manifest.json`, consumed by installed apps;
- `SHA256SUMS` and GitHub artifact attestations.

The host app checks the stable manifest at startup and every six hours. A newer
host version produces an update prompt. A Developer ID macOS release delegates
the signed download, atomic install, and relaunch to Sparkle; source/ad-hoc
macOS builds offer the verified-DMG manual fallback. Windows downloads and
verifies the matching portable ZIP, then opens it for manual replacement. A
newer device version produces a firmware prompt; the app uses the verified
firmware bundled in the app when it matches, or downloads and verifies the
release asset before flashing. Since firmware 0.10.0 the install goes through
the pad's own bootloader — a plain USB HID device, no driver on any OS — and
the pad validates the image (length + CRC-32) before it ever runs it; the
STM32 ROM DFU is used only once, to migrate a pad still on 0.9.0 or older
(see the hardware gate below).

## One-time GitHub setup

Add these Actions secrets:

| Secret | Value |
|---|---|
| `MACOS_DEVELOPER_ID_P12_BASE64` | Base64-encoded Developer ID Application certificate and private key exported as PKCS#12 |
| `MACOS_DEVELOPER_ID_P12_PASSWORD` | Password used for that `.p12` — must be non-empty, see below |
| `APPLE_API_KEY_ID` | App Store Connect API key ID |
| `APPLE_API_ISSUER_ID` | App Store Connect API issuer ID |
| `APPLE_API_PRIVATE_KEY_P8_BASE64` | Base64-encoded `AuthKey_<KEY_ID>.p8` |
| `SPARKLE_ED25519_PRIVATE_KEY` | Base64-encoded 32-byte Ed25519 seed matching `app/macos/sparkle-public-key.txt` |

Keychain Access exports a `.p12` with whatever password you type, including an
empty one — but the workflow treats an empty secret as "not configured" and
fails the signing step. If your export has no password, re-wrap it before
encoding:

```sh
openssl pkcs12 -in Certificates.p12 -passin pass: -nodes -out pair.pem
openssl pkcs12 -export -in pair.pem -passout pass:"$NEW_PASSWORD" -out signing.p12
rm -P pair.pem
```

Then store `base64 < signing.p12` and `$NEW_PASSWORD` together. Verify the pair
before relying on it — this must print a Developer ID Application identity:

```sh
security create-keychain -p probe probe.keychain-db
security unlock-keychain -p probe probe.keychain-db
security import signing.p12 -k probe.keychain-db -f pkcs12 -P "$NEW_PASSWORD" -A
security find-identity -v -p codesigning probe.keychain-db
security delete-keychain probe.keychain-db
```

The notary credentials can be checked without submitting anything:

```sh
xcrun notarytool history --key AuthKey_<KEY_ID>.p8 \
  --key-id <APPLE_API_KEY_ID> --issuer <APPLE_API_ISSUER_ID>
```

Create a GitHub environment named `release`, store or expose all six secrets
there, and add any desired reviewer/deployment-branch protections. The workflow
fails closed if any signing or notarization secret is missing; it will never
publish an ad-hoc-signed release. Restrict who may create `v*` tags as well.

Sparkle's update-signing identity is independent of the Apple certificate.
Keep the private seed backed up like any release credential; never commit it or
pass it on a command line. The public key is intentionally committed. CI
derives the public key from the protected secret and refuses to publish if it
does not match the key embedded in the app. For an intentional key rotation,
use the pinned Sparkle distribution's `bin/generate_keys`, update both the
protected secret and `app/macos/sparkle-public-key.txt`, then follow Sparkle's
key-rotation guidance before shipping.

Repository Actions must be allowed to create releases and attestations. The
workflow itself grants only the job-specific `contents: write`, `id-token:
write`, and `attestations: write` permissions.

## Versions

`app/Cargo.toml` is the GitHub Release version and must exactly match the tag:
app version `0.3.0` is released from tag `v0.3.0`.

Firmware has its own version in `fw/Cargo.toml`. It may differ from the host
version; both versions are recorded explicitly in the release manifest. Bump
the firmware version whenever the bytes or device behavior change. Do not reuse
a published version for different bytes.

The bootloader has its own version in `boot/Cargo.toml`, recorded in the
manifest as `firmware.bootloader_version` and in the info block at
`0x080000C0` on the pad. Bump it whenever the bootloader bytes change. A new
bootloader ships inside the combined `.bin` like everything else, but a pad
only picks it up over the ROM DFU path (`ENTER_DFU` + the combined image at
0x08000000), never over the normal HID update, so bootloader changes should
be rare and deliberate. The flash layout (`boot_size` 24576, `app_base`
134242304 = 0x08006000, `app_size` 86016) is fixed by `layout/src/lib.rs` and
published in the manifest as `firmware.layout`; changing it changes the link
address of every application image and is a new hardware-support line, not a
version bump.

Commit all lockfiles (`app/`, `fw/`, `boot/`, `layout/`, and the two
`host-tests` crates) whenever their manifests change; CI builds `--locked`.

## Required hardware gate

GitHub-hosted runners cannot exercise USB. Before creating a tag, run the host
gate (`scripts/test-firmware-host.sh`: the layout crate, both `host-tests`
suites, `fw-image.py selftest`, `test-bootloader.py selftest` — the same HID
client the steps below use, driven against an in-process model of the
bootloader — and the assembly dry run; CI runs the same script first), build
the exact production image (`scripts/build-firmware.sh dist`), quit the
OpenMicro app (it reacts to a pad in bootloader mode and would race the
scripts), and verify this sequence on a real pad. `scripts/test-bootloader.py`
needs `python3 -m pip install hidapi` for the hardware steps (the selftest does
not); on Linux it also needs the udev rules from
`docs/linux-firmware-updates.md`, including the `1209:0002` line.

1. **HID update round trip.**
   `python3 scripts/test-bootloader.py cycle dist/openmicro-fw-<ver>.bin`
   prints only PASS lines: the app acknowledges `ENTER_BOOT` (0x12), the pad
   re-enumerates as `1209:0002` "OpenMicro Bootloader" (keys breathing amber,
   underglow off), `BOOT_INFO` answers protocol 1 with the image's `app_base`,
   the application slice uploads with `UPDATE_BEGIN`/`UPDATE_DATA`/`UPDATE_END`
   all status 0 (the script prints the throughput; expect a few seconds for
   ~70 KB), `BOOT_INFO` then reports `app_valid 1` with the new version,
   `BOOT_RUN` brings the pad back as `1209:0001` within 15 s, and `VERSION`
   equals `fw/Cargo.toml`.
2. **Interrupted update recovery.**
   `python3 scripts/test-bootloader.py enter-boot`, then
   `python3 scripts/test-bootloader.py upload dist/openmicro-fw-<ver>.bin --abort-after 40`,
   then unplug and replug the pad. It must come back as `1209:0002` on its own
   (`info` shows `app_valid 0`: the half-written image fails the CRC), a plain
   `upload` must succeed, and `run` must start the new firmware. Do it once
   more pulling the cable during the erase (the ~1.5 s after `UPDATE_BEGIN`).
3. **Encoder-hold entry.** Hold the encoder switch while plugging in: the pad
   enumerates as the bootloader (`info` works, LEDs breathe amber) although
   the application is valid; `run` starts it. Plugging in without the hold
   boots straight into the application with no visible delay.
4. **`ENTER_DFU` from both images.** `python3 scripts/test-bootloader.py dfu`
   with the app running, and again with the bootloader running (after
   `enter-boot`): both times `0483:df11` appears in `dfu-util -l`. Return with
   the *combined* image —
   `dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave -D dist/openmicro-fw-<ver>.bin`
   — and the pad comes back as `1209:0001` with the same version. Never flash
   an application slice or an `.elf` at 0x08000000: that overwrites the
   bootloader.
5. **Settings survive.** After 1–4 the saved keymap/profile, the LED settings
   and the Codex-mode setting are unchanged: an update erases only
   `0x08006000..0x0801B000`, and `fw-image.py verify` refuses any image that
   would reach the file slots or the config page at `0x0801F800`.
6. Codex Micro compat mode round-trips (the bootloader is mode-agnostic; the
   identity chord is read by the application after the bootloader hands
   over): unplug, hold KEY 04 (the second key
   of the second row) while plugging in — the underglow blinks white and the
   pad enumerates as `303A:8360` "Codex Micro" (macOS may open Keyboard Setup
   Assistant for the new identity; dismiss it); with the Codex desktop app
   quit, `scripts/test-codex-compat.py` passes; with it running, its log
   (`~/Library/Logs/com.openai.codex/…`) shows `CodexMicroService` answering
   `device.status` with this firmware's version; the app still connects and its Settings toggle reads Codex Micro
   compat. Work Louder's Input app (if installed) shows the pad with its
   keymap loaded and no `fs.list` errors in `~/Library/Logs/input/main.log`.
   Hold KEY 03 (the first key of that row) while plugging in (amber blink)
   to return to `1209:0001`.
7. **App path.** With the OpenMicro app running, Install the built image from
   the firmware sheet: the update runs over the bootloader with no driver
   prompt (also on Windows), the app reconnects, and it shows the new version.

The updater refuses ambiguous situations: more than one pad in bootloader
mode, a bootloader-mode pad next to a ROM DFU device, or a pad next to a
generic DFU device; after `ENTER_BOOT` it only accepts a bootloader with the
serial of the pad it just rebooted.

### One-time migration from firmware 0.9.0 or older

A pad on 0.9.0 has no resident bootloader. The app (0.13.0 and later) notices
that `BOOT_INFO` goes unanswered and installs the combined image over the
STM32 ROM DFU exactly as earlier releases did — erase from 0x08000000,
program, leave. That single install still carries the old brick window (power
loss during it means SWD recovery on J2) and is the only step that needs the
WinUSB/Zadig binding on Windows or the USB 2.0 hub workaround on xHCI Linux
hosts (`docs/linux-firmware-updates.md`). Every later update goes through the
bootloader. For a release that changes the bootloader, verify the migration on
one pad: flash 0.9.0 (SWD or `dfu-util`), Install from the app, and confirm
`python3 scripts/test-bootloader.py info` shows the new bootloader version and
`app_valid 1`.

### When the bootloader itself faults

The bootloader never reset-loops and never sleeps in `WFI`: a panic or
HardFault is counted in the `FAULTS` handoff word (`0x20003FFC`; SRAM, so it
survives a system reset but not a power cycle) and the ladder escalates with
the count:

| where the fault happens | 1st | 2nd | 3rd and later |
|---|---|---|---|
| in update mode (USB up, protocol running) | reset back into update mode — the retry lands where the fault happened, not in the application | reset into the ROM DFU (`0483:df11`): reinstall the combined image at `0x08000000` | halt with the keys red if the clocks are up, spinning on `nop` so an SWD probe can attach on J2 |
| while the application is starting (between the jump and the app's own vector remap, where faults still vector through the bootloader's table) | reset; the next boot lands in **update mode** instead of jumping to the same image a third time, so the host can install a different firmware over HID | — | — |

The application clears the counter once it is running (right after its
vector-table remap), and update mode clears it after answering its first
`BOOT_INFO`, so a transient never accumulates across good boots. If a pad ever
shows up in bootloader mode with a valid application and nothing asked for
it, suspect a start-up fault in that firmware build before suspecting the
bootloader. During the hardware gate, a pad that comes back as `1209:0001`
after `ENTER_BOOT` instead of `1209:0002` is a bootloader-mode fault: the app
reports it as such and points at the ROM DFU reinstall.

### Size budget

`scripts/build-firmware.sh` prints the layout after every build and fails when
a limit is crossed; quote the free bytes in the release notes:

| region | limit | note |
|---|---|---|
| bootloader `.bin` | 24,064 bytes (the 24 KiB slot minus 512) | the headroom exists so a bootloader fix never has to move `APP_BASE`, which is baked into every application image |
| application image | 86,016 bytes (84 KiB, 0x08006000..0x0801B000) | 0.10.0 needs 69,696 bytes with `DEFMT_LOG=off` (what every release build and `build-firmware.sh` use). Dev builds (`cargo run` over SWD) take `DEFMT_LOG` from `fw/.cargo/config.toml`, whose default is now `warn,openmicro_fw=info`: 85,264 bytes on 0.10.0, 752 bytes under the slot. The previous `info,openmicro_fw=debug` default linked 85,912 bytes with 104 bytes to spare; opt into it per invocation (`DEFMT_LOG=info,openmicro_fw=debug cargo build --release`) and expect it to stop fitting first. A `section .text will not fit in region FLASH` error on a dev build means the log level, not the release image, is over budget — lower it (or use `off`) before touching code |
| combined `.bin` | must end below 0x0801B000 | the file slots and the config page above are never part of an image |

## Local release build

CI uses Rust 1.92.0. Firmware and macOS release prerequisites are that
toolchain, `thumbv6m-none-eabi`, `llvm-tools-preview`, Xcode command-line tools,
`python3` (3.9 or later, standard library only) and `jq`.

```sh
rustup target add thumbv6m-none-eabi
rustup component add llvm-tools-preview
scripts/test-firmware-host.sh    # layout, fw + boot host tests, fw-image.py + test-bootloader.py selftests, assembly dry run
scripts/build-firmware.sh dist   # boot/ + fw/ -> dist/openmicro-fw-<firmware-version>.bin, .hex, both ELFs
scripts/package-macos.sh dist dist/openmicro-fw-<firmware-version>.bin

# Optional cross-build of the Intel DMG from an Apple Silicon Mac:
rustup target add x86_64-apple-darwin
OPENMICRO_MACOS_ARCH=x86_64 \
  scripts/package-macos.sh dist dist/openmicro-fw-<firmware-version>.bin
```

For Windows, install the same Rust toolchain and Visual Studio's **Desktop
development with C++** workload. Native Arm64 builds also need the Visual
Studio LLVM/Clang component. From PowerShell, after producing or downloading
the firmware binary:

```powershell
rustup target add x86_64-pc-windows-msvc
./scripts/package-windows.ps1 `
  -OutputDir dist/windows `
  -FirmwareBin dist/openmicro-fw-<firmware-version>.bin `
  -Target x86_64-pc-windows-msvc

# For Windows on Arm, change both occurrences of x86_64 to aarch64.
```

The Windows script uses the static Visual C++ runtime and stages the executable,
firmware, manifest, notices, and operating instructions into one portable ZIP.
It does not currently Authenticode-sign the executable, so a downloaded public
build can show a Windows reputation warning until release signing is configured.

The packaging script downloads Sparkle 2.9.6 from its official release,
verifies the pinned SHA-256, and embeds the framework while preserving its
symlinks. Without `MACOS_SIGN_IDENTITY`, the local script deliberately packages
an ad-hoc-signed test app with self-installation disabled. CI sets
`REQUIRE_SIGNING=1`, signs Sparkle's nested helpers and the app from the inside
out with Developer ID, submits the final DMG to Apple, staples the ticket, and
validates it.

Useful local checks:

```sh
cargo test --manifest-path app/Cargo.toml --locked --lib
scripts/test-firmware-host.sh
python3 scripts/fw-image.py info dist/openmicro-fw-<firmware-version>.bin
python3 scripts/fw-image.py verify dist/openmicro-fw-<firmware-version>.bin
hdiutil verify dist/OpenMicro-<app-version>-macos-<arch>.dmg
codesign --verify --deep --strict dist/macos-<arch>/OpenMicro.app
otool -L dist/macos-<arch>/OpenMicro.app/Contents/MacOS/OpenMicro
```

```powershell
cargo test --manifest-path app/Cargo.toml --locked --all-targets --target x86_64-pc-windows-msvc
tar -tf dist/windows/OpenMicro-<app-version>-windows-x86_64.zip
```

## Publish

After tests, review, and the hardware gate:

```sh
git tag -s v<app-version> -m "OpenMicro v<app-version>"
git push origin v<app-version>
```

Only a pushed `vX.Y.Z` tag can publish. CI builds and verifies every artifact
first, signs each final notarized DMG with Sparkle's Ed25519 key, generates and
verifies one signed appcast per architecture, builds both Windows packages,
then creates a draft, uploads the complete set, and makes it public only after
the upload succeeds. If signing, notarization, packaging, appcast generation,
checksums, or manifest generation fails, no new public release appears and
installed apps continue seeing the last successful stable release.

After publication, verify both DMGs and both Windows ZIPs on the GitHub Release
page and:

```text
https://github.com/conol-ai/openmicrokbd/releases/latest/download/release-manifest.json
https://github.com/conol-ai/openmicrokbd/releases/latest/download/appcast-aarch64.xml
https://github.com/conol-ai/openmicrokbd/releases/latest/download/appcast-x86_64.xml
```

Then launch the previous public app version and confirm both update prompts.
The first release containing Sparkle is the bootstrap: older apps reach it via
the manual DMG path; later releases install and relaunch entirely in-app.
On Windows, confirm the previous version downloads the correct architecture,
rejects a modified ZIP, and opens the verified package.
