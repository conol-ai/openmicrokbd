#!/usr/bin/env python3
"""Hardware client and round-trip test for the OpenMicro v1 resident bootloader
(firmware >= 0.10.0, bootloader >= 1.0.0).

Talks to the pad over the same raw HID interfaces the OpenMicro app uses:
the application's vendor interface (1209:0001 or 303A:8360 in Codex mode,
usage page 0xFF60, 32-byte reports) and the bootloader's (1209:0002
"OpenMicro Bootloader", 64-byte reports). No driver is needed on any OS;
on Linux the udev rule in docs/linux-firmware-updates.md must cover 1209:0002.

    python3 -m pip install hidapi                      # once
    python3 scripts/test-bootloader.py find             # what is plugged in
    python3 scripts/test-bootloader.py info             # BOOT_INFO from app or bootloader
    python3 scripts/test-bootloader.py enter-boot       # app -> bootloader (ENTER_BOOT)
    python3 scripts/test-bootloader.py upload dist/openmicro-fw-<ver>.bin
    python3 scripts/test-bootloader.py upload ... --abort-after 40   # simulate an interrupted update
    python3 scripts/test-bootloader.py run              # bootloader -> app (BOOT_RUN)
    python3 scripts/test-bootloader.py dfu              # ENTER_DFU from whichever image runs
    python3 scripts/test-bootloader.py cycle dist/openmicro-fw-<ver>.bin   # the whole round trip, PASS/FAIL
    python3 scripts/test-bootloader.py selftest         # no hardware: the client against a model of the bootloader

`upload` and `cycle` accept the published combined image (bootloader + app)
or an application slice; the application part is cut out with fw-image.py's
logic and verified locally before a byte is sent. The OpenMicro app may stay
running for `find`/`info`, but quit it before `enter-boot`/`upload`/`cycle`:
it reacts to the bootloader appearing and would race this script.

Protocol summary (layout/src/lib.rs is the authority; replies echo the opcode):
    0x01 VERSION      [0x01] -> [0x01, len, "boot X.Y.Z" | "X.Y.Z"]
    0x02 ENTER_DFU    [0x02,"DFU!"] -> [0x02, 1], then the pad resets into ROM DFU (0483:df11)
    0x12 ENTER_BOOT   [0x12,"BOOT"] -> [0x12, 1], then the app resets into the bootloader (app only)
    0x20 BOOT_INFO    [0x20] -> [0x20, status, proto, maj, min, patch, app_base u32, app_size u32,
                                  app_valid, app_version[16]] (+ page u16, max_chunk u8 from the bootloader)
    0x21 UPDATE_BEGIN [0x21, length u32, crc u32] -> [0x21, status]   (erases; up to ~1.5 s)
    0x22 UPDATE_DATA  [0x22, offset u32, n u8, data[n]] -> [0x22, status, next_offset u32]
    0x23 UPDATE_END   [0x23] -> [0x23, status]   (validates header + CRC from flash)
    0x24 BOOT_RUN     [0x24] -> [0x24, 0], then the bootloader resets and starts the app
"""

import argparse
import importlib.util
import os
import shutil
import struct
import subprocess
import sys
import time

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))


