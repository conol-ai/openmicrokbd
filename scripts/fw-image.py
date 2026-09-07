#!/usr/bin/env python3
"""OpenMicro v1 firmware image tool: inspect, stamp, verify and slice images.

Since firmware 0.10.0 the pad runs a resident bootloader (boot/) in the first
24 KiB of flash and the application (fw/) behind it at 0x08006000. Both carry
a 32-byte block right after their vector table (offset 0xC0):

    bootloader info block   "OMKB", protocol, variant, app_base, app_size, version
    application header      "OMKA", length, crc32, header version, flags, version

The bootloader validates the application header (length + CRC-32) before every
jump and stays in its USB HID update mode when it does not check out, so the
release pipeline must stamp every application image and never publish one
that would fail on the pad. This tool is that pipeline's single source of
truth on the host side; the numbers below mirror layout/src/lib.rs
(openmicro-layout), which the bootloader, the firmware and the host app share.
`selftest` cross-checks the CRC against the crate's test vectors.

    fw-image.py info <file> [--json]        what is in an image (combined, app, boot, legacy)
    fw-image.py patch <app.bin> [--out P]   pad to a word and stamp length + crc into the header
    fw-image.py verify <file>               release gate: exit 1 unless the image would boot (a combined
                                            image must also agree with itself on the pin map: the app
                                            header's proto flag against the info block's variant)
    fw-image.py slice <combined> --out P    extract the application image (exactly header.length bytes)
    fw-image.py pad-boot <boot.bin> --out P pad a raw bootloader to its 24 KiB slot
    fw-image.py selftest                    synthetic round trip, tamper checks, CRC vectors

Python 3.9, standard library only. Multi-byte fields are little-endian.
"""

import argparse
import json
import os
import struct
import sys
import tempfile
import zlib

# ---- layout: MUST match layout/src/lib.rs ---------------------------------

FLASH_BASE = 0x08000000
FLASH_SIZE = 0x20000
PAGE_SIZE = 2048

BOOT_BASE = FLASH_BASE
BOOT_SIZE = 0x6000  # 24 KiB, 12 pages; never erased by an update
APP_BASE = BOOT_BASE + BOOT_SIZE  # 0x08006000, also the app's link address
APP_SIZE = 0x15000  # 84 KiB, 42 pages
APP_END = APP_BASE + APP_SIZE
# First byte the bootloader must never erase: Work Louder file slots and the
# config page live above it.
DATA_BASE = 0x0801B000

RAM_BASE = 0x20000000
RAM_SIZE = 0x4000
RAM_END = RAM_BASE + RAM_SIZE

# 16 core + 32 device vectors on the F072; both blocks sit right behind them
# and cortex-m-rt places Reset at _stext = base + 0xE0.
VECTORS_LEN = 0xC0
HEADER_OFFSET = VECTORS_LEN
HEADER_LEN = 32
RESET_OFFSET = HEADER_OFFSET + HEADER_LEN  # 0xE0

APP_MAGIC = 0x414B4D4F  # bytes "OMKA"
BOOT_MAGIC = 0x424B4D4F  # bytes "OMKB"
HEADER_VERSION = 1
BOOT_PROTOCOL = 1
VARIANT_PROD = 0
VARIANT_PROTO = 1
VARIANT_NAMES = {VARIANT_PROD: "prod", VARIANT_PROTO: "proto"}
# Application header `flags` bit 0: built with the `proto` pin map. The
# bootloader refuses an upload whose flag disagrees with its own variant, so
# a combined image must agree with itself (verify checks that).
FLAG_PROTO = 1


def flags_for_variant(variant):
    """The header flags an application built for `variant` must carry."""
    return FLAG_PROTO if variant == VARIANT_PROTO else 0


def variant_for_flags(flags):
    """The pin-map variant an application header's flags describe."""
    return VARIANT_PROTO if flags & FLAG_PROTO else VARIANT_PROD


# The raw bootloader must leave headroom in its slot so a future fix always
# fits without moving APP_BASE (which is baked into every app image).
BOOT_MAX_LEN = BOOT_SIZE - 512  # 24064

HEADER_FMT = "<IIIHH16s"  # magic, length, crc, header_version, flags, version
INFO_FMT = "<IHHII16s"  # magic, protocol, variant, app_base, app_size, version

KIND_COMBINED = "combined"
KIND_APP = "app"
KIND_BOOT = "boot"
KIND_LEGACY = "legacy"


class ImageError(Exception):
    """A check failed; the message says which one and why."""


# ---- small helpers ----------------------------------------------------------


def u32(b, off):
    return struct.unpack_from("<I", b, off)[0]


def cstr(raw):
    end = raw.find(b"\0")
    if end >= 0:
        raw = raw[:end]
    return raw.decode("ascii", "replace")


def version_bytes(s):
    """A version string as the NUL-padded 16-byte field (truncated if longer)."""
    return s.encode("ascii")[:16].ljust(16, b"\0")


def app_header_bytes(version, flags=0):
    """The header as linked into the ELF (unstamped: length and crc zero);
    `flags` is FLAG_PROTO for a `proto` build, else 0."""
    return struct.pack(HEADER_FMT, APP_MAGIC, 0, 0, HEADER_VERSION, flags, version_bytes(version))


def boot_info_bytes(version, variant):
    return struct.pack(INFO_FMT, BOOT_MAGIC, BOOT_PROTOCOL, variant, APP_BASE, APP_SIZE, version_bytes(version))


def fmt_size(n):
    return "{:,}".format(n)


# ---- CRC-32 (ISO-HDLC: zlib.crc32 == openmicro_layout::crc32) --------------


def crc32(data):
    return zlib.crc32(data) & 0xFFFFFFFF


def image_crc(image):
    """CRC of an application image with the header's crc field taken as zero,
    exactly like openmicro_layout::image_crc: the field cannot cover itself."""
    at = HEADER_OFFSET + 8
    c = zlib.crc32(image[:at])
    c = zlib.crc32(b"\0\0\0\0", c)
    c = zlib.crc32(image[at + 4 :], c)
    return c & 0xFFFFFFFF


# ---- header / info block parsing --------------------------------------------


def parse_app_header(b):
    """The 32-byte application header, or None if the magic is missing."""
    if len(b) < HEADER_LEN:
        return None
    magic, length, crc, header_version, flags, version = struct.unpack_from(HEADER_FMT, b)
    if magic != APP_MAGIC:
        return None
    return {
        "length": length,
        "crc": crc,
        "header_version": header_version,
        "flags": flags,
        # Bit 0 of flags: the image was built with the `proto` pin map.
        "proto": bool(flags & FLAG_PROTO),
        "version": cstr(version),
        # What cargo run / probe-rs flashes straight from the ELF: the
        # bootloader then only checks the vectors.
        "stamped": not (length == 0 and crc == 0),
    }


def parse_boot_info(b):
    """The 32-byte bootloader info block, or None if the magic is missing."""
    if len(b) < HEADER_LEN:
        return None
    magic, protocol, variant, app_base, app_size, version = struct.unpack_from(INFO_FMT, b)
    if magic != BOOT_MAGIC:
        return None
    return {
        "protocol": protocol,
        "variant": variant,
        "variant_name": VARIANT_NAMES.get(variant, "unknown"),
        "app_base": app_base,
        "app_size": app_size,
        "version": cstr(version),
    }


