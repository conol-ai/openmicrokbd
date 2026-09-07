#!/usr/bin/env bash
# Host-side tests for the firmware side of the repository: everything that
# runs on the build machine without a pad or a cross toolchain. The release
# workflow runs this before scripts/build-firmware.sh.
#
#   scripts/test-firmware-host.sh
#
#   1. layout/          the shared contract crate (constants, header, CRC-32,
#                       validate_app) — unit tests
#   2. fw/host-tests    the Codex Micro wire codec (what scripts/test-codex-wire.sh runs)
#   3. boot/host-tests  the bootloader's update state machine over a RAM
#                       Programmer: happy path, out-of-order chunk, bad
#                       length, CRC mismatch, BEGIN/END edge cases
#   4. fw-image.py selftest: CRC vectors shared with the crate, stamp /
#                       verify / tamper / slice round trip through the CLI,
#                       pin-map agreement of a combined image
#   5. test-bootloader.py selftest: the host-side HID client (chunking,
#                       stale replies, resync, load_app_slice refusals)
#                       against an in-process model of the bootloader;
#                       needs no hardware and no hidapi
#   6. build-firmware.sh's assembly stage on synthetic boot.bin / app.bin:
#                       the same checks the release build applies, plus
#                       the mismatches it must refuse
#
# fw/.cargo/config.toml pins the build target to thumbv6m for everything under
# fw/ (and boot/.cargo/config.toml does the same), so the host triple is passed
# explicitly to every cargo invocation. --locked (not --offline) so CI with a
# cold registry still resolves while a stale lockfile is still an error.
set -euo pipefail
cd "$(dirname "$0")/.."

host="$(rustc -vV | sed -n 's/^host: //p')"
if [[ -z "$host" ]]; then
    echo "cannot determine the rustc host triple" >&2
    exit 1
fi

failed=()
run() {
    echo "== $*"
    if "$@"; then
        echo "   ok"
    else
        echo "   FAILED: $*" >&2
        failed+=("$*")
    fi
}

