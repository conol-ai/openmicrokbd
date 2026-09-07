#!/usr/bin/env bash
# Builds the OpenMicro v1 firmware release artifacts into OUTPUT_DIR:
#
#   openmicro-fw-<ver>[-<features>].bin    bootloader + application, one image
#                                          flashable at 0x08000000 (ROM DFU, SWD,
#                                          or the app's one-time migration path)
#   openmicro-fw-<ver>[-<features>].hex    the same bytes as Intel HEX with the
#                                          load address embedded, for the factory
#   openmicro-fw-<ver>[-<features>].elf    application ELF with debug info
#                                          (linked at 0x08006000; does not boot alone)
#   openmicro-boot-<bootver>[-<features>].elf  bootloader ELF with debug info
#
# Since fw 0.10.0 the pad only starts an application whose header carries the
# right length and CRC-32 (boot/ validates it before every jump), so this script
# is the only path that produces publishable bytes: build boot/, check it and
# pad it to its 24 KiB slot, build fw/, stamp its header with fw-image.py, glue
# the two together and verify the result exactly the way the bootloader will.
# An application-only .bin is never written into OUTPUT_DIR: flashed at
# 0x08000000 it would overwrite the bootloader (the temp dir holds it and is
# removed on exit).
#
#   scripts/build-firmware.sh [OUTPUT_DIR]        default <repo>/dist
#   FW_FEATURES=proto scripts/build-firmware.sh   prototype pin map, passed to
#                                                 BOTH crates; names get a suffix
#   OBJCOPY=/path/to/llvm-objcopy ...             override the objcopy binary
#
# The pin map is recorded twice — the bootloader's info block `variant` and
# the application header's proto flag — and the assembly stage checks both
# against FW_FEATURES, then `fw-image.py verify` checks the finished image
# agrees with itself. Stale outputs of an earlier run for the same version
# are removed before anything is built, so a failed run leaves nothing that
# looks publishable.
#
# scripts/test-firmware-host.sh sources this file and runs the assembly stage
# (assemble_firmware_image) on synthetic images, so keep the checks in there
# and the cargo/objcopy calls in main.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
FW_IMAGE="$SCRIPT_DIR/fw-image.py"

# Layout constants, mirrored from layout/src/lib.rs. fw-image.py carries the
# same numbers and validates both images against them; the copies here are
# for the cross-checks of what the built images say about themselves and for
# the summary, so a drift in any one place fails the build instead of
# shipping.
BOOT_SIZE=24576              # 0x6000: the bootloader slot, 12 pages
BOOT_BUDGET=24064            # BOOT_SIZE - 512 headroom for future bootloader fixes
APP_BASE=134242304           # 0x08006000: the application slot and link address
APP_SIZE=86016               # 0x15000, 42 pages
BOOT_RESET_VECTOR=134217953  # 0x080000E1 = 0x08000000 + 0xE0 | 1 (cortex-m-rt Reset at _stext)
APP_RESET_VECTOR=134242529   # 0x080060E1 = APP_BASE + 0xE0 | 1
BOOT_PROTOCOL=1

die() {
    echo "build-firmware.sh: $*" >&2
    exit 1
}

# The version of one package from its manifest, via cargo metadata so the
# lockfile is honoured (--locked) and TOML is never parsed by hand.
package_version() {
    local manifest="$1" name="$2" version
    version="$(
        cargo metadata \
            --manifest-path "$manifest" \
            --locked \
            --no-deps \
            --format-version 1 |
            jq -r --arg name "$name" '.packages[] | select(.name == $name) | .version'
    )"
    if [[ -z "$version" || "$version" == "null" ]]; then
        die "could not read the version of $name from $manifest"
    fi
    printf '%s' "$version"
}

# 0 for the production pin map, 1 when FW_FEATURES enables `proto`; the
# bootloader's info block records the variant it was built for and the two
# images must agree or the pad's switch/LED pins would be wrong for one of them.
expected_variant() {
    case ",${FW_FEATURES:-}," in
        *,proto,*) echo 1 ;;
        *) echo 0 ;;
    esac
}