def load_fw_image():
    """scripts/fw-image.py has a hyphen in its name, so import it by path:
    it owns the layout constants, the header parser and validate_app."""
    spec = importlib.util.spec_from_file_location("fw_image", os.path.join(SCRIPT_DIR, "fw-image.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


fw_image = load_fw_image()

try:
    import hid
except ImportError:
    hid = None

# ---- identities ---------------------------------------------------------------

APP_IDENTITIES = ((0x1209, 0x0001), (0x303A, 0x8360))
RAW_USAGE_PAGE = 0xFF60
BOOT_VID, BOOT_PID = 0x1209, 0x0002
BOOT_MANUFACTURER = "conol"
BOOT_PRODUCT = "OpenMicro Bootloader"
DFU_VID, DFU_PID = 0x0483, 0xDF11

APP_REPORT_LEN = 32
BOOT_REPORT_LEN = 64

# ---- protocol -----------------------------------------------------------------

OP_VERSION = 0x01
OP_ENTER_DFU = 0x02
OP_ENTER_BOOT = 0x12
OP_BOOT_INFO = 0x20
OP_UPDATE_BEGIN = 0x21
OP_UPDATE_DATA = 0x22
OP_UPDATE_END = 0x23
OP_BOOT_RUN = 0x24
ENTER_DFU_KEY = b"DFU!"
ENTER_BOOT_KEY = b"BOOT"
# Byte 0 of an unsolicited application input event; never a reply.
EVENT_MARK = 0x80

MAX_CHUNK = 56
BOOT_INFO_LEN = 31
BOOT_INFO_EXT_LEN = 34

STATUS_OK = 0
STATUS_BAD_LENGTH = 1
STATUS_FLASH = 2
STATUS_OUT_OF_ORDER = 3
STATUS_HEADER = 4
STATUS_NO_BEGIN = 5
STATUS_CRC = 6
STATUS_UNKNOWN = 0xFF
STATUS_NAMES = {
    STATUS_OK: "ok",
    STATUS_BAD_LENGTH: "bad length",
    STATUS_FLASH: "flash error",
    STATUS_OUT_OF_ORDER: "out of order",
    STATUS_HEADER: "header invalid",
    STATUS_NO_BEGIN: "no UPDATE_BEGIN",
    STATUS_CRC: "crc mismatch",
    STATUS_UNKNOWN: "unknown opcode",
}
APP_VALID_NAMES = {0: "none", 1: "valid", 2: "unstamped"}

# Host timeouts from the design brief: BEGIN erases up to 42 pages (~1.5 s),
# DATA/END are a flash write or a CRC pass, BOOT_INFO is immediate.
TIMEOUTS = {"info": 1.0, "begin": 8.0, "data": 2.0, "end": 2.0, "reply": 0.5}
WAIT_DEVICE_S = 15.0
POLL_S = 0.25


class ProtocolError(Exception):
    pass


def status_name(st):
    return "{} ({})".format(st, STATUS_NAMES.get(st, "?"))


def cstr(raw):
    end = raw.find(b"\0")
    return (raw[:end] if end >= 0 else raw).decode("ascii", "replace")


def parse_boot_info_reply(rep):
    """The 31-byte reply both images send, plus the bootloader's page and
    max_chunk when the report is long enough to carry them."""
    if len(rep) < BOOT_INFO_LEN or rep[0] != OP_BOOT_INFO:
        return None
    app_base, app_size = struct.unpack_from("<II", rep, 6)
    info = {
        "status": rep[1],
        "protocol": rep[2],
        "boot_version": "{}.{}.{}".format(rep[3], rep[4], rep[5]),
        "app_base": app_base,
        "app_size": app_size,
        "app_valid": rep[14],
        "app_valid_name": APP_VALID_NAMES.get(rep[14], "?"),
        "app_version": cstr(bytes(rep[15:31])),
        "page": None,
        "max_chunk": None,
    }
    if len(rep) >= BOOT_INFO_EXT_LEN:
        info["page"] = struct.unpack_from("<H", rep, 31)[0]
        info["max_chunk"] = rep[33]
    return info


def format_boot_info(info):
    lines = [
        "  status {}  protocol {}  bootloader {}".format(info["status"], info["protocol"], info["boot_version"]),
        "  app_base 0x{:08X}  app_size 0x{:X}  app {} ({})  app_version {!r}".format(
            info["app_base"], info["app_size"], info["app_valid"], info["app_valid_name"], info["app_version"]),
    ]
    if info["page"] is not None:
        lines.append("  page {}  max_chunk {}".format(info["page"], info["max_chunk"]))
    return "\n".join(lines)


# ---- transports -----------------------------------------------------------------


def non_exclusive():
    """hidapi seizes the device on macOS by default, which would knock the
    OpenMicro app off the interface; the flag lives in the compiled module."""
    if sys.platform != "darwin":
        return
    try:
        import ctypes

        ctypes.CDLL(hid.__file__).hid_darwin_set_open_exclusive(0)
    except (AttributeError, OSError):
        pass


class HidTransport:
    """One raw HID interface. hidapi wants a leading report id (0: none) on
    every write, so the bootloader takes 65-byte writes and gives 64-byte
    reads; the application 33 / 32."""

    def __init__(self, info, report_len):
        self.info = info
        self.report_len = report_len
        self.dev = None

    def open(self):
        if hid is None:
            sys.exit("python hidapi missing — run: python3 -m pip install hidapi")
        non_exclusive()
        dev = hid.device()
        try:
            dev.open_path(self.info["path"])
        except (OSError, IOError, ValueError) as e:
            raise ProtocolError(
                "cannot open {} ({}); on Linux check the udev rule in docs/linux-firmware-updates.md".format(
                    describe_device(self.info), e))
        self.dev = dev
        return self

    def close(self):
        if self.dev is not None:
            self.dev.close()
            self.dev = None

    def write(self, payload):
        if len(payload) > self.report_len:
            raise ValueError("payload of {} bytes exceeds the {}-byte report".format(len(payload), self.report_len))
        self.dev.write(bytes([0]) + bytes(payload) + bytes(self.report_len - len(payload)))

    def read(self, timeout_ms):
        rep = self.dev.read(self.report_len, timeout_ms=timeout_ms)
        return bytes(rep) if rep else None


class Client:
    """Request/reply over a transport with `write(payload)` / `read(timeout_ms)`.
    Replies are matched on the echoed opcode; application input events
    (0x80+) and stale replies to earlier, timed-out requests are skipped."""

    def __init__(self, transport, timeouts=None):
        self.t = transport
        self.timeouts = dict(TIMEOUTS)
        if timeouts:
            self.timeouts.update(timeouts)

    def command(self, req, timeout_s, accept=None):
        self.t.write(req)
        deadline = time.monotonic() + timeout_s
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("no reply to opcode 0x{:02X} within {:.1f} s".format(req[0], timeout_s))
            rep = self.t.read(max(1, int(min(remaining, 0.1) * 1000)))
            if not rep or rep[0] >= EVENT_MARK or rep[0] != req[0]:
                continue
            if accept is not None and not accept(rep):
                continue
            return rep

    def version(self):
        rep = self.command(bytes([OP_VERSION]), self.timeouts["reply"])
        n = min(rep[1], len(rep) - 2)
        return rep[2 : 2 + n].decode("ascii", "replace")

    def boot_info(self):
        rep = self.command(bytes([OP_BOOT_INFO]), self.timeouts["info"])
        info = parse_boot_info_reply(rep)
        if info is None:
            raise ProtocolError("malformed BOOT_INFO reply: {}".format(rep.hex()))
        return info

    def enter_boot(self):
        rep = self.command(bytes([OP_ENTER_BOOT]) + ENTER_BOOT_KEY, self.timeouts["reply"])
        if len(rep) < 2 or rep[1] != 1:
            raise ProtocolError("ENTER_BOOT not acknowledged: {}".format(rep[:2].hex()))

    def enter_dfu(self):
        rep = self.command(bytes([OP_ENTER_DFU]) + ENTER_DFU_KEY, self.timeouts["reply"])
        if len(rep) < 2 or rep[1] != 1:
            raise ProtocolError("ENTER_DFU not acknowledged: {}".format(rep[:2].hex()))

    def update_begin(self, length, crc):
        rep = self.command(bytes([OP_UPDATE_BEGIN]) + struct.pack("<II", length, crc), self.timeouts["begin"])
        return rep[1]

    def update_data(self, offset, data):
        n = len(data)
        req = bytes([OP_UPDATE_DATA]) + struct.pack("<IB", offset, n) + bytes(data)

        def accept(rep):
            # The pad answers every request exactly once, so a reply that
            # arrives after we gave up on its request is still in the pipe.
            # A late "ok" for the previous chunk carries next_offset == this
            # chunk's offset, never offset + n. A late "out of order" can
            # only tell us to resume at an offset we did not just send: the
            # pad never rejects a chunk at the offset it expects, so
            # next_offset == offset marks it stale (without this, one late
            # reply would trigger a re-send, whose own reply is late, and so
            # on until the resync budget runs out).
            if len(rep) < 6:
                return False
            nxt = struct.unpack_from("<I", rep, 2)[0]
            if rep[1] == STATUS_OK:
                return nxt == offset + n
            if rep[1] == STATUS_OUT_OF_ORDER:
                return nxt != offset
            return True

        rep = self.command(req, self.timeouts["data"], accept)
        return rep[1], struct.unpack_from("<I", rep, 2)[0]

    def update_end(self):
        rep = self.command(bytes([OP_UPDATE_END]), self.timeouts["end"])
        return rep[1]

    def boot_run(self):
        rep = self.command(bytes([OP_BOOT_RUN]), self.timeouts["reply"])
        return rep[1]


# ---- upload -------------------------------------------------------------------


def upload(client, image, log=None, abort_after=None, chunk=MAX_CHUNK, progress=None):
    """Sends a stamped application image (exactly header.length bytes) with
    UPDATE_BEGIN / UPDATE_DATA / UPDATE_END. Returns a stats dict; raises
    ProtocolError / TimeoutError. `abort_after` stops after that many chunks
    without UPDATE_END to leave the pad with an interrupted update."""
    log = log or (lambda s: None)
    header = fw_image.parse_app_header(image[fw_image.HEADER_OFFSET :])
    if header is None or not header["stamped"] or header["length"] != len(image):
        raise ProtocolError("upload wants a stamped application image of exactly header.length bytes")
    length, crc = header["length"], header["crc"]
    if not 0 < chunk <= MAX_CHUNK or chunk % 4:
        raise ProtocolError("chunk size must be a multiple of 4 up to {}".format(MAX_CHUNK))

    t0 = time.monotonic()
    st = client.update_begin(length, crc)
    if st != STATUS_OK:
        raise ProtocolError("UPDATE_BEGIN({}, 0x{:08X}) failed: {}".format(length, crc, status_name(st)))
    t_begin = time.monotonic()
    log("UPDATE_BEGIN ok after {:.2f} s (erase)".format(t_begin - t0))

    offset = 0
    chunks = 0
    resyncs = 0
    retries = 0  # consecutive timeouts on the chunk in flight
    timeouts = 0  # total, for the report
    while offset < length:
        n = min(chunk, length - offset)
        try:
            st, nxt = client.update_data(offset, image[offset : offset + n])
        except TimeoutError:
            # The pad may have programmed the chunk and the reply was lost, or
            # it is still busy. BOOT_INFO proves it is alive; re-sending the
            # chunk then either lands it or yields "out of order" with the
            # offset the pad expects, and the loop resumes from there.
            retries += 1
            timeouts += 1
            if retries > 3:
                raise
            log("no reply for chunk at {}; re-syncing with BOOT_INFO (retry {})".format(offset, retries))
            client.boot_info()
            continue
        if st == STATUS_OUT_OF_ORDER:
            resyncs += 1
            if resyncs > 8 or nxt > length or nxt % 4:
                raise ProtocolError("cannot resync: pad expects offset {} (we sent {})".format(nxt, offset))
            log("pad expects offset {} (we sent {}); resuming there".format(nxt, offset))
            offset = nxt
            continue
        if st != STATUS_OK:
            raise ProtocolError("UPDATE_DATA at {} failed: {}".format(offset, status_name(st)))
        if nxt != offset + n:
            raise ProtocolError("UPDATE_DATA at {} acknowledged with next_offset {}".format(offset, nxt))
        offset = nxt
        chunks += 1
        retries = 0
        if progress:
            progress(offset, length)
        if abort_after is not None and chunks >= abort_after:
            log("aborting after {} chunks ({} of {} bytes) as requested; no UPDATE_END".format(chunks, offset, length))
            return {"aborted": True, "chunks": chunks, "bytes": offset, "length": length, "resyncs": resyncs, "timeouts": timeouts}
    t_data = time.monotonic()

    st = client.update_end()
    if st != STATUS_OK:
        raise ProtocolError("UPDATE_END failed: {}".format(status_name(st)))
    t_end = time.monotonic()
    seconds = t_end - t0
    data_s = t_data - t_begin
    stats = {
        "aborted": False,
        "chunks": chunks,
        "bytes": length,
        "length": length,
        "crc": crc,
        "seconds": seconds,
        "begin_s": t_begin - t0,
        "data_s": data_s,
        "end_s": t_end - t_data,
        "bytes_per_s": length / data_s if data_s > 0 else float("inf"),
        "resyncs": resyncs,
        "timeouts": timeouts,
    }
    log("UPDATE_END ok: {} bytes in {} chunks, {:.2f} s total (erase {:.2f} s, data {:.2f} s = {:.0f} B/s, verify {:.2f} s){}".format(
        length, chunks, seconds, stats["begin_s"], data_s, stats["bytes_per_s"], stats["end_s"],
        "; {} timeouts, {} resyncs".format(timeouts, resyncs) if timeouts or resyncs else ""))
    return stats


def load_app_slice(path):
    """The stamped application image out of a combined or app-only file,
    verified locally the way the bootloader will verify it."""
    with open(path, "rb") as f:
        image = f.read()
    kind = fw_image.detect(image)
    if kind == fw_image.KIND_LEGACY:
        raise ProtocolError("{}: legacy image without a header (fw <= 0.9.0); the bootloader cannot take it".format(path))
    if kind == fw_image.KIND_BOOT:
        raise ProtocolError("{}: bootloader-only image; the bootloader cannot update itself over HID (use ENTER_DFU + the combined image)".format(path))
    try:
        piece, base, header = fw_image.app_slice(image)
    except fw_image.ImageError as e:
        raise ProtocolError("{}: {}".format(path, e))
    return piece, base, header, kind


# ---- discovery ------------------------------------------------------------------


def describe_device(info):
    return "{:04x}:{:04x} {!r} / {!r} serial {} release 0x{:04x} usage {:04x}:{:02x}".format(
        info.get("vendor_id", 0), info.get("product_id", 0), info.get("manufacturer_string"),
        info.get("product_string"), info.get("serial_number"), info.get("release_number", 0) or 0,
        info.get("usage_page", 0) or 0, info.get("usage", 0) or 0)


def enumerate_pads(serial=None):
    """(app-mode pads, bootloader pads, unrecognised 1209:0002 entries)."""
    if hid is None:
        sys.exit("python hidapi missing — run: python3 -m pip install hidapi")
    apps, boots, odd = [], [], []
    for info in hid.enumerate():
        ids = (info["vendor_id"], info["product_id"])
        if serial and info.get("serial_number") not in (serial, None):
            continue
        if ids in APP_IDENTITIES and info.get("usage_page") == RAW_USAGE_PAGE:
            apps.append(info)
        elif ids == (BOOT_VID, BOOT_PID):
            if info.get("manufacturer_string") == BOOT_MANUFACTURER and info.get("product_string") == BOOT_PRODUCT:
                boots.append(info)
            else:
                odd.append(info)
    # Some backends list one entry per collection; prefer the raw one.
    raw = [b for b in boots if b.get("usage_page") == RAW_USAGE_PAGE]
    if raw:
        boots = raw
    return apps, boots, odd


def wait_for(kind, serial=None, timeout_s=WAIT_DEVICE_S, log=print):
    """Polls until an app-mode ("app") or bootloader ("boot") pad appears;
    with a serial only that pad counts, others are reported at the end."""
    deadline = time.monotonic() + timeout_s
    seen_other = set()
    while time.monotonic() < deadline:
        apps, boots, odd = enumerate_pads()
        candidates = apps if kind == "app" else boots
        for c in candidates:
            if serial and c.get("serial_number") != serial:
                seen_other.add(c.get("serial_number"))
                continue
            return c
        if kind == "boot" and odd:
            log("note: 1209:0002 present but not readable as {!r}/{!r}: {}".format(
                BOOT_MANUFACTURER, BOOT_PRODUCT, "; ".join(describe_device(o) for o in odd)))
            log("      on Linux this usually means the udev rule for 1209:0002 is missing")
            time.sleep(POLL_S * 4)
            continue
        time.sleep(POLL_S)
    extra = " (saw serial(s) {})".format(", ".join(sorted(str(s) for s in seen_other))) if seen_other else ""
    raise TimeoutError("no {} pad{} within {:.0f} s{}".format(
        "application-mode" if kind == "app" else "bootloader", " with serial {}".format(serial) if serial else "", timeout_s, extra))


def open_app(info):
    return Client(HidTransport(info, APP_REPORT_LEN).open())


def open_boot(info):
    return Client(HidTransport(info, BOOT_REPORT_LEN).open())


def dfu_devices():
    """Lines of `dfu-util -l` naming 0483:df11, or None when dfu-util is missing."""
    if shutil.which("dfu-util") is None:
        return None
    try:
        out = subprocess.run(["dfu-util", "-l"], capture_output=True, text=True, timeout=15).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    return [line.strip() for line in out.splitlines() if "0483:df11" in line.lower()]


# ---- commands ---------------------------------------------------------------------


def cmd_find(args):
    apps, boots, odd = enumerate_pads(args.serial)
    for a in apps:
        print("app        {}".format(describe_device(a)))
    for b in boots:
        print("bootloader {}".format(describe_device(b)))
    for o in odd:
        print("unreadable {}  (1209:0002 without the expected strings: permissions?)".format(describe_device(o)))
    dfu = dfu_devices()
    if dfu:
        for line in dfu:
            print("rom-dfu    {}".format(line))
    if not apps and not boots and not odd and not dfu:
        print("no OpenMicro pad found (app 1209:0001 / 303a:8360 usage page 0xFF60, bootloader 1209:0002)")
        return 1
    return 0


def cmd_info(args):
    apps, boots, _ = enumerate_pads(args.serial)
    if not apps and not boots:
        print("no pad found; run `find`")
        return 1
    rc = 0
    for kind, infos, opener in (("bootloader", boots, open_boot), ("app", apps, open_app)):
        for info in infos:
            print("{} {}".format(kind, describe_device(info)))
            client = opener(info)
            try:
                print("  VERSION: {!r}".format(client.version()))
                bi = client.boot_info()
                print(format_boot_info(bi))
                if bi["status"] != 0:
                    print("  (status {}: the application found no bootloader info block at 0x080000C0)".format(bi["status"]))
                    rc = 1
            except (ProtocolError, TimeoutError) as e:
                print("  error: {}".format(e))
                rc = 1
            finally:
                client.t.close()
    return rc


def cmd_enter_boot(args):
    apps, boots, _ = enumerate_pads(args.serial)
    if not apps:
        if boots:
            print("already in bootloader mode: {}".format(describe_device(boots[0])))
            return 0
        print("no application-mode pad found")
        return 1
    info = apps[0]
    serial = info.get("serial_number")
    print("app {}".format(describe_device(info)))
    client = open_app(info)
    try:
        client.enter_boot()
    finally:
        client.t.close()
    print("ENTER_BOOT acknowledged; waiting for the bootloader (serial {})...".format(serial))
    boot = wait_for("boot", serial or None)
    print("bootloader {}".format(describe_device(boot)))
    client = open_boot(boot)
    try:
        print(format_boot_info(client.boot_info()))
    finally:
        client.t.close()
    return 0


def cmd_upload(args):
    piece, base, header, kind = load_app_slice(args.file)
    print("{}: {} image; application {} for 0x{:08X}, {} bytes, crc 0x{:08X}".format(
        args.file, kind, header["version"], base, len(piece), header["crc"]))
    _, boots, _ = enumerate_pads(args.serial)
    if not boots:
        print("no bootloader pad found; run `enter-boot` first (or hold the encoder switch while plugging in)")
        return 1
    info = boots[0]
    print("bootloader {}".format(describe_device(info)))
    client = open_boot(info)
    try:
        bi = client.boot_info()
        print(format_boot_info(bi))
        if bi["protocol"] != 1:
            raise ProtocolError("bootloader protocol {} is not 1".format(bi["protocol"]))
        if bi["app_base"] != base:
            raise ProtocolError("image is linked for 0x{:08X} but the pad's app slot is 0x{:08X}".format(base, bi["app_base"]))
        if len(piece) > bi["app_size"]:
            raise ProtocolError("image ({} bytes) exceeds the pad's app slot ({} bytes)".format(len(piece), bi["app_size"]))
        chunk = min(args.chunk, bi["max_chunk"] or MAX_CHUNK)
        last = [0]

        def progress(done, total):
            pct = done * 100 // total
            if pct // 10 != last[0] // 10:
                print("  {:3d}%  {} / {} bytes".format(pct, done, total))
            last[0] = pct

        stats = upload(client, piece, log=lambda s: print("  " + s), abort_after=args.abort_after, chunk=chunk, progress=progress)
        bi = client.boot_info()
        print("after upload:")
        print(format_boot_info(bi))
        if stats["aborted"]:
            print("interrupted on purpose: app_valid should read 0 and the pad stays in the bootloader until a full upload")
            return 0
        if bi["app_valid"] != 1 or bi["app_version"] != header["version"]:
            raise ProtocolError("pad reports app {} version {!r} after a successful UPDATE_END".format(
                bi["app_valid_name"], bi["app_version"]))
        print("throughput: {:.0f} B/s over UPDATE_DATA, {:.2f} s end to end".format(stats["bytes_per_s"], stats["seconds"]))
        return 0
    finally:
        client.t.close()


def cmd_run(args):
    _, boots, _ = enumerate_pads(args.serial)
    if not boots:
        print("no bootloader pad found")
        return 1
    info = boots[0]
    serial = info.get("serial_number")
    print("bootloader {}".format(describe_device(info)))
    client = open_boot(info)
    try:
        st = client.boot_run()
        if st != STATUS_OK:
            raise ProtocolError("BOOT_RUN refused: {}".format(status_name(st)))
    finally:
        client.t.close()
    print("BOOT_RUN acknowledged; waiting for the application (serial {})...".format(serial))
    app = wait_for("app", serial or None)
    print("app {}".format(describe_device(app)))
    client = open_app(app)
    try:
        print("  VERSION: {!r}".format(client.version()))
    finally:
        client.t.close()
    return 0


def cmd_dfu(args):
    apps, boots, _ = enumerate_pads(args.serial)
    if boots:
        kind, info, opener = "bootloader", boots[0], open_boot
    elif apps:
        kind, info, opener = "app", apps[0], open_app
    else:
        print("no pad found")
        return 1
    print("{} {}".format(kind, describe_device(info)))
    client = opener(info)
    try:
        client.enter_dfu()
    finally:
        client.t.close()
    print("ENTER_DFU acknowledged; waiting for the ROM DFU device 0483:df11...")
    deadline = time.monotonic() + WAIT_DEVICE_S
    while time.monotonic() < deadline:
        found = dfu_devices()
        if found is None:
            print("dfu-util is not installed; check for 0483:df11 with lsusb / system_profiler SPUSBDataType")
            return 0
        if found:
            for line in found:
                print("  " + line)
            print("flash the COMBINED image back with: dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave -D openmicro-fw-<ver>.bin")
            return 0
        time.sleep(POLL_S * 2)
    print("no 0483:df11 device within {:.0f} s".format(WAIT_DEVICE_S))
    return 1


class Checks:
    def __init__(self):
        self.failures = []

    def __call__(self, name, ok, detail=""):
        print("  {}  {}{}".format("PASS" if ok else "FAIL", name, " — " + detail if detail else ""))
        if not ok:
            self.failures.append(name)
        return ok


def cmd_cycle(args):
    """The whole round trip: app -> bootloader -> upload -> app, with the
    same PASS/FAIL style as scripts/test-codex-compat.py."""
    check = Checks()
    piece, base, header, kind = load_app_slice(args.file)
    print("{}: {} image; application {} for 0x{:08X}, {} bytes, crc 0x{:08X}".format(
        args.file, kind, header["version"], base, len(piece), header["crc"]))

    apps, boots, _ = enumerate_pads(args.serial)
    serial = None
    if apps:
        info = apps[0]
        serial = info.get("serial_number") or None
        print("app {}".format(describe_device(info)))
        client = open_app(info)
        try:
            before = client.version()
            check("app answers VERSION", bool(before), repr(before))
            bi = client.boot_info()
            check("app answers BOOT_INFO with status 0", bi["status"] == 0, format_boot_info(bi).strip())
            check("app reports app_valid 1 or 2", bi["app_valid"] in (1, 2), bi["app_valid_name"])
            check("app's app_base matches the image", bi["app_base"] == base, "0x{:08X}".format(bi["app_base"]))
            client.enter_boot()
            check("ENTER_BOOT acknowledged", True)
        except (ProtocolError, TimeoutError) as e:
            check("application phase", False, str(e))
        finally:
            client.t.close()
        t0 = time.monotonic()
        try:
            boot = wait_for("boot", serial)
            check("bootloader enumerates after ENTER_BOOT", True, "{:.1f} s, {}".format(time.monotonic() - t0, describe_device(boot)))
        except TimeoutError as e:
            check("bootloader enumerates after ENTER_BOOT", False, str(e))
            return finish(check)
    elif boots:
        boot = boots[0]
        serial = boot.get("serial_number") or None
        print("bootloader already present: {}".format(describe_device(boot)))
    else:
        print("no pad found")
        return 1

    client = open_boot(boot)
    try:
        v = client.version()
        check("bootloader VERSION starts with 'boot '", v.startswith("boot "), repr(v))
        bi = client.boot_info()
        check("bootloader BOOT_INFO status 0 / protocol 1", bi["status"] == 0 and bi["protocol"] == 1, format_boot_info(bi).strip())
        check("bootloader max_chunk is 56 and page 2048", bi["max_chunk"] == MAX_CHUNK and bi["page"] == fw_image.PAGE_SIZE)
        check("bootloader app_base/app_size match the image", bi["app_base"] == base and bi["app_size"] >= len(piece))
        stats = upload(client, piece, log=lambda s: print("    " + s), chunk=min(args.chunk, bi["max_chunk"] or MAX_CHUNK))
        check("upload completes (BEGIN/DATA/END all status 0)", not stats["aborted"],
              "{} chunks, {:.0f} B/s, {} resyncs".format(stats["chunks"], stats["bytes_per_s"], stats["resyncs"]))
        bi = client.boot_info()
        check("BOOT_INFO after upload: app_valid 1", bi["app_valid"] == 1, bi["app_valid_name"])
        check("BOOT_INFO after upload: version matches the image", bi["app_version"] == header["version"], repr(bi["app_version"]))
        st = client.boot_run()
        check("BOOT_RUN acknowledged", st == STATUS_OK, status_name(st))
    except (ProtocolError, TimeoutError) as e:
        check("bootloader phase", False, str(e))
        return finish(check)
    finally:
        client.t.close()

    t0 = time.monotonic()
    try:
        app = wait_for("app", serial)
        check("application enumerates after BOOT_RUN", True, "{:.1f} s, {}".format(time.monotonic() - t0, describe_device(app)))
    except TimeoutError as e:
        check("application enumerates after BOOT_RUN", False, str(e))
        return finish(check)
    client = open_app(app)
    try:
        after = client.version()
        check("application VERSION equals the image's header version", after == header["version"], "{!r} vs {!r}".format(after, header["version"]))
        bi = client.boot_info()
        check("application BOOT_INFO reports app_valid 1", bi["app_valid"] == 1, bi["app_valid_name"])
    except (ProtocolError, TimeoutError) as e:
        check("application after update", False, str(e))
    finally:
        client.t.close()
    return finish(check)


def finish(check):
    print()
    if check.failures:
        print("FAILED: {}".format(", ".join(check.failures)))
        return 1
    print("all checks passed")
    return 0


# ---- selftest: the client against a model of boot/src/update.rs ---------------


class FakeBootloader:
    """In-process model of the bootloader's update state machine, close
    enough to test the client's chunking, matching and resync logic:
    UPDATE_BEGIN erases whole pages, UPDATE_DATA must arrive in order,
    UPDATE_END validates with fw_image.validate_app (the same rules the
    pad applies). Link faults, keyed by the 0-based count of DATA requests
    seen: `drop_data_replies` swallows the reply, `dup_data_replies` re-sends
    the previous DATA reply in front of the real one, and
    `late_data_replies` {k: writes} holds the reply back until that many
    further requests have been written (a reply arriving after the host
    gave up on it)."""

    def __init__(self, drop_data_replies=(), dup_data_replies=(), late_data_replies=None, app_size=fw_image.APP_SIZE):
        self.app_size = app_size
        self.slot = bytearray(b"\xff" * app_size)
        self.begun = False
        self.expected = 0
        self.length = 0
        self.crc = 0
        self.queue = []
        self.data_count = 0
        self.last_data_reply = None
        self.drop = set(drop_data_replies)
        self.dup = set(dup_data_replies)
        self.late = dict(late_data_replies or {})
        self.held = []  # [remaining writes, reply]
        self.log = []
        self.reset_requested = None

    # transport side
    def write(self, payload):
        for h in self.held:
            h[0] -= 1
        released = [h[1] for h in self.held if h[0] <= 0]
        self.held = [h for h in self.held if h[0] > 0]
        self.queue.extend(released)
        rep = self.handle(bytes(payload))
        if payload[0] == OP_UPDATE_DATA:
            k = self.data_count
            self.data_count += 1
            if k in self.dup and self.last_data_reply is not None:
                self.queue.append(self.last_data_reply)
            self.last_data_reply = rep
            if k in self.drop:
                return
            if k in self.late:
                self.held.append([self.late[k], rep])
                return
        self.queue.append(rep)

    def read(self, timeout_ms):
        if self.queue:
            return self.queue.pop(0)
        time.sleep(timeout_ms / 1000.0)
        return None

    # protocol side
    def app_valid(self):
        try:
            validity, _ = fw_image.validate_app(bytes(self.slot), fw_image.APP_BASE, self.app_size)
        except fw_image.ImageError:
            return 0
        return 1 if validity == "stamped" else 2

    def handle(self, req):
        op = req[0]
        self.log.append(op)
        if op == OP_VERSION:
            return bytes([op, 10]) + b"boot 1.0.0"
        if op == OP_ENTER_DFU:
            if req[1:5] != ENTER_DFU_KEY:
                return bytes([op, 0])
            self.reset_requested = "dfu"
            return bytes([op, 1])
        if op == OP_BOOT_INFO:
            valid = self.app_valid()
            header = fw_image.parse_app_header(bytes(self.slot[fw_image.HEADER_OFFSET :]))
            version = fw_image.version_bytes(header["version"]) if valid and header else b"\0" * 16
            return (bytes([op, 0, 1, 1, 0, 0]) + struct.pack("<II", fw_image.APP_BASE, self.app_size)
                    + bytes([valid]) + version + struct.pack("<H", fw_image.PAGE_SIZE) + bytes([MAX_CHUNK]))
        if op == OP_UPDATE_BEGIN:
            length, crc = struct.unpack_from("<II", req, 1)
            if length < fw_image.RESET_OFFSET or length > self.app_size or length % 4:
                return bytes([op, STATUS_BAD_LENGTH])
            pages = (length + fw_image.PAGE_SIZE - 1) // fw_image.PAGE_SIZE
            self.slot[: pages * fw_image.PAGE_SIZE] = b"\xff" * (pages * fw_image.PAGE_SIZE)
            self.begun, self.expected, self.length, self.crc = True, 0, length, crc
            return bytes([op, STATUS_OK])
        if op == OP_UPDATE_DATA:
            if not self.begun:
                return bytes([op, STATUS_NO_BEGIN]) + struct.pack("<I", 0)
            offset, n = struct.unpack_from("<IB", req, 1)
            data = req[6 : 6 + n]
            if n == 0 or n > MAX_CHUNK or len(data) != n or (n % 4 and offset + n != self.length) or offset + n > self.length:
                return bytes([op, STATUS_BAD_LENGTH]) + struct.pack("<I", self.expected)
            if offset != self.expected:
                return bytes([op, STATUS_OUT_OF_ORDER]) + struct.pack("<I", self.expected)
            self.slot[offset : offset + n] = data
            self.expected += n
            return bytes([op, STATUS_OK]) + struct.pack("<I", self.expected)
        if op == OP_UPDATE_END:
            if not self.begun:
                return bytes([op, STATUS_NO_BEGIN])
            self.begun = False
            try:
                validity, header = fw_image.validate_app(bytes(self.slot), fw_image.APP_BASE, self.app_size)
            except fw_image.ImageError as e:
                return bytes([op, STATUS_CRC if "crc" in str(e) else STATUS_HEADER])
            if validity != "stamped" or header["crc"] != self.crc:
                return bytes([op, STATUS_CRC])
            return bytes([op, STATUS_OK])
        if op == OP_BOOT_RUN:
            self.reset_requested = "run"
            return bytes([op, STATUS_OK])
        return bytes([op, STATUS_UNKNOWN])


class FakeApp:
    """Enough of the application's raw interface to test event skipping and
    ENTER_BOOT: an input event is queued in front of every reply."""

    def __init__(self):
        self.queue = []

    def write(self, payload):
        op = payload[0]
        self.queue.append(bytes([EVENT_MARK, 0, 3, 1]))
        if op == OP_VERSION:
            self.queue.append(bytes([op, 6]) + b"0.10.0")
        elif op == OP_BOOT_INFO:
            self.queue.append(bytes([op, 0, 1, 1, 0, 0]) + struct.pack("<II", fw_image.APP_BASE, fw_image.APP_SIZE)
                              + bytes([1]) + fw_image.version_bytes("0.10.0"))
        elif op == OP_ENTER_BOOT:
            self.queue.append(bytes([op, 1 if bytes(payload[1:5]) == ENTER_BOOT_KEY else 0]))
        else:
            # No reply to this opcode, only a leftover VERSION reply from an
            # earlier exchange: it must not be taken for the answer.
            self.queue.append(bytes([OP_VERSION, 6]) + b"0.10.0")

    def read(self, timeout_ms):
        return self.queue.pop(0) if self.queue else None


def cmd_selftest(args):
    check = Checks()
    fast = {"info": 0.05, "begin": 0.2, "data": 0.05, "end": 0.1, "reply": 0.05}
    raw = fw_image.synth_app(version="0.10.0", body_len=3001)
    stamped, header = fw_image.stamp_app(raw)
    boot_padded = fw_image.pad_boot(fw_image.synth_boot())
    combined = boot_padded + stamped

    # Reply decoding for both report sizes.
    fb = FakeBootloader()
    rep = fb.handle(bytes([OP_BOOT_INFO]))
    info = parse_boot_info_reply(rep)
    check("BOOT_INFO 34-byte reply decodes", info is not None and info["page"] == 2048 and info["max_chunk"] == 56
          and info["app_valid"] == 0 and info["boot_version"] == "1.0.0" and info["app_base"] == fw_image.APP_BASE)
    info = parse_boot_info_reply(rep[:31] + b"\0")
    check("BOOT_INFO 31-byte (app) reply decodes without the extension", info is not None and info["page"] is None)
    check("short BOOT_INFO reply is rejected", parse_boot_info_reply(rep[:30]) is None)

    # Happy path through the model.
    fb = FakeBootloader()
    client = Client(fb, fast)
    stats = upload(client, stamped, chunk=MAX_CHUNK)
    check("upload succeeds", not stats["aborted"] and stats["chunks"] == (len(stamped) + MAX_CHUNK - 1) // MAX_CHUNK, str(stats["chunks"]))
    check("model slot holds the image", bytes(fb.slot[: len(stamped)]) == stamped)
    check("BOOT_INFO after upload: valid, version", client.boot_info()["app_valid"] == 1 and client.boot_info()["app_version"] == "0.10.0")
    check("last chunk is short", len(stamped) % MAX_CHUNK != 0, str(len(stamped) % MAX_CHUNK))
    check("BOOT_RUN acknowledged", client.boot_run() == 0 and fb.reset_requested == "run")
    check("VERSION reply", client.version() == "boot 1.0.0")
    client.enter_dfu()
    check("ENTER_DFU acknowledged", fb.reset_requested == "dfu")

    # Lost replies: the chunk was programmed, the host must not re-send it
    # blindly (the model would answer out-of-order) but resume from the
    # offset the pad reports.
    # Requests 5 and 6 are the same chunk (6 is the re-send after 5's reply
    # was lost), so two lost replies in a row cost two timeouts and one
    # resync; request 20 costs one of each.
    fb = FakeBootloader(drop_data_replies=(5, 6, 20))
    stats = upload(Client(fb, fast), stamped)
    check("upload survives dropped replies", not stats["aborted"] and bytes(fb.slot[: len(stamped)]) == stamped,
          "timeouts {} resyncs {}".format(stats["timeouts"], stats["resyncs"]))
    # A chunk whose reply was lost lands through the resync, so it is not
    # among the acknowledged ones: acknowledged + resynced == all chunks.
    check("dropped replies cost timeouts and resyncs, not data", stats["timeouts"] == 3 and stats["resyncs"] == 2
          and stats["chunks"] + stats["resyncs"] == (len(stamped) + MAX_CHUNK - 1) // MAX_CHUNK)
    check("re-sync used BOOT_INFO", fb.log.count(OP_BOOT_INFO) == 3)
    try:
        upload(Client(FakeBootloader(drop_data_replies=(8, 9, 10, 11)), fast), stamped)
        check("four lost replies in a row give up", False)
    except TimeoutError:
        check("four lost replies in a row give up", True)

    # Duplicated replies must be discarded by the next_offset match.
    fb = FakeBootloader(dup_data_replies=(3, 4, 10))
    stats = upload(Client(fb, fast), stamped)
    check("upload ignores duplicated stale replies", not stats["aborted"] and bytes(fb.slot[: len(stamped)]) == stamped
          and stats["resyncs"] == 0 and stats["timeouts"] == 0)

    # Late replies: one released by the BOOT_INFO re-sync (the re-sent chunk
    # is then answered out-of-order, and the host resumes), one released
    # only after the re-send (its "ok" is accepted for the re-send, and the
    # re-send's own out-of-order reply must then be recognised as stale).
    fb = FakeBootloader(late_data_replies={7: 1, 15: 2})
    stats = upload(Client(fb, fast), stamped)
    check("upload survives late replies", not stats["aborted"] and bytes(fb.slot[: len(stamped)]) == stamped,
          "timeouts {} resyncs {}".format(stats["timeouts"], stats["resyncs"]))
    check("late replies never trigger a resync cascade", stats["timeouts"] == 2 and stats["resyncs"] == 1
          and stats["chunks"] + stats["resyncs"] == (len(stamped) + MAX_CHUNK - 1) // MAX_CHUNK)

    # Interrupted update: partial slot is invalid; a fresh upload recovers.
    fb = FakeBootloader()
    client = Client(fb, fast)
    stats = upload(client, stamped, abort_after=10)
    check("--abort-after stops without UPDATE_END", stats["aborted"] and stats["chunks"] == 10 and fb.begun)
    check("partial slot reads app_valid 0", client.boot_info()["app_valid"] == 0)
    stats = upload(client, stamped)
    check("full upload after an interrupted one succeeds", not stats["aborted"] and client.boot_info()["app_valid"] == 1)

    # Refusals the pad must make and the client must surface.
    fb = FakeBootloader()
    client = Client(fb, fast)
    st, nxt = client.update_data(0, stamped[:56])
    check("DATA before BEGIN -> no UPDATE_BEGIN", st == STATUS_NO_BEGIN, status_name(st))
    st = client.update_begin(len(stamped) + 2, header["crc"])
    check("BEGIN with a length that is not a word multiple -> bad length", st == STATUS_BAD_LENGTH, status_name(st))
    check("BEGIN with a length beyond the slot -> bad length", client.update_begin(fw_image.APP_SIZE + 4, 0) == STATUS_BAD_LENGTH)
    tampered = bytearray(stamped)
    tampered[fw_image.RESET_OFFSET + 100] ^= 1
    try:
        upload(Client(FakeBootloader(), fast), bytes(tampered))
        check("tampered body -> UPDATE_END crc mismatch", False)
    except ProtocolError as e:
        check("tampered body -> UPDATE_END crc mismatch", "crc mismatch" in str(e), str(e))
    try:
        upload(Client(FakeBootloader(), fast), raw)
        check("unstamped image is refused before sending", False)
    except ProtocolError as e:
        check("unstamped image is refused before sending", "stamped" in str(e), str(e))
    try:
        upload(Client(FakeBootloader(), fast), stamped, chunk=30)
        check("chunk size must be a word multiple", False)
    except ProtocolError:
        check("chunk size must be a word multiple", True)
    stats = upload(Client(FakeBootloader(), fast), stamped, chunk=8)
    check("small chunks work", not stats["aborted"])

    # Image handling shared with fw-image.py.
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        p_comb = os.path.join(tmp, "openmicro-fw-0.10.0.bin")
        with open(p_comb, "wb") as f:
            f.write(combined)
        piece, base, h, kind = load_app_slice(p_comb)
        check("load_app_slice cuts the app out of a combined image", piece == stamped and base == fw_image.APP_BASE and kind == "combined")
        p_app = os.path.join(tmp, "app.bin")
        with open(p_app, "wb") as f:
            f.write(stamped)
        check("load_app_slice passes an app image through", load_app_slice(p_app)[0] == stamped)
        p_boot = os.path.join(tmp, "boot.bin")
        with open(p_boot, "wb") as f:
            f.write(boot_padded)
        try:
            load_app_slice(p_boot)
            check("load_app_slice refuses a bootloader image", False)
        except ProtocolError as e:
            check("load_app_slice refuses a bootloader image", "bootloader" in str(e))
        p_legacy = os.path.join(tmp, "legacy.bin")
        with open(p_legacy, "wb") as f:
            f.write(b"\xf0\x3f\x00\x20\xc1\x00\x00\x08" + b"\0" * 300)
        try:
            load_app_slice(p_legacy)
            check("load_app_slice refuses a legacy image", False)
        except ProtocolError as e:
            check("load_app_slice refuses a legacy image", "legacy" in str(e))
        with open(p_comb, "r+b") as f:
            f.seek(fw_image.BOOT_SIZE + fw_image.RESET_OFFSET + 9)
            f.write(b"\xaa")
        try:
            load_app_slice(p_comb)
            check("load_app_slice refuses a tampered image", False)
        except ProtocolError as e:
            check("load_app_slice refuses a tampered image", "crc" in str(e))

    # The application side: events and unrelated replies are skipped.
    app = Client(FakeApp(), fast)
    check("app VERSION through input events", app.version() == "0.10.0")
    check("app BOOT_INFO through input events", app.boot_info()["app_valid"] == 1)
    app.enter_boot()
    check("ENTER_BOOT ack", True)
    try:
        app.boot_run()
        check("stale reply for another opcode is never matched", False)
    except TimeoutError:
        check("stale reply for another opcode is never matched", True)

    return finish(check)


# ---- command line -------------------------------------------------------------------


def build_parser():
    p = argparse.ArgumentParser(prog="test-bootloader.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--serial", help="only talk to the pad with this USB serial")
    sub = p.add_subparsers(dest="command", metavar="command")
    sub.required = True

    s = sub.add_parser("find", help="list app-mode pads, bootloader pads and ROM DFU devices")
    s.set_defaults(fn=cmd_find)
    s = sub.add_parser("info", help="VERSION + BOOT_INFO from whatever is present")
    s.set_defaults(fn=cmd_info)
    s = sub.add_parser("enter-boot", help="ENTER_BOOT on the app, wait for the bootloader, print BOOT_INFO")
    s.set_defaults(fn=cmd_enter_boot)
    s = sub.add_parser("upload", help="send the application image to a pad in bootloader mode")
    s.add_argument("file", help="combined openmicro-fw-<ver>.bin or an application slice")
    s.add_argument("--abort-after", type=int, metavar="N", help="stop after N chunks without UPDATE_END (interrupted-update test)")
    s.add_argument("--chunk", type=int, default=MAX_CHUNK, help="bytes per UPDATE_DATA (default and max %d)" % MAX_CHUNK)
    s.set_defaults(fn=cmd_upload)
    s = sub.add_parser("run", help="BOOT_RUN, wait for the application, print its version")
    s.set_defaults(fn=cmd_run)
    s = sub.add_parser("dfu", help="ENTER_DFU on whichever image runs, wait for 0483:df11")
    s.set_defaults(fn=cmd_dfu)
    s = sub.add_parser("cycle", help="enter-boot -> info -> upload -> run -> version check, PASS/FAIL")
    s.add_argument("file")
    s.add_argument("--chunk", type=int, default=MAX_CHUNK)
    s.set_defaults(fn=cmd_cycle)
    s = sub.add_parser("selftest", help="exercise the client against an in-process bootloader model")
    s.set_defaults(fn=cmd_selftest)
    return p


def main(argv=None):
    args = build_parser().parse_args(argv)
    try:
        return args.fn(args)
    except (ProtocolError, TimeoutError) as e:
        print("test-bootloader.py {}: {}".format(args.command, e), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("interrupted", file=sys.stderr)
        return 130


if __name__ == "__main__":
    sys.exit(main())