# The assembly stage of the release build, dry-run on synthetic images written
# by fw-image.py synth (real vectors, real header/info block, fake bodies):
# a good pair must produce a verified combined image and hex; a variant
# mismatch, a version mismatch, an over-budget bootloader and a mislinked
# application must each be refused; and no application-only .bin may be left
# in the output directory.
#
# `run` invokes this inside an `if`, where bash ignores `set -e` for
# everything below (subshells and re-issued `set -e` included), so nothing
# here relies on it: every step goes through `must` / `must_fail`, which
# exit the subshell explicitly.
assembly_dry_run() {
    local work
    work="$(mktemp -d "${TMPDIR:-/tmp}/openmicro-assembly.XXXXXX")"
    local rc=0
    (
        must() {  # must DESCRIPTION cmd args...
            local what="$1"
            shift
            if ! "$@"; then
                echo "dry run: $what failed: $*" >&2
                exit 1
            fi
        }
        must_fail() {  # must_fail DESCRIPTION boot.bin app.bin fw_ver boot_ver variant workdir
            local what="$1" boot="$2" app="$3" fw_ver="$4" boot_ver="$5" variant="$6" w="$7"
            mkdir -p "$w"
            if (assemble_firmware_image "$boot" "$app" "$work/out/$what.bin" "$work/out/$what.hex" \
                    "$fw_ver" "$boot_ver" "$variant" "$w") > /dev/null 2>&1; then
                echo "dry run: $what was not refused" >&2
                exit 1
            fi
        }
        # shellcheck source=scripts/build-firmware.sh
        source scripts/build-firmware.sh
        mkdir -p "$work/out" "$work/w-prod" "$work/w-proto"

        # Production pair.
        must "synth prod" python3 scripts/fw-image.py synth --out "$work/prod" --variant 0 --boot-version 1.0.0 --fw-version 0.10.0 --quiet
        must "assemble prod" assemble_firmware_image "$work/prod/boot.bin" "$work/prod/app.bin" \
            "$work/out/openmicro-fw-0.10.0.bin" "$work/out/openmicro-fw-0.10.0.hex" \
            0.10.0 1.0.0 0 "$work/w-prod" > "$work/prod.log"
        must "combined .bin written" test -s "$work/out/openmicro-fw-0.10.0.bin"
        must "combined .hex written" test -s "$work/out/openmicro-fw-0.10.0.hex"
        must "combined image verifies" python3 scripts/fw-image.py verify "$work/out/openmicro-fw-0.10.0.bin" --quiet
        must "layout summary printed" grep -q '^Layout:' "$work/prod.log"
        must "stamped app kept in the work dir" test -s "$work/w-prod/app-stamped.bin"
        # The stamped application stays in the work dir, never next to the outputs.
        if ls "$work/out" | grep -q 'app'; then
            echo "dry run: an application-only image was written into the output directory" >&2
            exit 1
        fi

        # Prototype pair, expected variant 1.
        must "synth proto" python3 scripts/fw-image.py synth --out "$work/proto" --variant 1 --quiet
        must "assemble proto" assemble_firmware_image "$work/proto/boot.bin" "$work/proto/app.bin" \
            "$work/out/openmicro-fw-0.10.0-proto.bin" "$work/out/openmicro-fw-0.10.0-proto.hex" \
            0.10.0 1.0.0 1 "$work/w-proto" > /dev/null

        # Refusals, each with a fresh work dir like the real build's mktemp.
        # A proto bootloader with a prod app (caught at the info block), and
        # a prod bootloader with a proto app (caught at the header flag).
        must_fail "variant-mismatch" "$work/proto/boot.bin" "$work/prod/app.bin" 0.10.0 1.0.0 0 "$work/w-bad1"
        must_fail "app-flag-mismatch" "$work/prod/boot.bin" "$work/proto/app.bin" 0.10.0 1.0.0 0 "$work/w-bad7"
        must_fail "app-flag-mismatch-proto" "$work/proto/boot.bin" "$work/prod/app.bin" 0.10.0 1.0.0 1 "$work/w-bad8"
        must_fail "boot-version-mismatch" "$work/prod/boot.bin" "$work/prod/app.bin" 0.10.0 1.0.1 0 "$work/w-bad2"
        must_fail "app-version-mismatch" "$work/prod/boot.bin" "$work/prod/app.bin" 0.11.0 1.0.0 0 "$work/w-bad3"
        must "synth over-budget boot" python3 scripts/fw-image.py synth --out "$work/bigboot" --boot-body 24000 --quiet
        must_fail "boot-over-budget" "$work/bigboot/boot.bin" "$work/prod/app.bin" 0.10.0 1.0.0 0 "$work/w-bad4"
        # An application linked at the wrong address (reset vector off by a page).
        must "make mislinked app" python3 - "$work/prod/app.bin" "$work/mislinked.bin" <<'PY'
import struct, sys
img = bytearray(open(sys.argv[1], "rb").read())
struct.pack_into("<I", img, 4, 0x08006800 + 0xE0 + 1)
open(sys.argv[2], "wb").write(img)
PY
        must_fail "mislinked-app" "$work/prod/boot.bin" "$work/mislinked.bin" 0.10.0 1.0.0 0 "$work/w-bad5"
        # The mislinked case with a stale stamped image already in the work
        # dir: the refusal must not be fooled by leftovers.
        mkdir -p "$work/w-bad6"
        must "plant stale stamped image" cp "$work/w-prod/app-stamped.bin" "$work/w-bad6/app-stamped.bin"
        must_fail "mislinked-app-stale-workdir" "$work/prod/boot.bin" "$work/mislinked.bin" 0.10.0 1.0.0 0 "$work/w-bad6"
        if ls "$work/out" | grep -q -- '-mismatch\|-budget\|mislinked'; then
            echo "dry run: a refused build left output files behind: $(ls "$work/out")" >&2
            exit 1
        fi
        exit 0
    )
    rc=$?
    rm -rf "$work"
    return "$rc"
}

run cargo test --manifest-path layout/Cargo.toml --locked --target "$host"
run cargo test --manifest-path fw/host-tests/Cargo.toml --locked --target "$host"
run cargo test --manifest-path boot/host-tests/Cargo.toml --locked --target "$host"
run python3 scripts/fw-image.py selftest --quiet
run python3 scripts/test-bootloader.py selftest
run assembly_dry_run

if (( ${#failed[@]} > 0 )); then
    echo
    echo "FAILED (${#failed[@]}):" >&2
    printf '  %s\n' "${failed[@]}" >&2
    exit 1
fi
echo
echo "all host-side firmware tests passed"
