#!/usr/bin/env python3
"""Hardware test for Codex Micro compat mode (fw >= 0.8.0).

Talks to the pad the way ChatGPT Desktop does — over the vendor-page 0xFF00
HID interface it exposes when booted in compat mode (identity 303A:8360,
report ID 6, 64-byte reports carrying newline-terminated JSON in 61-byte
fragments) — and checks the request/reply protocol plus live input events.
Boot the pad into compat mode first (hold KEY 04, the second key of the
second row, while plugging in; or toggle it in the app's Settings), then:

    python3 -m pip install hidapi   # once
    python3 scripts/test-codex-compat.py

The OpenMicro app may stay running: it holds the 0xFF60 interface, this
script only opens the 0xFF00 one. QUIT THE CODEX / CHATGPT DESKTOP APP
FIRST: it grabs the 0xFF00 interface exclusively as soon as the pad
appears, so either it or this script gets the device, never both. After the RPC checks it listens for
~10 s and prints every key/dial/stick event the pad sends, so press things.
"""

import json
import sys
import time

try:
    import hid
except ImportError:
    sys.exit("python hidapi missing — run: python3 -m pip install hidapi")

VID, PID = 0x303A, 0x8360
CODEX_USAGE_PAGE = 0xFF00
REPORT_ID = 6
MSG_TYPE = 2
PAYLOAD_MAX = 61

failures = []


def check(name, ok, detail=""):
    print(f"  {'PASS' if ok else 'FAIL'}  {name}" + (f" — {detail}" if detail else ""))
    if not ok:
        failures.append(name)


def open_codex():
    for info in hid.enumerate(VID, PID):
        if info["usage_page"] == CODEX_USAGE_PAGE:
            dev = hid.device()
            dev.open_path(info["path"])
            return dev, info
    sys.exit(
        "no Codex Micro (303A:8360, usage page 0xFF00) found — is the pad "
        "plugged in and booted in compat mode?"
    )


class Assembler:
    """Reassembles input reports into newline-terminated JSON lines, the
    way the reference host probe does: optional leading report ID, type 2,
    length <= 61, then split the accumulated payload on '\\n'."""

    def __init__(self):
        self.buf = bytearray()

    def feed(self, rep):
        rep = bytes(rep)
        if rep and rep[0] == REPORT_ID:
            rep = rep[1:]
        if len(rep) < 2 or rep[0] != MSG_TYPE or rep[1] > PAYLOAD_MAX:
            return []
        n = rep[1]
        self.buf += rep[2 : 2 + n]
        lines = []
        while b"\n" in self.buf:
            line, _, self.buf = self.buf.partition(b"\n")
            if line:
                lines.append(line.decode("utf-8", "replace"))
        return lines


def send(dev, obj):
    payload = (json.dumps(obj, separators=(",", ":")) + "\n").encode()
    for off in range(0, len(payload), PAYLOAD_MAX):
        chunk = payload[off : off + PAYLOAD_MAX]
        rep = bytes([REPORT_ID, MSG_TYPE, len(chunk)]) + chunk
        dev.write(rep + bytes(64 - len(rep)))


def rpc(dev, asm, method, params, req_id, timeout_s=2.0):
    """One request; returns the reply object with matching id (events that
    arrive meanwhile are printed and skipped)."""
    send(dev, {"method": method, "params": params, "id": req_id})
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        rep = dev.read(64, timeout_ms=100)
        if not rep:
            continue
        for line in asm.feed(rep):
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                print(f"  (unparseable line from pad: {line!r})")
                continue
            if msg.get("id") == req_id:
                return msg
            if "method" in msg:
                print(f"  event while waiting: {line}")
    raise TimeoutError(f"no reply to {method} (id {req_id})")


