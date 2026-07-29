#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
OUTPUT_DIR="${1:-$REPO_ROOT/dist}"
FIRMWARE_BIN="${2:-}"

case "$OUTPUT_DIR" in
    "" | "/")
        echo "refusing unsafe output directory: $OUTPUT_DIR" >&2
        exit 1
        ;;
esac

REQUESTED_ARCH="${OPENMICRO_MACOS_ARCH:-$(uname -m)}"
case "$REQUESTED_ARCH" in
    arm64 | aarch64)
        ARCH="aarch64"
        RUST_TARGET="aarch64-apple-darwin"
        ;;
    x86_64)
        ARCH="x86_64"
        RUST_TARGET="x86_64-apple-darwin"
        ;;
    *)
        echo "unsupported macOS architecture: $REQUESTED_ARCH" >&2
        exit 1
        ;;
esac

APP_VERSION="$(
    cargo metadata \
        --manifest-path "$REPO_ROOT/app/Cargo.toml" \
        --locked \
        --no-deps \
        --format-version 1 |
        jq -r '.packages[] | select(.name == "openmicro-app") | .version'
)"
FW_VERSION="$(
    cargo metadata \
        --manifest-path "$REPO_ROOT/fw/Cargo.toml" \
        --locked \
        --no-deps \
        --format-version 1 |
        jq -r '.packages[] | select(.name == "openmicro-fw") | .version'
)"

STAGE="$OUTPUT_DIR/macos-$ARCH"
APP_BUNDLE="$STAGE/OpenMicro.app"
CONTENTS="$APP_BUNDLE/Contents"
RESOURCES="$CONTENTS/Resources"
DMG_ROOT="$STAGE/dmg-root"
DMG="$OUTPUT_DIR/OpenMicro-$APP_VERSION-macos-$ARCH.dmg"

rm -rf "$STAGE"
mkdir -p "$CONTENTS/MacOS" "$RESOURCES" "$DMG_ROOT"

(
    cd "$REPO_ROOT/app"
    HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
    if [[ "$RUST_TARGET" != "$HOST_TRIPLE" ]]; then
        # Makepad 1.0 compiles one Objective-C helper without forwarding
        # Cargo's target architecture. The wrapper makes cross-built DMGs
        # contain a consistently targeted helper object.
        export PATH="$REPO_ROOT/scripts/macos-cross-tools:$PATH"
    fi
    MACOSX_DEPLOYMENT_TARGET=11.0 \
    MAKEPAD="apple_bundle,target_$ARCH" \
    MAKEPAD_PACKAGE_DIR=. \
    OPENMICRO_FIRMWARE_VERSION="$FW_VERSION" \
        cargo build --release --locked --target "$RUST_TARGET"
)

cp "$REPO_ROOT/app/target/$RUST_TARGET/release/openmicro-app" "$CONTENTS/MacOS/OpenMicro"
chmod 755 "$CONTENTS/MacOS/OpenMicro"
sed "s/@APP_VERSION@/$APP_VERSION/g" \
    "$REPO_ROOT/app/macos/Info.plist.in" > "$CONTENTS/Info.plist"
printf 'APPL????' > "$CONTENTS/PkgInfo"

# Makepad resolves crate:// URLs from NSBundle.resourcePath when built with
# MAKEPAD=apple_bundle. Mirror every Cargo package's resources directory under
# its normalized Rust crate name, including the root openmicro_app package.
METADATA_JSON="$STAGE/cargo-metadata.json"
cargo metadata \
    --manifest-path "$REPO_ROOT/app/Cargo.toml" \
    --locked \
    --format-version 1 \
    > "$METADATA_JSON"
while IFS=$'\t' read -r PACKAGE_NAME MANIFEST_PATH; do
    PACKAGE_DIR="$(dirname "$MANIFEST_PATH")"
    SOURCE_RESOURCES="$PACKAGE_DIR/resources"
    if [[ ! -d "$SOURCE_RESOURCES" ]]; then
        continue
    fi
    CRATE_NAME="${PACKAGE_NAME//-/_}"
    DESTINATION="$RESOURCES/$CRATE_NAME/resources"
    mkdir -p "$(dirname "$DESTINATION")"
    ditto "$SOURCE_RESOURCES" "$DESTINATION"
done < <(jq -r '.packages[] | [.name, .manifest_path] | @tsv' "$METADATA_JSON")

if [[ -n "$FIRMWARE_BIN" ]]; then
    if [[ ! -f "$FIRMWARE_BIN" ]]; then
        echo "firmware image does not exist: $FIRMWARE_BIN" >&2
        exit 1
    fi
    mkdir -p "$RESOURCES/firmware"
    cp "$FIRMWARE_BIN" "$RESOURCES/firmware/openmicro-fw.bin"
    FW_SHA256="$(shasum -a 256 "$FIRMWARE_BIN" | awk '{print $1}')"
    jq -n \
        --arg version "$FW_VERSION" \
        --arg sha256 "$FW_SHA256" \
        '{version: $version, sha256: $sha256}' \
        > "$RESOURCES/firmware/manifest.json"
fi

plutil -lint "$CONTENTS/Info.plist"
file "$CONTENTS/MacOS/OpenMicro"
case "$ARCH" in
    aarch64) LIPO_ARCH="arm64" ;;
    x86_64) LIPO_ARCH="x86_64" ;;
esac
lipo "$CONTENTS/MacOS/OpenMicro" -verify_arch "$LIPO_ARCH"

if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
    codesign \
        --force \
        --options runtime \
        --timestamp \
        --sign "$MACOS_SIGN_IDENTITY" \
        "$APP_BUNDLE"
else
    if [[ "${REQUIRE_SIGNING:-0}" == "1" ]]; then
        echo "MACOS_SIGN_IDENTITY is required for a release build" >&2
        exit 1
    fi
    echo "MACOS_SIGN_IDENTITY is unset; packaging a locally ad-hoc-signed app" >&2
    codesign --force --sign - "$APP_BUNDLE"
fi
codesign --verify --deep --strict --verbose=2 "$APP_BUNDLE"

ditto "$APP_BUNDLE" "$DMG_ROOT/OpenMicro.app"
ln -s /Applications "$DMG_ROOT/Applications"
rm -f "$DMG"
hdiutil create \
    -volname "OpenMicro" \
    -srcfolder "$DMG_ROOT" \
    -format UDZO \
    -ov \
    "$DMG"

if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
    codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" "$DMG"
    codesign --verify --strict --verbose=2 "$DMG"
fi
hdiutil verify "$DMG"
shasum -a 256 "$DMG"
echo "DMG: $DMG"