def detect(image):
    """Classify an image by the magics behind the vector table(s).

    combined: OMKB at 0xC0 and OMKA behind the bootloader slot (the release .bin)
    boot:     OMKB at 0xC0 only (raw or padded bootloader)
    app:      OMKA at 0xC0 (raw objcopy output of fw/, or a slice)
    legacy:   neither — fw <= 0.9.0 linked at 0x08000000, or not firmware
    """
    if len(image) >= RESET_OFFSET and u32(image, HEADER_OFFSET) == BOOT_MAGIC:
        off = app_slice_offset(parse_boot_info(image[HEADER_OFFSET:]))
        if len(image) >= off + RESET_OFFSET and u32(image, off + HEADER_OFFSET) == APP_MAGIC:
            return KIND_COMBINED
        return KIND_BOOT
    if len(image) >= RESET_OFFSET and u32(image, HEADER_OFFSET) == APP_MAGIC:
        return KIND_APP
    return KIND_LEGACY


def app_slice_offset(info):
    """Where the application starts inside a combined image. The info block
    is the authority (the host app never hard-codes the layout either); a
    garbage app_base falls back to the compiled-in constant so `info` can
    still describe a broken image."""
    if info and BOOT_BASE + RESET_OFFSET <= info["app_base"] < DATA_BASE and info["app_base"] % PAGE_SIZE == 0:
        return info["app_base"] - FLASH_BASE
    return BOOT_SIZE


def boot_used(region):
    """Bytes of the bootloader slot that are not erased-flash padding."""
    stripped = region.rstrip(b"\xff")
    return max(len(stripped), RESET_OFFSET)


# ---- validation (mirrors openmicro_layout::validate_app) --------------------


def check_vectors(image, base, what):
    if len(image) < RESET_OFFSET:
        raise ImageError("{}: only {} bytes, fewer than a vector table plus header ({})".format(what, len(image), RESET_OFFSET))
    sp, reset = struct.unpack_from("<II", image)
    if sp < RAM_BASE or sp > RAM_END or sp % 8 != 0:
        raise ImageError("{}: initial stack pointer 0x{:08X} is not in RAM 0x{:08X}..=0x{:08X} or not 8-byte aligned".format(what, sp, RAM_BASE, RAM_END))
    expected = (base + RESET_OFFSET) | 1
    if reset != expected:
        raise ImageError(
            "{}: reset vector 0x{:08X} is not 0x{:08X} (base 0x{:08X} + 0x{:X} | 1) — linked for another address?".format(
                what, reset, expected, base, RESET_OFFSET
            )
        )
    return sp, reset


def validate_app(image, app_base=APP_BASE, app_size=APP_SIZE):
    """Returns (validity, header) with validity "stamped" or "unstamped", or
    raises ImageError. `image` starts at app_base; it may be the whole slot
    (bytes past header.length are ignored, like the bootloader does)."""
    what = "application"
    if len(image) < RESET_OFFSET:
        raise ImageError("{}: only {} bytes, fewer than a vector table plus header ({})".format(what, len(image), RESET_OFFSET))
    header = parse_app_header(image[HEADER_OFFSET:])
    if header is None:
        raise ImageError("{}: no OMKA header magic at +0x{:X}".format(what, HEADER_OFFSET))
    if header["header_version"] != HEADER_VERSION:
        raise ImageError("{}: header version {} is not {}".format(what, header["header_version"], HEADER_VERSION))
    check_vectors(image, app_base, what)
    if not header["stamped"]:
        return "unstamped", header
    length = header["length"]
    if length < RESET_OFFSET or length > app_size or length % 4 != 0:
        raise ImageError(
            "{}: header length {} is outside {}..={} or not a multiple of 4".format(what, length, RESET_OFFSET, app_size)
        )
    if length > len(image):
        raise ImageError("{}: header length {} exceeds the {} bytes given".format(what, length, len(image)))
    actual = image_crc(image[:length])
    if actual != header["crc"]:
        raise ImageError("{}: crc mismatch: header 0x{:08X}, image 0x{:08X}".format(what, header["crc"], actual))
    return "stamped", header


def validate_boot(region):
    """Checks a bootloader image (raw objcopy output or the padded slot):
    vectors, info block contents against the constants, and the size budget.
    Returns (info, used_bytes)."""
    what = "bootloader"
    if len(region) > BOOT_SIZE:
        raise ImageError("{}: {} bytes exceed the {}-byte slot".format(what, len(region), BOOT_SIZE))
    sp, reset = check_vectors(region, BOOT_BASE, what)
    info = parse_boot_info(region[HEADER_OFFSET:])
    if info is None:
        raise ImageError("{}: no OMKB info block magic at +0x{:X}".format(what, HEADER_OFFSET))
    if info["protocol"] != BOOT_PROTOCOL:
        raise ImageError("{}: info block protocol {} is not {}".format(what, info["protocol"], BOOT_PROTOCOL))
    if info["app_base"] != FLASH_BASE + BOOT_SIZE:
        raise ImageError("{}: info block app_base 0x{:08X} is not 0x{:08X}".format(what, info["app_base"], FLASH_BASE + BOOT_SIZE))
    if info["app_size"] != APP_SIZE:
        raise ImageError("{}: info block app_size 0x{:X} is not 0x{:X}".format(what, info["app_size"], APP_SIZE))
    if info["variant"] not in VARIANT_NAMES:
        raise ImageError("{}: info block variant {} is neither prod (0) nor proto (1)".format(what, info["variant"]))
    used = boot_used(region)
    if used > BOOT_MAX_LEN:
        raise ImageError("{}: {} bytes used, over the {}-byte budget (slot {} minus 512 headroom)".format(what, used, BOOT_MAX_LEN, BOOT_SIZE))
    info = dict(info, sp=sp, reset_vector=reset, used=used)
    return info, used


def check_variant(header, info):
    """A combined image must agree with itself: the application header's
    proto flag and the bootloader info block's variant must name the same
    pin map, or the pad's switch and LED pins would be wrong for one of them
    (the bootloader refuses such an upload over HID for the same reason)."""
    app_variant = variant_for_flags(header["flags"])
    if app_variant != info["variant"]:
        raise ImageError(
            "application header flags 0x{:04X} ({}) do not match the bootloader info block variant {} ({}): "
            "the two images were built for different pin maps".format(
                header["flags"], VARIANT_NAMES[app_variant], info["variant"], VARIANT_NAMES.get(info["variant"], "unknown")
            )
        )


def stamp_app(image):
    """Pads to a multiple of 4 with 0xFF (erased flash) and writes length and
    crc into the header. Returns (bytes, header)."""
    if len(image) < RESET_OFFSET or parse_app_header(image[HEADER_OFFSET:]) is None:
        raise ImageError("no OMKA header magic at +0x{:X}: not an application image".format(HEADER_OFFSET))
    out = bytearray(image)
    while len(out) % 4:
        out.append(0xFF)
    length = len(out)
    struct.pack_into("<II", out, HEADER_OFFSET + 4, length, 0)
    crc = image_crc(bytes(out))
    struct.pack_into("<I", out, HEADER_OFFSET + 8, crc)
    return bytes(out), parse_app_header(out[HEADER_OFFSET:])


# ---- description of an image (info) ------------------------------------------


