# Releasing OpenMicro

The release workflow publishes both parts of the product from one GitHub
Release:

- notarized macOS DMGs for Apple Silicon and Intel;
- the production STM32F072 firmware as `.bin`, debug `.elf`, and factory
  programming `.hex` (same bytes as the `.bin` with the 0x08000000 load
  address embedded, for SWD/gang programmers at manufacturing);
- `release-manifest.json`, consumed by installed apps;
- `SHA256SUMS` and GitHub artifact attestations.

The host app checks the stable manifest at startup and every six hours. A newer
host version produces a download prompt; the app verifies and opens the correct
DMG, and the user replaces OpenMicro in Applications. A newer device version
produces a firmware prompt; the app uses the verified firmware bundled in the
app when it matches, or downloads and verifies the release asset before
flashing.

## One-time GitHub setup

Add these Actions secrets:

| Secret | Value |
|---|---|
| `MACOS_DEVELOPER_ID_P12_BASE64` | Base64-encoded Developer ID Application certificate and private key exported as PKCS#12 |
| `MACOS_DEVELOPER_ID_P12_PASSWORD` | Password used for that `.p12` |
| `APPLE_API_KEY_ID` | App Store Connect API key ID |
| `APPLE_API_ISSUER_ID` | App Store Connect API issuer ID |
| `APPLE_API_PRIVATE_KEY_P8_BASE64` | Base64-encoded `AuthKey_<KEY_ID>.p8` |

Create a GitHub environment named `release`, store or expose the five secrets
there, and add any desired reviewer/deployment-branch protections. The workflow
fails closed if any signing or notarization secret is missing; it will never
publish an ad-hoc-signed release. Restrict who may create `v*` tags as well.

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

Do not disconnect USB power while flashing. Because this design uses the STM32
ROM bootloader rather than a resident rollback bootloader, power loss during
erase/program may require SWD recovery through J2.

The updater refuses ambiguous DFU situations: more than one STM32 ROM DFU
device, or a normal OpenMicro alongside a generic DFU device.

## Local release build

CI uses Rust 1.92.0. Local prerequisites are that toolchain,
`thumbv6m-none-eabi`, `llvm-tools-preview`, Xcode command-line tools, and `jq`.

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

Without `MACOS_SIGN_IDENTITY`, the local script deliberately packages an
ad-hoc-signed test app. CI sets `REQUIRE_SIGNING=1`, signs with Developer ID,
submits the final DMG to Apple, staples the ticket, and validates it.

Useful local checks:

```sh
cargo test --manifest-path app/Cargo.toml --locked --lib
hdiutil verify dist/OpenMicro-<app-version>-macos-<arch>.dmg
codesign --verify --deep --strict dist/macos-<arch>/OpenMicro.app
```

## Publish

After tests, review, and the hardware gate:

```sh
git tag -s v<app-version> -m "OpenMicro v<app-version>"
git push origin v<app-version>
```

Only a pushed `vX.Y.Z` tag can publish. CI builds and verifies every artifact
first, then creates a draft, uploads the complete set, and makes it public only
after the upload succeeds. If signing, notarization, packaging, checksums, or
manifest generation fails, no new public release appears and installed apps
continue seeing the last successful stable release.

After publication, verify both DMG downloads on the GitHub Release page and:

```text
https://github.com/conol-ai/openmicrokbd/releases/latest/download/release-manifest.json
```

Then launch the previous public app version and confirm both update prompts.