def main():
    dev, info = open_codex()
    print(
        f"found {info['manufacturer_string']!r} / {info['product_string']!r} "
        f"serial {info['serial_number']} release 0x{info['release_number']:04x}"
    )
    check("manufacturer string", info["manufacturer_string"] == "Work Louder")
    check("product string", info["product_string"] == "Codex Micro")
    check("usage 1 on page 0xFF00", info["usage"] == 1, f"usage={info['usage']}")

    asm = Assembler()

    print("RPC:")
    r = rpc(dev, asm, "sys.version", {}, 4241)
    version = r.get("result", {}).get("version", "")
    check("sys.version replies", bool(version), f"version={version!r}")
    check("version is the openmicro build", version.endswith("-openmicro"), version)

    r = rpc(dev, asm, "device.status", {}, 4242)
    res = r.get("result", {})
    check(
        "device.status shape",
        set(res) == {"version", "profile_index", "layer_index", "battery", "is_charging"},
        json.dumps(res),
    )
    check("device.status battery 0..100", 0 <= res.get("battery", -1) <= 100)

    # The six agent lights, exactly as the reference host probe sends them
    # (white idle, blue breathing, green, orange, red, slot 5 unused).
    lights = [
        {"id": 0, "c": 16777215, "b": 1, "e": "off", "s": 0},
        {"id": 1, "c": 1754367, "b": 1, "e": "breath", "s": 1},
        {"id": 2, "c": 4521796, "b": 1, "e": "off", "s": 0},
        {"id": 3, "c": 16753920, "b": 1, "e": "off", "s": 0},
        {"id": 4, "c": 16724787, "b": 1, "e": "off", "s": 0},
        {"id": 5, "c": 0, "b": 0, "e": "off", "s": 0},
    ]
    r = rpc(dev, asm, "v.oai.thstatus", lights, 4243)
    check("v.oai.thstatus acked", r.get("result") == {"ok": True}, json.dumps(r))
    print("  -> keys 1-5 should now show white, breathing blue, green, orange, red; key 6 dark")

    r = rpc(
        dev,
        asm,
        "v.oai.rgbcfg",
        {
            "ambient": {"c": 0x2020FF, "b": 0.6, "e": "off", "s": 0},
            "keys": {"c": 0xFFFFFF, "b": 0.3, "e": "off", "s": 0},
        },
        4244,
    )
    check("v.oai.rgbcfg acked", r.get("result") == {"ok": True}, json.dumps(r))
    print("  -> underglow blue, command keys (rows 3-4) dim white")

    r = rpc(dev, asm, "host.focused_app", {"app": "Terminal"}, 4245)
    check("host.focused_app acked", r.get("result") == {"ok": True})

    r = rpc(dev, asm, "no.such.method", {}, 4246)
    check(
        "unknown method -> -32601",
        r.get("error", {}).get("code") == -32601,
        json.dumps(r),
    )

    # A request fragmented across reports with the last fragment delayed.
    payload = (json.dumps({"method": "sys.version", "params": {}, "id": 4247}) + "\n").encode()
    payload = payload.replace(b'"params":{}', b'"params":{"pad":"' + b"x" * 80 + b'"}')
    chunks = [payload[i : i + PAYLOAD_MAX] for i in range(0, len(payload), PAYLOAD_MAX)]
    for i, chunk in enumerate(chunks):
        rep = bytes([REPORT_ID, MSG_TYPE, len(chunk)]) + chunk
        dev.write(rep + bytes(64 - len(rep)))
        if i == 0:
            time.sleep(0.2)
    deadline = time.time() + 2.0
    got = None
    while time.time() < deadline and got is None:
        rep = dev.read(64, timeout_ms=100)
        for line in asm.feed(rep) if rep else []:
            msg = json.loads(line)
            if msg.get("id") == 4247:
                got = msg
    check("fragmented request reassembled", got is not None and "result" in got)

    print("listening for input events for 10 s — press keys, turn the dial, move the stick:")
    seen = set()
    deadline = time.time() + 10.0
    while time.time() < deadline:
        rep = dev.read(64, timeout_ms=100)
        if not rep:
            continue
        for line in asm.feed(rep):
            print(f"  {line}")
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            seen.add(msg.get("method"))
    check("saw at least one v.oai.hid / v.oai.rad event", bool(seen & {"v.oai.hid", "v.oai.rad"}),
          "press something during the listen window" if not seen else ", ".join(sorted(seen)))

    dev.close()
    print()
    if failures:
        print(f"FAILED: {', '.join(failures)}")
        sys.exit(1)
    print("all checks passed")


if __name__ == "__main__":
    main()