def describe(image, path="", app_base_override=None):
    """Everything `info` prints, as a dict; never raises on a bad image (it
    records the problem instead) so a broken build can still be inspected."""
    kind = detect(image)
    d = {"file": path, "kind": kind, "size": len(image)}
    if kind in (KIND_COMBINED, KIND_BOOT):
        region = image[:BOOT_SIZE]
        boot = {"base": BOOT_BASE, "slot_size": BOOT_SIZE, "budget": BOOT_MAX_LEN, "valid": True, "error": None}
        raw_info = parse_boot_info(region[HEADER_OFFSET:]) or {}
        boot.update(raw_info)
        if len(region) >= 8:
            boot["sp"], boot["reset_vector"] = struct.unpack_from("<II", region)
        boot["used"] = boot_used(region) if kind == KIND_COMBINED or len(region) == BOOT_SIZE else len(region)
        boot["free"] = BOOT_MAX_LEN - boot["used"]
        try:
            validate_boot(region)
        except ImageError as e:
            boot["valid"] = False
            boot["error"] = str(e)
        d["boot"] = boot
    if kind in (KIND_COMBINED, KIND_APP):
        if kind == KIND_COMBINED:
            off = app_slice_offset(d["boot"])
            app_base = FLASH_BASE + off
            app_size = d["boot"].get("app_size") or APP_SIZE
            if app_size <= 0 or app_size > FLASH_SIZE:
                app_size = APP_SIZE
            piece = image[off:]
        else:
            app_base = APP_BASE
            app_size = APP_SIZE
            piece = image
        if app_base_override is not None:
            app_base = app_base_override
        app = {"base": app_base, "slot_size": app_size, "slice_size": len(piece), "valid": True, "error": None, "validity": None}
        app.update(parse_app_header(piece[HEADER_OFFSET:]) or {})
        if len(piece) >= 8:
            app["sp"], app["reset_vector"] = struct.unpack_from("<II", piece)
        image_len = app["length"] if app.get("stamped") else len(piece)
        app["free"] = app_size - image_len
        if app.get("stamped") and app["length"] <= len(piece):
            app["crc_ok"] = image_crc(piece[: app["length"]]) == app["crc"]
        else:
            app["crc_ok"] = None
        try:
            app["validity"], _ = validate_app(piece, app_base, app_size)
        except ImageError as e:
            app["valid"] = False
            app["error"] = str(e)
        d["app"] = app
        if kind == KIND_COMBINED:
            d["trailing_after_app"] = len(piece) - app["length"] if app.get("stamped") else 0
            # The pair must agree on the pin map; a mismatch makes the
            # application invalid for THIS bootloader, whatever its CRC says.
            app["variant_ok"] = None
            if "flags" in app and d["boot"].get("variant") in VARIANT_NAMES:
                try:
                    check_variant(app, d["boot"])
                    app["variant_ok"] = True
                except ImageError as e:
                    app["variant_ok"] = False
                    if app["valid"]:
                        app["valid"] = False
                        app["error"] = str(e)
    if kind == KIND_LEGACY and len(image) >= 8:
        d["sp"], d["reset_vector"] = struct.unpack_from("<II", image)
    return d


def print_description(d):
    kind = d["kind"]
    title = {
        KIND_COMBINED: "combined image (bootloader + application)",
        KIND_APP: "application image (app slot only)",
        KIND_BOOT: "bootloader image",
        KIND_LEGACY: "legacy image (no header: fw <= 0.9.0 linked at 0x08000000, or not firmware)",
    }[kind]
    print("{}: {}, {} bytes".format(d["file"] or "<image>", title, fmt_size(d["size"])))
    if "boot" in d:
        b = d["boot"]
        print(
            "  bootloader   0x{:08X}  version {}  protocol {}  variant {} ({})".format(
                b["base"], b.get("version", "?"), b.get("protocol", "?"), b.get("variant", "?"), b.get("variant_name", "?")
            )
        )
        print("               SP 0x{:08X}  reset 0x{:08X}  app_base 0x{:08X}  app_size 0x{:X}".format(
            b.get("sp", 0), b.get("reset_vector", 0), b.get("app_base", 0), b.get("app_size", 0)))
        print("               {} bytes used, {} free of the {}-byte budget (slot {})".format(
            fmt_size(b["used"]), fmt_size(b["free"]), fmt_size(b["budget"]), fmt_size(b["slot_size"])))
        if not b["valid"]:
            print("               INVALID: {}".format(b["error"]))
    if "app" in d:
        a = d["app"]
        state = "stamped" if a.get("stamped") else "unstamped (length/crc zero: debugger image)"
        print("  application  0x{:08X}  version {}  header v{}  flags 0x{:04X} ({})  {}".format(
            a["base"], a.get("version", "?"), a.get("header_version", "?"), a.get("flags", 0),
            VARIANT_NAMES[variant_for_flags(a.get("flags", 0))], state))
        print("               SP 0x{:08X}  reset 0x{:08X}".format(a.get("sp", 0), a.get("reset_vector", 0)))
        if a.get("stamped"):
            print("               length {}  crc 0x{:08X} ({})".format(
                fmt_size(a["length"]), a["crc"], "ok" if a["crc_ok"] else "MISMATCH" if a["crc_ok"] is False else "unchecked"))
        print("               {} bytes in the slice, {} free of the {}-byte slot".format(
            fmt_size(a["slice_size"]), fmt_size(a["free"]), fmt_size(a["slot_size"])))
        if d.get("trailing_after_app"):
            print("               {} bytes follow the stamped image".format(fmt_size(d["trailing_after_app"])))
        if not a["valid"]:
            print("               INVALID: {}".format(a["error"]))
    if kind == KIND_LEGACY and "sp" in d:
        print("  SP 0x{:08X}  reset 0x{:08X}".format(d["sp"], d["reset_vector"]))


# ---- verify -----------------------------------------------------------------


