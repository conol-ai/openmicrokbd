#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
OUTPUT_DIR="${1:-$REPO_ROOT/dist}"
# FW_FEATURES=proto builds for the prototype board (pre-v23 pin map); the
# feature list is appended to the artifact names so revisions can't be mixed up.
FW_FEATURES="${FW_FEATURES:-}"
VARIANT="${FW_FEATURES:+-${FW_FEATURES//,/-}}"

mkdir -p "$OUTPUT_DIR"

FW_VERSION="$(
    cargo metadata \
        --manifest-path "$REPO_ROOT/fw/Cargo.toml" \
        --locked \
        --no-deps \
        --format-version 1 |
        jq -r '.packages[] | select(.name == "openmicro-fw") | .version'
)"
if [[ -z "$FW_VERSION" || "$FW_VERSION" == "null" ]]; then
    echo "could not read firmware version" >&2
    exit 1
fi

(
    cd "$REPO_ROOT/fw"
    DEFMT_LOG=off cargo build --release --locked ${FW_FEATURES:+--features "$FW_FEATURES"}
)

ELF="$REPO_ROOT/fw/target/thumbv6m-none-eabi/release/openmicro-fw"
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
LLVM_TOOLS_DIR="$(rustc --print sysroot)/lib/rustlib/$HOST_TRIPLE/bin"
RUST_OBJCOPY="$LLVM_TOOLS_DIR/llvm-objcopy"
if [[ ! -x "$RUST_OBJCOPY" && -x "$LLVM_TOOLS_DIR/rust-objcopy" ]]; then
    RUST_OBJCOPY="$LLVM_TOOLS_DIR/rust-objcopy"
fi
if [[ -n "${OBJCOPY:-}" ]]; then
    RUST_OBJCOPY="$OBJCOPY"
fi
if [[ ! -x "$RUST_OBJCOPY" ]]; then
    echo "LLVM objcopy not found; run: rustup component add llvm-tools-preview" >&2
    exit 1
fi

BIN="$OUTPUT_DIR/openmicro-fw-$FW_VERSION$VARIANT.bin"
DEBUG_ELF="$OUTPUT_DIR/openmicro-fw-$FW_VERSION$VARIANT.elf"
"$RUST_OBJCOPY" -O binary "$ELF" "$BIN"
cp "$ELF" "$DEBUG_ELF"

BIN_SIZE="$(wc -c < "$BIN" | tr -d ' ')"
if (( BIN_SIZE < 192 || BIN_SIZE > 129024 )); then
    echo "firmware size $BIN_SIZE is outside the safe 192..129024-byte range" >&2
    exit 1
fi

INITIAL_SP="$(od -An -tu4 -N4 "$BIN" | tr -d ' ')"
RESET_VECTOR="$(od -An -tu4 -j4 -N4 "$BIN" | tr -d ' ')"
RESET_ADDRESS=$((RESET_VECTOR & 0xfffffffe))
if (( INITIAL_SP < 0x20000000 || INITIAL_SP > 0x20004000 || INITIAL_SP % 4 != 0 )); then
    printf 'invalid initial stack pointer in firmware vector table: 0x%08x\n' "$INITIAL_SP" >&2
    exit 1
fi
if (( RESET_VECTOR % 2 != 1 || RESET_ADDRESS < 0x08000000 || RESET_ADDRESS >= 0x0801f800 )); then
    printf 'invalid reset vector in firmware vector table: 0x%08x\n' "$RESET_VECTOR" >&2
    exit 1
fi

echo "Firmware $FW_VERSION: $BIN ($BIN_SIZE bytes)"
printf 'Vectors: SP=0x%08x reset=0x%08x\n' "$INITIAL_SP" "$RESET_VECTOR"
shasum -a 256 "$BIN" "$DEBUG_ELF"