find_objcopy() {
    local host_triple llvm_tools_dir objcopy
    host_triple="$(rustc -vV | sed -n 's/^host: //p')"
    llvm_tools_dir="$(rustc --print sysroot)/lib/rustlib/$host_triple/bin"
    objcopy="$llvm_tools_dir/llvm-objcopy"
    if [[ ! -x "$objcopy" && -x "$llvm_tools_dir/rust-objcopy" ]]; then
        objcopy="$llvm_tools_dir/rust-objcopy"
    fi
    if [[ -n "${OBJCOPY:-}" ]]; then
        objcopy="$OBJCOPY"
    fi
    if [[ ! -x "$objcopy" ]]; then
        die "LLVM objcopy not found; run: rustup component add llvm-tools-preview"
    fi
    printf '%s' "$objcopy"
}

json_field() {
    jq -r "$2" "$1"
}

# Stage 2: from the two raw objcopy outputs to the verified release image.
#
#   assemble_firmware_image BOOT_BIN APP_BIN OUT_BIN OUT_HEX FW_VERSION BOOT_VERSION EXPECTED_VARIANT WORK_DIR
#
# Every check fails the build loudly; nothing is written to OUT_BIN before the
# inputs passed, and the stamped application only ever exists in WORK_DIR.
# Each external step carries its own `|| die`: bash switches `set -e` off
# inside `if`/`&&`/`||` contexts, and a gate that only works when nobody
# calls it from one is not a gate.
assemble_firmware_image() {
    local boot_bin="$1" app_bin="$2" out_bin="$3" out_hex="$4"
    local fw_version="$5" boot_version="$6" expected_variant="$7" work="$8"
    local boot_padded="$work/boot-padded.bin"
    local app_stamped="$work/app-stamped.bin"
    local info="$work/image-info.json"
    local boot_size app_length value expected_proto

    # --- bootloader: size budget, vectors, info block; then pad to the slot ---
    boot_size="$(wc -c < "$boot_bin" | tr -d ' ')"
    if (( boot_size > BOOT_BUDGET )); then
        die "bootloader is $boot_size bytes, over the $BOOT_BUDGET-byte budget (BOOT_SIZE $BOOT_SIZE minus 512 headroom)"
    fi
    python3 "$FW_IMAGE" verify "$boot_bin" || die "bootloader image failed verification"
    python3 "$FW_IMAGE" info --json "$boot_bin" > "$info" || die "could not describe the bootloader image"
    value="$(json_field "$info" '.boot.valid')"
    [[ "$value" == "true" ]] || die "bootloader image is invalid: $(json_field "$info" '.boot.error')"
    value="$(json_field "$info" '.boot.version')"
    [[ "$value" == "$boot_version" ]] || die "info block says bootloader $value, boot/Cargo.toml says $boot_version"
    value="$(json_field "$info" '.boot.protocol')"
    [[ "$value" == "$BOOT_PROTOCOL" ]] || die "info block protocol $value, expected $BOOT_PROTOCOL"
    value="$(json_field "$info" '.boot.variant')"
    [[ "$value" == "$expected_variant" ]] ||
        die "bootloader info block variant $value does not match FW_FEATURES='${FW_FEATURES:-}' (expected $expected_variant: 0 prod, 1 proto)"
    value="$(json_field "$info" '.boot.app_base')"
    [[ "$value" == "$APP_BASE" ]] || die "info block app_base $value, expected $APP_BASE (0x08006000)"
    value="$(json_field "$info" '.boot.app_size')"
    [[ "$value" == "$APP_SIZE" ]] || die "info block app_size $value, expected $APP_SIZE (0x15000)"
    value="$(json_field "$info" '.boot.reset_vector')"
    [[ "$value" == "$BOOT_RESET_VECTOR" ]] || die "bootloader reset vector $value, expected $BOOT_RESET_VECTOR (0x080000E1)"
    python3 "$FW_IMAGE" pad-boot "$boot_bin" --out "$boot_padded" || die "could not pad the bootloader"
    [[ -s "$boot_padded" ]] || die "padded bootloader was not written"

    # --- application: stamp length + crc, then check what the header says ---
    rm -f "$app_stamped"
    python3 "$FW_IMAGE" patch "$app_bin" --out "$app_stamped" || die "could not stamp the application image"
    [[ -s "$app_stamped" ]] || die "stamped application was not written"
    python3 "$FW_IMAGE" info --json "$app_stamped" > "$info" || die "could not describe the stamped application"
    value="$(json_field "$info" '.app.valid')"
    [[ "$value" == "true" ]] || die "stamped application is invalid: $(json_field "$info" '.app.error')"
    value="$(json_field "$info" '.app.stamped')"
    [[ "$value" == "true" ]] || die "application header is still unstamped after patch"
    value="$(json_field "$info" '.app.version')"
    [[ "$value" == "$fw_version" ]] || die "application header says $value, fw/Cargo.toml says $fw_version"
    # The application records its pin map too (header flags bit 0, set from
    # the `proto` feature at compile time); it must agree with FW_FEATURES
    # like the bootloader's variant did above. The final `verify` below
    # re-checks the pair against each other.
    if (( expected_variant == 1 )); then expected_proto=true; else expected_proto=false; fi
    value="$(json_field "$info" '.app.proto')"
    [[ "$value" == "$expected_proto" ]] ||
        die "application header proto flag is $value, but FW_FEATURES='${FW_FEATURES:-}' expects variant $expected_variant (0 prod, 1 proto): the two crates were built with different feature sets"
    value="$(json_field "$info" '.app.reset_vector')"
    [[ "$value" == "$APP_RESET_VECTOR" ]] ||
        die "application reset vector $value, expected $APP_RESET_VECTOR (0x080060E1): is fw/memory.x linked at 0x08006000 with _stext = +0xE0?"
    app_length="$(json_field "$info" '.app.length')"
    if (( app_length > APP_SIZE )); then
        die "application is $app_length bytes, over the $APP_SIZE-byte slot"
    fi

    # --- combine: bootloader slot + stamped application, back to back ---
    cat "$boot_padded" "$app_stamped" > "$out_bin" || die "could not write $out_bin"

    # The historical bounds checks. They still hold for the combined image:
    # word 0/1 are the bootloader's (0x20003FF0 / 0x080000E1) and the whole
    # image ends below the data pages.
    local bin_size initial_sp reset_vector reset_address
    bin_size="$(wc -c < "$out_bin" | tr -d ' ')"
    if (( bin_size < 192 || bin_size > 129024 )); then
        die "firmware size $bin_size is outside the safe 192..129024-byte range"
    fi
    initial_sp="$(od -An -tu4 -N4 "$out_bin" | tr -d ' ')"
    reset_vector="$(od -An -tu4 -j4 -N4 "$out_bin" | tr -d ' ')"
    reset_address=$((reset_vector & 0xfffffffe))
    if (( initial_sp < 0x20000000 || initial_sp > 0x20004000 || initial_sp % 4 != 0 )); then
        printf 'invalid initial stack pointer in firmware vector table: 0x%08x\n' "$initial_sp" >&2
        exit 1
    fi
    if (( reset_vector % 2 != 1 || reset_address < 0x08000000 || reset_address >= 0x0801f800 )); then
        printf 'invalid reset vector in firmware vector table: 0x%08x\n' "$reset_vector" >&2
        exit 1
    fi
    if (( bin_size != BOOT_SIZE + app_length )); then
        die "combined image is $bin_size bytes, expected $BOOT_SIZE + $app_length"
    fi

    # The gate the bootloader applies on the pad (header, vectors, CRC), run
    # over the final bytes rather than the pieces. A failure here removes the
    # output so a half-checked image never sits in the output directory.
    if ! python3 "$FW_IMAGE" verify "$out_bin"; then
        rm -f "$out_bin"
        die "combined image failed verification"
    fi

    # Intel HEX for factory programming: same bytes as the .bin, load address
    # embedded so SWD/gang programmers place it at 0x08000000 without operator
    # input; the entry record is the bootloader's reset vector, which is what a
    # programmer should jump to anyway. bin2hex.py verifies its own round-trip.
    python3 "$SCRIPT_DIR/bin2hex.py" "$out_bin" "$out_hex" || die "could not write $out_hex"

    echo "Firmware $fw_version: $out_bin ($bin_size bytes)"
    printf 'Vectors: SP=0x%08x reset=0x%08x\n' "$initial_sp" "$reset_vector"
    echo "Layout:"
    printf '  bootloader  %s  %6d bytes, %6d free of the %d-byte budget (slot %d)\n' \
        "$boot_version" "$boot_size" "$((BOOT_BUDGET - boot_size))" "$BOOT_BUDGET" "$BOOT_SIZE"
    printf '  application %s  %6d bytes, %6d free of the %d-byte slot at 0x08006000\n' \
        "$fw_version" "$app_length" "$((APP_SIZE - app_length))" "$APP_SIZE"
    printf '  combined    %6d bytes at 0x08000000, variant %s\n' "$bin_size" "$expected_variant"
}