def verify_image(image, app_base=None, allow_unstamped=False):
    """The release gate. Returns a one-line summary or raises ImageError."""
    kind = detect(image)
    if kind == KIND_LEGACY:
        raise ImageError("legacy image: no bootloader info block or application header at 0xC0 (fw <= 0.9.0 image, or not firmware)")
    if kind == KIND_BOOT:
        info, used = validate_boot(image)
        return "bootloader {} ({}) ok: {} bytes used of {}".format(info["version"], info["variant_name"], used, BOOT_MAX_LEN)
    if kind == KIND_COMBINED:
        if len(image) < BOOT_SIZE + RESET_OFFSET:
            raise ImageError("combined image is only {} bytes: the bootloader slot alone is {}".format(len(image), BOOT_SIZE))
        if len(image) > DATA_BASE - FLASH_BASE:
            raise ImageError("combined image ({} bytes) reaches into the data pages above 0x{:08X}".format(len(image), DATA_BASE))
        info, used = validate_boot(image[:BOOT_SIZE])
        if app_base is not None and app_base != info["app_base"]:
            raise ImageError("--app-base 0x{:08X} contradicts the info block's 0x{:08X}".format(app_base, info["app_base"]))
        piece = image[BOOT_SIZE:]
        validity, header = validate_app(piece, info["app_base"], info["app_size"])
        # The flag is set at compile time, so it is checked for unstamped
        # images too: a debugger image for the other pin map is just as wrong.
        check_variant(header, info)
        if validity != "stamped":
            if not allow_unstamped:
                raise ImageError("application is unstamped: run `fw-image.py patch` on the app image before combining")
            return "combined image ok (UNSTAMPED app {}, boot {}, {})".format(header["version"], info["version"], info["variant_name"])
        tail = piece[header["length"] :]
        if tail.strip(b"\xff"):
            raise ImageError("{} bytes of non-0xFF data follow the stamped application image".format(len(tail)))
        return "combined image ok: boot {} ({}) {} bytes + app {} ({}) {} bytes, crc 0x{:08X}".format(
            info["version"], info["variant_name"], used, header["version"], VARIANT_NAMES[variant_for_flags(header["flags"])],
            header["length"], header["crc"])
    # KIND_APP: nothing to compare the flag against; the summary names the
    # pin map so the operator can.
    base = APP_BASE if app_base is None else app_base
    validity, header = validate_app(image, base, APP_SIZE)
    variant_name = VARIANT_NAMES[variant_for_flags(header["flags"])]
    if validity != "stamped":
        if not allow_unstamped:
            raise ImageError("application is unstamped: run `fw-image.py patch` first")
        return "application {} ({}) at 0x{:08X} ok (UNSTAMPED)".format(header["version"], variant_name, base)
    tail = image[header["length"] :]
    if tail.strip(b"\xff"):
        raise ImageError("{} bytes of non-0xFF data follow the stamped application image".format(len(tail)))
    return "application {} ({}) at 0x{:08X} ok: {} bytes, crc 0x{:08X}".format(
        header["version"], variant_name, base, header["length"], header["crc"])


# ---- slicing ----------------------------------------------------------------


def app_slice(image):
    """The stamped application image inside a combined (or app) image, exactly
    header.length bytes, plus (app_base, header). Raises if it does not verify."""
    kind = detect(image)
    info = None
    if kind == KIND_COMBINED:
        info, _ = validate_boot(image[:BOOT_SIZE])
        base, piece = info["app_base"], image[BOOT_SIZE:]
    elif kind == KIND_APP:
        base, piece = APP_BASE, image
    else:
        raise ImageError("not a combined or application image ({})".format(kind))
    validity, header = validate_app(piece, base, APP_SIZE)
    if info is not None:
        check_variant(header, info)
    if validity != "stamped":
        raise ImageError("application is unstamped, its length is unknown: run `fw-image.py patch` first")
    return piece[: header["length"]], base, header


def suggested_slice_name(header, app_base):
    return "openmicro-fw-{}-app-slot-0x{:08X}.bin".format(header["version"], app_base)


def slice_flash_hint(path, app_base):
    """How to put a slice on the pad with dfu-util, without `:leave`: leave
    makes the ROM jump straight to the slice, skipping the bootloader's
    validation and its reset hygiene (MEM_MODE, SysTick). A replug goes
    through the bootloader, which is the only start path the slice is
    validated on."""
    return (
        "dfu-util -d 0483:df11 -a 0 -s 0x{:08X} -D {}\n"
        "then unplug and replug the pad: the bootloader validates the slot and starts it"
    ).format(app_base, path)


def inside_dist(path):
    parts = os.path.normpath(os.path.abspath(path)).split(os.sep)
    return "dist" in parts[:-1]


# ---- synthetic images (selftest and the build-script dry run) ---------------


