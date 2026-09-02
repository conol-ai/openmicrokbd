# Releasing OpenMicro

The release workflow publishes both parts of the product from one GitHub
Release:

- notarized macOS DMGs for Apple Silicon and Intel;
- signed Sparkle appcasts for Apple Silicon and Intel;
- portable Windows ZIPs for Arm64 and x64;
- the production STM32F072 firmware as `.bin`, debug `.elf`, and factory
  programming `.hex` (same bytes as the `.bin` with the 0x08000000 load
  address embedded, for SWD/gang programmers at manufacturing);
- `release-manifest.json`, consumed by installed apps;
- `SHA256SUMS` and GitHub artifact attestations.

The host app checks the stable manifest at startup and every six hours. A newer
host version produces an update prompt. A Developer ID macOS release delegates
the signed download, atomic install, and relaunch to Sparkle; source/ad-hoc
macOS builds offer the verified-DMG manual fallback. Windows downloads and
verifies the matching portable ZIP, then opens it for manual replacement. A
newer device version produces a firmware prompt; the app uses the verified
firmware bundled in the app when it matches, or downloads and verifies the
release asset before flashing.

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

Commit both lockfiles whenever their manifests change.

## Required hardware gate

GitHub-hosted runners cannot validate USB ROM DFU. Before creating a tag, use
the exact production firmware build and verify this sequence on a real pad:

1. `ENTER_DFU` receives its HID acknowledgement.
2. The pad disconnects as `1209:0001` and enumerates as `0483:df11`.
3. Erase and programming complete without touching `0x0801f800`.
4. The pad re-enumerates as `1209:0001`.
5. The reported firmware version equals `fw/Cargo.toml`.
6. The saved keymap/profile still works.
7. Codex Micro compat mode round-trips: unplug, hold KEY 04 (the second key
   of the second row) while plugging in — the underglow blinks white and the
   pad enumerates as `303A:8360` "Codex Micro" (macOS may open Keyboard Setup
   Assistant for the new identity; dismiss it); with the Codex desktop app
   quit, `scripts/test-codex-compat.py` passes; with it running, its log
   (`~/Library/Logs/com.openai.codex/…`) shows `CodexMicroService` answering
   `device.status` with this firmware's version; the app still connects and its Settings toggle reads Codex Micro
   compat. Hold KEY 03 (the first key of that row) while plugging in (amber
   blink) to return to `1209:0001`.

Do not disconnect USB power while flashing. Because this design uses the STM32
ROM bootloader rather than a resident rollback bootloader, power loss during
erase/program may require SWD recovery through J2.

The updater refuses ambiguous DFU situations: more than one STM32 ROM DFU
device, or a normal OpenMicro alongside a generic DFU device.

## Local release build

CI uses Rust 1.92.0. Firmware and macOS release prerequisites are that
toolchain, `thumbv6m-none-eabi`, `llvm-tools-preview`, Xcode command-line tools,
and `jq`.

```sh
rustup target add thumbv6m-none-eabi
rustup component add llvm-tools-preview
scripts/build-firmware.sh dist
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