main() {
    local output_dir="${1:-$REPO_ROOT/dist}"
    # FW_FEATURES=proto builds for the prototype board (pre-v23 pin map); the
    # feature list is appended to the artifact names so revisions can't be mixed up.
    FW_FEATURES="${FW_FEATURES:-}"
    local variant="${FW_FEATURES:+-${FW_FEATURES//,/-}}"
    local fw_version boot_version objcopy work
    local boot_elf app_elf bin hex app_debug_elf boot_debug_elf

    mkdir -p "$output_dir"
    # The EXIT trap runs after main() has returned, so the directory it
    # removes must live in a global, not in one of main's locals.
    WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/openmicro-build.XXXXXX")"
    trap 'rm -rf "${WORK_DIR:-}"' EXIT
    work="$WORK_DIR"

    fw_version="$(package_version "$REPO_ROOT/fw/Cargo.toml" openmicro-fw)"
    boot_version="$(package_version "$REPO_ROOT/boot/Cargo.toml" openmicro-boot)"

    # The outputs are only written at the end, so anything with these names
    # in OUTPUT_DIR is from an earlier run. Remove it before anything else
    # can fail: a missing objcopy, a cargo error or a gate that dies below
    # must not leave yesterday's bytes behind under today's version for
    # package-macos.sh to pick up.
    bin="$output_dir/openmicro-fw-$fw_version$variant.bin"
    hex="$output_dir/openmicro-fw-$fw_version$variant.hex"
    app_debug_elf="$output_dir/openmicro-fw-$fw_version$variant.elf"
    boot_debug_elf="$output_dir/openmicro-boot-$boot_version$variant.elf"
    rm -f "$bin" "$hex" "$app_debug_elf" "$boot_debug_elf"

    objcopy="$(find_objcopy)"

    # The bootloader: same feature set as the application so both agree on
    # the pin map (the info block records it; assemble_firmware_image checks).
    # boot/.cargo/config.toml pins the thumbv6m target and -Tlink.x.
    (
        cd "$REPO_ROOT/boot"
        cargo build --release --locked ${FW_FEATURES:+--features "$FW_FEATURES"}
    )
    boot_elf="$REPO_ROOT/boot/target/thumbv6m-none-eabi/release/openmicro-boot"
    [[ -f "$boot_elf" ]] || die "bootloader ELF not produced: $boot_elf"
    "$objcopy" -O binary "$boot_elf" "$work/boot.bin"

    # The application, logging off as for every release.
    (
        cd "$REPO_ROOT/fw"
        DEFMT_LOG=off cargo build --release --locked ${FW_FEATURES:+--features "$FW_FEATURES"}
    )
    app_elf="$REPO_ROOT/fw/target/thumbv6m-none-eabi/release/openmicro-fw"
    [[ -f "$app_elf" ]] || die "application ELF not produced: $app_elf"
    "$objcopy" -O binary "$app_elf" "$work/app.bin"

    assemble_firmware_image "$work/boot.bin" "$work/app.bin" "$bin" "$hex" \
        "$fw_version" "$boot_version" "$(expected_variant)" "$work"

    cp "$app_elf" "$app_debug_elf"
    cp "$boot_elf" "$boot_debug_elf"

    echo "Bootloader $boot_version: $boot_debug_elf"
    shasum -a 256 "$bin" "$app_debug_elf" "$boot_debug_elf" "$hex"
}

# Sourced by scripts/test-firmware-host.sh for assemble_firmware_image only.
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