def synth_app(version="0.10.0", body_len=1001, app_base=APP_BASE, sp=RAM_END - 16, variant=VARIANT_PROD):
    """An unstamped application image the way objcopy would emit it: real
    vectors, the linked-in header (flags for `variant`), and a deterministic
    non-trivial body."""
    img = bytearray(RESET_OFFSET + body_len)
    struct.pack_into("<II", img, 0, sp, (app_base + RESET_OFFSET) | 1)
    for i in range(2, VECTORS_LEN // 4):
        struct.pack_into("<I", img, i * 4, (app_base + RESET_OFFSET + 0x10 * i) | 1)
    img[HEADER_OFFSET:RESET_OFFSET] = app_header_bytes(version, flags_for_variant(variant))
    for i in range(body_len):
        img[RESET_OFFSET + i] = (i * 7 + 3) & 0xFF
    return bytes(img)


def synth_boot(version="1.0.0", variant=VARIANT_PROD, body_len=5000, sp=RAM_END - 16):
    img = bytearray(RESET_OFFSET + body_len)
    struct.pack_into("<II", img, 0, sp, (BOOT_BASE + RESET_OFFSET) | 1)
    for i in range(2, VECTORS_LEN // 4):
        struct.pack_into("<I", img, i * 4, (BOOT_BASE + RESET_OFFSET + 0x10 * i) | 1)
    img[HEADER_OFFSET:RESET_OFFSET] = boot_info_bytes(version, variant)
    for i in range(body_len):
        img[RESET_OFFSET + i] = (i * 13 + 5) & 0xFF
    return bytes(img)


def pad_boot(raw):
    if len(raw) > BOOT_MAX_LEN:
        raise ImageError("bootloader is {} bytes, over the {}-byte budget".format(len(raw), BOOT_MAX_LEN))
    validate_boot(raw)
    return raw + b"\xff" * (BOOT_SIZE - len(raw))


# ---- selftest -----------------------------------------------------------------


def selftest(verbose=True):
    failures = []
    out = sys.stdout  # the CLI section below redirects stdout; results still go here

    def check(name, ok, detail=""):
        if verbose:
            print("  {}  {}{}".format("PASS" if ok else "FAIL", name, " — " + detail if detail else ""), file=out)
        checks_run[0] += 1
        if not ok:
            failures.append(name)

    checks_run = [0]

    def expect_error(name, fn, needle):
        try:
            fn()
        except ImageError as e:
            check(name, needle in str(e), str(e))
        else:
            check(name, False, "no error raised")

    # Constants: the same invariants layout/src/lib.rs asserts in its tests.
    check("APP_BASE is 0x08006000", APP_BASE == 0x08006000)
    check("app slot ends at the data pages", APP_END == DATA_BASE)
    check("slots are whole pages", BOOT_SIZE % PAGE_SIZE == 0 and APP_SIZE % PAGE_SIZE == 0)
    check("RESET_OFFSET is 0xE0", RESET_OFFSET == 0xE0)
    check("magics spell OMKA / OMKB", struct.pack("<I", APP_MAGIC) == b"OMKA" and struct.pack("<I", BOOT_MAGIC) == b"OMKB")
    check("bootloader budget is 24064", BOOT_MAX_LEN == 24064)
    check("FLAG_PROTO is bit 0", FLAG_PROTO == 1 and flags_for_variant(VARIANT_PROTO) == FLAG_PROTO and flags_for_variant(VARIANT_PROD) == 0)
    check("variant_for_flags inverts flags_for_variant",
          variant_for_flags(flags_for_variant(VARIANT_PROTO)) == VARIANT_PROTO and variant_for_flags(0) == VARIANT_PROD
          and variant_for_flags(FLAG_PROTO | 0x8000) == VARIANT_PROTO)
    check("app header carries the proto flag",
          parse_app_header(app_header_bytes("0.10.0", FLAG_PROTO))["proto"] is True
          and parse_app_header(app_header_bytes("0.10.0", FLAG_PROTO))["flags"] == FLAG_PROTO
          and parse_app_header(app_header_bytes("0.10.0"))["proto"] is False)

    # CRC: the ISO-HDLC vectors openmicro_layout::crc32 is tested against,
    # and the image rule (crc field zeroed) spelled out with plain zlib.
    check("crc32(\"123456789\") == 0xCBF43926", crc32(b"123456789") == 0xCBF43926, "0x{:08X}".format(crc32(b"123456789")))
    check("crc32(\"\") == 0", crc32(b"") == 0)
    sample = bytearray(synth_app())
    struct.pack_into("<I", sample, HEADER_OFFSET + 8, 0xDEADBEEF)
    zeroed = bytearray(sample)
    struct.pack_into("<I", zeroed, HEADER_OFFSET + 8, 0)
    check("image_crc ignores the crc field", image_crc(bytes(sample)) == crc32(bytes(zeroed)))

    # Stamp an odd-length synthetic app (exercises the 0xFF padding), verify,
    # then tamper in every way the bootloader must catch.
    raw = synth_app(body_len=1001)
    check("synthetic app is unstamped", validate_app(raw)[0] == "unstamped")
    check("synthetic app length needs padding", len(raw) % 4 != 0)
    stamped, header = stamp_app(raw)
    check("stamp pads to a word with 0xFF", len(stamped) % 4 == 0 and stamped[len(raw):] == b"\xff" * (len(stamped) - len(raw)))
    check("stamp writes the length", header["length"] == len(stamped))
    check("stamp writes the crc", header["crc"] == image_crc(stamped))
    check("stamped app verifies", validate_app(stamped)[0] == "stamped")
    check("verify_image accepts the app", "ok" in verify_image(stamped))
    check("re-stamping is a no-op", stamp_app(stamped)[0] == stamped)
    expect_error("verify_image refuses an unstamped app", lambda: verify_image(raw), "unstamped")
    check("verify_image --allow-unstamped", "UNSTAMPED" in verify_image(raw, allow_unstamped=True))

    bad = bytearray(stamped)
    bad[RESET_OFFSET + 500] ^= 1
    expect_error("flipped body byte fails the crc", lambda: validate_app(bytes(bad)), "crc mismatch")
    bad = bytearray(stamped)
    bad[0x40] ^= 1
    expect_error("flipped vector-table byte fails the crc", lambda: validate_app(bytes(bad)), "crc mismatch")
    bad = bytearray(stamped)
    struct.pack_into("<I", bad, 4, (APP_BASE + 0x1000 + RESET_OFFSET) | 1)
    expect_error("reset vector for another base is refused", lambda: validate_app(bytes(bad)), "reset vector")
    expect_error("image linked for the wrong base is refused", lambda: validate_app(stamped, APP_BASE + 0x800), "reset vector")
    bad = bytearray(stamped)
    struct.pack_into("<I", bad, 0, 0x20003FF4)
    expect_error("misaligned stack pointer is refused", lambda: validate_app(bytes(bad)), "stack pointer")
    bad = bytearray(stamped)
    struct.pack_into("<I", bad, 0, RAM_END + 4)
    expect_error("stack pointer outside RAM is refused", lambda: validate_app(bytes(bad)), "stack pointer")
    bad = bytearray(stamped)
    struct.pack_into("<H", bad, HEADER_OFFSET + 12, 2)
    expect_error("header version 2 is refused", lambda: validate_app(bytes(bad)), "header version")
    bad = bytearray(stamped)
    struct.pack_into("<I", bad, HEADER_OFFSET + 4, APP_SIZE + 4)
    expect_error("length beyond the slot is refused", lambda: validate_app(bytes(bad)), "length")
    bad = bytearray(stamped)
    struct.pack_into("<I", bad, HEADER_OFFSET + 4, len(stamped) - 2)
    expect_error("length not a multiple of 4 is refused", lambda: validate_app(bytes(bad)), "length")
    bad = bytearray(stamped)
    struct.pack_into("<I", bad, HEADER_OFFSET + 4, len(stamped) + 4)
    expect_error("length past the bytes given is refused", lambda: validate_app(bytes(bad)), "exceeds")
    bad = bytearray(stamped)
    bad[HEADER_OFFSET:HEADER_OFFSET + 4] = b"XXXX"
    expect_error("missing magic is refused", lambda: validate_app(bytes(bad)), "magic")
    expect_error("stamping needs the magic", lambda: stamp_app(b"\xff" * 0x200), "magic")
    expect_error("too-short image is refused", lambda: validate_app(b"\xff" * 0x80), "fewer")
    check("erased slot is legacy/invalid", detect(b"\xff" * 0x200) == KIND_LEGACY)

    # A whole slot with the image in front verifies (the bootloader reads
    # the full slot); trailing garbage does not pass the release gate.
    slot = stamped + b"\xff" * (APP_SIZE - len(stamped))
    check("stamped app inside a full erased slot validates", validate_app(slot)[0] == "stamped")
    check("verify_image accepts erased trailing bytes", "ok" in verify_image(slot))
    expect_error("verify_image refuses trailing data", lambda: verify_image(stamped + b"\x00\x01\x02\x03"), "follow")

    # Bootloader: budget, info block, padding.
    boot = synth_boot(version="1.0.0", variant=VARIANT_PROD, body_len=5000)
    info, used = validate_boot(boot)
    check("bootloader info block parses", info["version"] == "1.0.0" and info["variant"] == 0 and info["app_base"] == APP_BASE and info["app_size"] == APP_SIZE)
    check("bootloader used bytes", used == len(boot), str(used))
    padded = pad_boot(boot)
    check("pad-boot fills the slot with 0xFF", len(padded) == BOOT_SIZE and padded[len(boot):] == b"\xff" * (BOOT_SIZE - len(boot)))
    check("padded bootloader still validates with the same used count", validate_boot(padded)[1] == len(boot))
    check("verify_image accepts a raw bootloader", "bootloader" in verify_image(boot))
    expect_error("oversized bootloader is refused", lambda: pad_boot(synth_boot(body_len=BOOT_MAX_LEN)), "budget")
    bad = bytearray(boot)
    struct.pack_into("<H", bad, HEADER_OFFSET + 4, 2)
    expect_error("bootloader protocol 2 is refused", lambda: validate_boot(bytes(bad)), "protocol")
    bad = bytearray(boot)
    struct.pack_into("<I", bad, HEADER_OFFSET + 8, APP_BASE + PAGE_SIZE)
    expect_error("bootloader app_base mismatch is refused", lambda: validate_boot(bytes(bad)), "app_base")
    bad = bytearray(boot)
    struct.pack_into("<I", bad, HEADER_OFFSET + 12, APP_SIZE - PAGE_SIZE)
    expect_error("bootloader app_size mismatch is refused", lambda: validate_boot(bytes(bad)), "app_size")
    bad = bytearray(boot)
    struct.pack_into("<H", bad, HEADER_OFFSET + 6, 7)
    expect_error("bootloader variant 7 is refused", lambda: validate_boot(bytes(bad)), "variant")
    bad = bytearray(boot)
    struct.pack_into("<I", bad, 4, (BOOT_BASE + 0xC1))
    expect_error("bootloader reset vector must be 0x080000E1", lambda: validate_boot(bytes(bad)), "reset vector")
    check("proto bootloader variant parses", validate_boot(synth_boot(variant=VARIANT_PROTO))[0]["variant_name"] == "proto")

    # Combined image: detect, verify, describe, slice.
    combined = padded + stamped
    check("combined image detected", detect(combined) == KIND_COMBINED)
    check("raw bootloader detected", detect(boot) == KIND_BOOT and detect(padded) == KIND_BOOT)
    check("app image detected", detect(stamped) == KIND_APP and detect(raw) == KIND_APP)
    check("combined image verifies", "combined image ok" in verify_image(combined))
    check("verify --app-base agreeing with the info block", "ok" in verify_image(combined, app_base=APP_BASE))
    expect_error("verify --app-base contradicting the info block", lambda: verify_image(combined, app_base=APP_BASE + 0x800), "contradicts")
    expect_error("combined with unstamped app is refused", lambda: verify_image(padded + raw), "unstamped")
    check("combined with unstamped app passes --allow-unstamped", "UNSTAMPED" in verify_image(padded + raw, allow_unstamped=True))
    expect_error("combined image reaching the data pages is refused", lambda: verify_image(padded + stamped + b"\xff" * (APP_SIZE - len(stamped) + 4)), "data pages")
    bad = bytearray(combined)
    bad[BOOT_SIZE + RESET_OFFSET + 7] ^= 0x80
    expect_error("tampered app inside the combined image fails", lambda: verify_image(bytes(bad)), "crc mismatch")
    bad = bytearray(combined)
    bad[HEADER_OFFSET + 1] ^= 1
    check("broken info block makes the image legacy", detect(bytes(bad)) == KIND_LEGACY)
    piece, base, h = app_slice(combined)
    check("slice equals the stamped app", piece == stamped and base == APP_BASE and h == header)
    check("slice of a padded combined image is exact", app_slice(combined + b"\xff" * 100)[0] == stamped)
    check("suggested slice name", suggested_slice_name(h, base) == "openmicro-fw-0.10.0-app-slot-0x08006000.bin")
    check("dist refusal sees any dist component", inside_dist("dist/x.bin") and inside_dist("/a/dist/b/x.bin") and not inside_dist("out/dist.bin") and not inside_dist("/tmp/x/dist"))
    d = describe(combined, "synthetic")
    check("describe(combined)", d["kind"] == KIND_COMBINED and d["boot"]["version"] == "1.0.0" and d["app"]["version"] == "0.10.0"
          and d["app"]["stamped"] and d["app"]["crc_ok"] and d["app"]["valid"] and d["boot"]["valid"]
          and d["app"]["reset_vector"] == 0x080060E1 and d["boot"]["reset_vector"] == 0x080000E1
          and d["boot"]["used"] == len(boot) and d["app"]["free"] == APP_SIZE - len(stamped))
    d = describe(bytes(bad), "broken")
    check("describe never raises on a broken image", d["kind"] == KIND_LEGACY)
    d = describe(raw, "raw")
    check("describe(unstamped app)", d["app"]["validity"] == "unstamped" and d["app"]["valid"] and not d["app"]["stamped"])
    check("describe is JSON-serialisable", json.dumps(describe(combined)) is not None)

    # Pin-map agreement: the app header's proto flag against the bootloader's
    # variant. A matching pair of either kind verifies; a mixed pair is
    # refused by verify, by slice and (as app.valid false) by describe.
    proto_boot = pad_boot(synth_boot(version="1.0.0", variant=VARIANT_PROTO))
    proto_raw = synth_app(variant=VARIANT_PROTO)
    proto_app, proto_header = stamp_app(proto_raw)
    check("proto app header flags", proto_header["flags"] == FLAG_PROTO and proto_header["proto"] and not header["proto"])
    check("stamping keeps the flags", parse_app_header(proto_app[HEADER_OFFSET:])["flags"] == FLAG_PROTO)
    check("matching proto pair verifies", "combined image ok" in verify_image(proto_boot + proto_app)
          and "(proto)" in verify_image(proto_boot + proto_app))
    check("matching prod pair names its pin map", "app 0.10.0 (prod)" in verify_image(combined))
    expect_error("proto bootloader with a prod app is refused", lambda: verify_image(proto_boot + stamped), "different pin maps")
    expect_error("prod bootloader with a proto app is refused", lambda: verify_image(padded + proto_app), "different pin maps")
    expect_error("unstamped proto app on a prod bootloader is refused even with --allow-unstamped",
                 lambda: verify_image(padded + proto_raw, allow_unstamped=True), "different pin maps")
    check("unstamped proto app on a proto bootloader passes --allow-unstamped",
          "UNSTAMPED" in verify_image(proto_boot + proto_raw, allow_unstamped=True))
    expect_error("slice refuses a mixed pair", lambda: app_slice(padded + proto_app), "different pin maps")
    check("slice of the proto pair", app_slice(proto_boot + proto_app)[0] == proto_app)
    check("app-only verify names the pin map", "(proto)" in verify_image(proto_app) and "(prod)" in verify_image(stamped))
    d = describe(padded + proto_app, "mixed")
    check("describe flags a mixed pair", d["app"]["proto"] is True and d["app"]["flags"] == FLAG_PROTO and d["app"]["variant_ok"] is False
          and not d["app"]["valid"] and "different pin maps" in d["app"]["error"] and d["app"]["crc_ok"])
    d = describe(proto_boot + proto_app, "proto")
    check("describe(proto pair)", d["app"]["proto"] is True and d["app"]["variant_ok"] is True and d["app"]["valid"]
          and d["boot"]["variant_name"] == "proto")
    d = describe(combined, "prod")
    check("describe(prod pair) exposes app.flags / app.proto", d["app"]["flags"] == 0 and d["app"]["proto"] is False and d["app"]["variant_ok"] is True)
    check("describe(app only) has no variant verdict", "variant_ok" not in describe(proto_app)["app"] and describe(proto_app)["app"]["proto"] is True)
    check("slice hint has no :leave", ":leave" not in slice_flash_hint("x.bin", APP_BASE)
          and "dfu-util -d 0483:df11 -a 0 -s 0x08006000 -D x.bin" in slice_flash_hint("x.bin", APP_BASE)
          and "unplug and replug" in slice_flash_hint("x.bin", APP_BASE))

    # The command-line surface end to end, through real files. The expected
    # refusals print their reasons; keep them out of the PASS/FAIL listing.
    import contextlib
    import io

    quiet_out = io.StringIO()
    with tempfile.TemporaryDirectory() as tmp, contextlib.redirect_stdout(quiet_out), contextlib.redirect_stderr(quiet_out):
        app_path = os.path.join(tmp, "app.bin")
        with open(app_path, "wb") as f:
            f.write(raw)
        out_path = os.path.join(tmp, "app-stamped.bin")
        check("cli patch --out", main(["patch", app_path, "--out", out_path, "--quiet"]) == 0 and open(out_path, "rb").read() == stamped)
        check("cli patch refuses a stamped image", main(["patch", out_path, "--quiet"]) == 1)
        check("cli patch --force re-stamps", main(["patch", out_path, "--force", "--quiet"]) == 0 and open(out_path, "rb").read() == stamped)
        check("cli patch in place", main(["patch", app_path, "--quiet"]) == 0 and open(app_path, "rb").read() == stamped)
        legacy_path = os.path.join(tmp, "legacy.bin")
        with open(legacy_path, "wb") as f:
            f.write(b"\xf0\x3f\x00\x20\xc1\x00\x00\x08" + b"\x00" * 0x200)
        check("cli patch refuses a legacy image", main(["patch", legacy_path, "--quiet"]) == 1)
        check("cli verify legacy fails", main(["verify", legacy_path, "--quiet"]) == 1)
        boot_path = os.path.join(tmp, "boot.bin")
        with open(boot_path, "wb") as f:
            f.write(boot)
        padded_path = os.path.join(tmp, "boot-padded.bin")
        check("cli pad-boot", main(["pad-boot", boot_path, "--out", padded_path, "--quiet"]) == 0 and open(padded_path, "rb").read() == padded)
        check("cli verify boot", main(["verify", boot_path, "--quiet"]) == 0)
        combined_path = os.path.join(tmp, "openmicro-fw-0.10.0.bin")
        with open(combined_path, "wb") as f:
            f.write(open(padded_path, "rb").read() + open(out_path, "rb").read())
        check("cli verify combined", main(["verify", combined_path, "--quiet"]) == 0)
        check("cli verify combined --app-base", main(["verify", combined_path, "--app-base", "0x08006000", "--quiet"]) == 0)
        check("cli info combined", main(["info", combined_path, "--quiet"]) == 0)
        dist_dir = os.path.join(tmp, "dist")
        os.mkdir(dist_dir)
        dist_slice = os.path.join(dist_dir, "openmicro-fw-0.10.0-app-slot-0x08006000.bin")
        check("cli slice refuses dist/", main(["slice", combined_path, "--out", dist_slice, "--quiet"]) == 1 and not os.path.exists(dist_slice))
        check("cli slice --force into dist/", main(["slice", combined_path, "--out", dist_slice, "--force", "--quiet"]) == 0 and open(dist_slice, "rb").read() == stamped)
        slice_path = os.path.join(tmp, "openmicro-fw-0.10.0-app-slot-0x08006000.bin")
        check("cli slice", main(["slice", combined_path, "--out", slice_path, "--quiet"]) == 0 and open(slice_path, "rb").read() == stamped)
        check("cli verify slice", main(["verify", slice_path, "--quiet"]) == 0)
        check("cli slice passes an app-only image through", main(["slice", slice_path, "--out", slice_path + ".2", "--quiet"]) == 0
              and open(slice_path + ".2", "rb").read() == stamped)
        with open(combined_path, "r+b") as f:
            f.seek(BOOT_SIZE + RESET_OFFSET + 3)
            f.write(b"\x55")
        check("cli verify tampered combined fails", main(["verify", combined_path, "--quiet"]) == 1)
        check("cli slice tampered combined fails", main(["slice", combined_path, "--out", slice_path + ".3", "--quiet"]) == 1)
        check("cli info tampered combined still prints", main(["info", combined_path, "--quiet"]) == 0)
        json_out = io.StringIO()
        with contextlib.redirect_stdout(json_out):
            rc = main(["info", combined_path, "--json"])
        j = json.loads(json_out.getvalue())
        check("cli info --json", rc == 0 and j["kind"] == KIND_COMBINED)
        check("cli info --json exposes app.flags / app.proto", j["app"]["flags"] == 0 and j["app"]["proto"] is False and j["app"]["variant_ok"] is True)
        # The slice hint the CLI prints: never `:leave` for a slice.
        hint_out = io.StringIO()
        with open(combined_path, "wb") as f:
            f.write(combined)
        with contextlib.redirect_stdout(hint_out):
            rc = main(["slice", combined_path, "--out", slice_path + ".4"])
        check("cli slice prints the dfu-util hint without :leave", rc == 0 and ":leave" not in hint_out.getvalue()
              and "dfu-util -d 0483:df11 -a 0 -s 0x08006000 -D " + slice_path + ".4" in hint_out.getvalue()
              and "unplug and replug the pad: the bootloader validates the slot and starts it" in hint_out.getvalue())
        # A mixed pair on disk: verify and slice refuse it, info reports it.
        mixed_path = os.path.join(tmp, "openmicro-fw-0.10.0-mixed.bin")
        with open(mixed_path, "wb") as f:
            f.write(padded + proto_app)
        check("cli verify mixed pair fails", main(["verify", mixed_path, "--quiet"]) == 1)
        check("cli slice mixed pair fails", main(["slice", mixed_path, "--out", slice_path + ".5", "--quiet"]) == 1 and not os.path.exists(slice_path + ".5"))
        json_out = io.StringIO()
        with contextlib.redirect_stdout(json_out):
            rc = main(["info", mixed_path, "--json"])
        j = json.loads(json_out.getvalue())
        check("cli info --json on a mixed pair", rc == 0 and j["app"]["proto"] is True and j["app"]["variant_ok"] is False and j["app"]["valid"] is False)
        synth_dir = os.path.join(tmp, "synth")
        synth_boot_bin = os.path.join(synth_dir, "boot.bin")
        synth_app_bin = os.path.join(synth_dir, "app.bin")
        check("cli synth", main(["synth", "--out", synth_dir, "--variant", "1", "--quiet"]) == 0
              and validate_boot(open(synth_boot_bin, "rb").read())[0]["variant"] == 1
              and validate_app(open(synth_app_bin, "rb").read())[0] == "unstamped")
        check("cli synth --variant 1 flags the app too",
              parse_app_header(open(synth_app_bin, "rb").read()[HEADER_OFFSET:])["flags"] == FLAG_PROTO
              and "combined image ok" in verify_image(pad_boot(open(synth_boot_bin, "rb").read()) + stamp_app(open(synth_app_bin, "rb").read())[0]))
        check("cli synth default is a prod pair",
              main(["synth", "--out", synth_dir + "-prod", "--quiet"]) == 0
              and parse_app_header(open(os.path.join(synth_dir + "-prod", "app.bin"), "rb").read()[HEADER_OFFSET:])["flags"] == 0)

    if verbose:
        print(file=out)
        if failures:
            print("FAILED: {}".format(", ".join(failures)), file=out)
        else:
            print("all fw-image checks passed ({} checks)".format(checks_run[0]), file=out)
    return 0 if not failures else 1


# ---- command line ---------------------------------------------------------------


def read_image(path):
    try:
        with open(path, "rb") as f:
            return f.read()
    except OSError as e:
        raise ImageError("cannot read {}: {}".format(path, e.strerror))


def write_image(path, data):
    d = os.path.dirname(os.path.abspath(path))
    if not os.path.isdir(d):
        raise ImageError("output directory does not exist: {}".format(d))
    with open(path, "wb") as f:
        f.write(data)


def parse_int(s):
    return int(s, 0)


def cmd_info(args):
    image = read_image(args.file)
    d = describe(image, args.file, args.app_base)
    if args.json:
        print(json.dumps(d, indent=2, sort_keys=True))
    elif not args.quiet:
        print_description(d)
    return 0


def cmd_patch(args):
    image = read_image(args.file)
    kind = detect(image)
    if kind == KIND_COMBINED:
        raise ImageError("{} is a combined image; patch the raw application .bin (objcopy output), not the release image".format(args.file))
    if kind == KIND_BOOT:
        raise ImageError("{} is a bootloader image; it carries no application header to stamp".format(args.file))
    if kind != KIND_APP:
        raise ImageError("{}: no OMKA application header at +0x{:X}; fw <= 0.9.0 images cannot be stamped".format(args.file, HEADER_OFFSET))
    header = parse_app_header(image[HEADER_OFFSET:])
    if header["stamped"] and not args.force:
        raise ImageError("{} is already stamped (length {}, crc 0x{:08X}); use --force to re-stamp".format(args.file, header["length"], header["crc"]))
    stamped, header = stamp_app(image)
    # A stamped image that the bootloader would reject anyway (wrong link
    # address, bad stack pointer) must not leave this step looking healthy.
    base = APP_BASE if args.app_base is None else args.app_base
    validate_app(stamped, base, APP_SIZE)
    out = args.out or args.file
    write_image(out, stamped)
    if not args.quiet:
        print("{}: stamped application {} for 0x{:08X}: {} bytes (padded from {}), crc 0x{:08X}".format(
            out, header["version"], base, header["length"], len(image), header["crc"]))
    return 0


def cmd_verify(args):
    image = read_image(args.file)
    summary = verify_image(image, args.app_base, args.allow_unstamped)
    if not args.quiet:
        print("{}: {}".format(args.file, summary))
    return 0


def cmd_slice(args):
    image = read_image(args.file)
    piece, base, header = app_slice(image)
    suggested = suggested_slice_name(header, base)
    if inside_dist(args.out) and not args.force:
        raise ImageError(
            "refusing to write an application-only image under a dist/ directory: it is not flashable at "
            "0x08000000 and must never be published next to the combined image (use --force if you really mean it; "
            "suggested name: {})".format(suggested)
        )
    if "app-slot" not in os.path.basename(args.out) and not args.quiet:
        print("note: the suggested name is {} (it says where the image goes)".format(suggested), file=sys.stderr)
    write_image(args.out, piece)
    if not args.quiet:
        print("{}: application {} ({}) for 0x{:08X}, {} bytes, crc 0x{:08X}".format(
            args.out, header["version"], VARIANT_NAMES[variant_for_flags(header["flags"])], base, len(piece), header["crc"]))
        print("flash it with: " + slice_flash_hint(args.out, base))
    return 0


def cmd_pad_boot(args):
    raw = read_image(args.file)
    if detect(raw) not in (KIND_BOOT, KIND_COMBINED):
        raise ImageError("{}: no OMKB info block at +0x{:X}; not a bootloader image".format(args.file, HEADER_OFFSET))
    if detect(raw) == KIND_COMBINED:
        raise ImageError("{} is already a combined image".format(args.file))
    padded = pad_boot(raw)
    write_image(args.out, padded)
    if not args.quiet:
        info = parse_boot_info(raw[HEADER_OFFSET:])
        print("{}: bootloader {} ({}) {} bytes padded to {} ({} free of the {}-byte budget)".format(
            args.out, info["version"], info["variant_name"], len(raw), BOOT_SIZE, BOOT_MAX_LEN - len(raw), BOOT_MAX_LEN))
    return 0


def cmd_synth(args):
    """Synthetic raw boot.bin / app.bin the way objcopy would produce them, so
    scripts/test-firmware-host.sh can dry-run build-firmware.sh's assembly
    checks without a cross toolchain."""
    os.makedirs(args.out, exist_ok=True)
    boot = synth_boot(version=args.boot_version, variant=args.variant, body_len=args.boot_body)
    app = synth_app(version=args.fw_version, body_len=args.app_body, variant=args.variant)
    write_image(os.path.join(args.out, "boot.bin"), boot)
    write_image(os.path.join(args.out, "app.bin"), app)
    if not args.quiet:
        print("{}: synthetic boot.bin ({} bytes, {} {}) and unstamped app.bin ({} bytes, {} {})".format(
            args.out, len(boot), args.boot_version, VARIANT_NAMES[args.variant], len(app), args.fw_version, VARIANT_NAMES[args.variant]))
    return 0


def cmd_selftest(args):
    return selftest(verbose=not args.quiet)


def build_parser():
    p = argparse.ArgumentParser(prog="fw-image.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="command", metavar="command")
    sub.required = True

    def common(sp):
        sp.add_argument("--quiet", action="store_true", help="print nothing on success")

    s = sub.add_parser("info", help="describe an image (combined, app, boot or legacy)")
    s.add_argument("file")
    s.add_argument("--json", action="store_true", help="machine-readable output")
    s.add_argument("--app-base", type=parse_int, default=None, help="validate an app-only image against this link address (default 0x%08X)" % APP_BASE)
    common(s)
    s.set_defaults(fn=cmd_info)

    s = sub.add_parser("patch", help="pad an application .bin to a word and stamp length + crc")
    s.add_argument("file")
    s.add_argument("--out", help="write here instead of in place")
    s.add_argument("--force", action="store_true", help="re-stamp an already stamped image")
    s.add_argument("--app-base", type=parse_int, default=None, help="link address the image must be valid for (default 0x%08X)" % APP_BASE)
    common(s)
    s.set_defaults(fn=cmd_patch)

    s = sub.add_parser("verify", help="release gate: exit 1 unless the image would boot on the pad")
    s.add_argument("file")
    s.add_argument("--app-base", type=parse_int, default=None, help="expected application link address (default: the info block, or 0x%08X)" % APP_BASE)
    s.add_argument("--allow-unstamped", action="store_true", help="accept a debugger image (length/crc zero); never for a release")
    common(s)
    s.set_defaults(fn=cmd_verify)

    s = sub.add_parser("slice", help="write the application image out of a combined image")
    s.add_argument("file")
    s.add_argument("--out", required=True, help="output path; suggested: openmicro-fw-<ver>-app-slot-0x%08X.bin" % APP_BASE)
    s.add_argument("--force", action="store_true", help="allow an output path under a dist/ directory")
    common(s)
    s.set_defaults(fn=cmd_slice)

    s = sub.add_parser("pad-boot", help="check a raw bootloader .bin and pad it to the %d-byte slot" % BOOT_SIZE)
    s.add_argument("file")
    s.add_argument("--out", required=True)
    common(s)
    s.set_defaults(fn=cmd_pad_boot)

    s = sub.add_parser("synth", help="write synthetic boot.bin/app.bin for build-script dry runs")
    s.add_argument("--out", required=True, help="directory to write into")
    s.add_argument("--variant", type=int, choices=(VARIANT_PROD, VARIANT_PROTO), default=VARIANT_PROD,
                   help="pin map recorded in the info block and as the app header's proto flag (0 prod, 1 proto)")
    s.add_argument("--boot-version", default="1.0.0")
    s.add_argument("--fw-version", default="0.10.0")
    s.add_argument("--boot-body", type=int, default=5000)
    s.add_argument("--app-body", type=int, default=1001)
    common(s)
    s.set_defaults(fn=cmd_synth)

    s = sub.add_parser("selftest", help="synthetic round trip, tamper checks and CRC vectors; exit 0/1")
    common(s)
    s.set_defaults(fn=cmd_selftest)
    return p


def main(argv=None):
    args = build_parser().parse_args(argv)
    try:
        return args.fn(args)
    except ImageError as e:
        print("fw-image.py {}: {}".format(args.command, e), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
