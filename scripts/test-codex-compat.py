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


def non_exclusive():
    """hidapi seizes the device on macOS by default, which knocks the Codex
    app / Input off it; the flag lives in the compiled module, so poke it."""
    if sys.platform != "darwin":
        return
    try:
        import ctypes
        ctypes.CDLL(hid.__file__).hid_darwin_set_open_exclusive(0)
    except (AttributeError, OSError):
        pass


def open_codex():
    for info in hid.enumerate(VID, PID):
        if info["usage_page"] == CODEX_USAGE_PAGE:
            non_exclusive()
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

    # Work Louder Input's file system: list, read the keymap in chunks,
    # and round-trip a small file through the streamed write path.
    r = rpc(dev, asm, "fs.list", {"checksum": True}, 4250)
    files = {f["name"]: f for f in r.get("result", [])}
    check("fs.list has keymap.json", "keymap.json" in files, json.dumps(r)[:200])
    import base64, hashlib
    buf = bytearray()
    off = 0
    while "keymap.json" in files:
        r = rpc(dev, asm, "fs.readbin", {"file": "keymap.json", "offset": off, "len": 3072}, 4251)
        res = r.get("result", {})
        chunk = base64.b64decode(res.get("data", ""))
        if not chunk:
            break
        buf += chunk
        off += len(chunk)
        if off >= res.get("total_size", 0):
            break
    check("keymap.json reassembles and parses", bool(buf) and json.loads(buf.decode()).get("profiles") is not None, f"{len(buf)} bytes")
    check("keymap.json checksum matches fs.list", hashlib.sha1(buf).hexdigest() == files.get("keymap.json", {}).get("checksum"))
    sa = json.dumps({"version": 1, "smartActions": {"SA_0": {"name": "t", "type": "URL_STEP", "payload": {"url": "https://openmicrokbd.org"}}}, "smartActionGroups": []}).encode()
    b64 = base64.b64encode(sa).decode()
    r = rpc(dev, asm, "fs.writebin", {"file": "smart_actions.json", "data": b64, "append": True, "completed": True, "offset": 0}, 4252, timeout_s=4.0)
    check("fs.writebin stores smart_actions.json", r.get("result", {}).get("data_written") == len(sa), json.dumps(r))
    r = rpc(dev, asm, "fs.read", {"file": "smart_actions.json"}, 4253)
    check("fs.read returns the written JSON", r.get("result") == json.loads(sa), json.dumps(r)[:200])
    r = rpc(dev, asm, "fs.delete", {"file": "smart_actions.json"}, 4254, timeout_s=4.0)
    check("fs.delete", r.get("result") == {"ok": True})

    # A keymap large enough to need two writebin chunks (Input sends 4096
    # base64 chars = 3072 bytes per call), with profile 1 active: the pad
    # must re-read it and report profile_index 1, and fall back to the
    # built-in keymap (profile 0) once it is deleted.
    layer = {"id": 0, "name": "L", "color": 0, "os": 0, "layout": {
        "keymap": [["KC_A", "KC_B"], ["KC_C", "KC_D", "KC_E", "KC_F"], ["KC_G", "KC_H", "KC_I", "KC_J"], ["KC_K", "KC_L", "KC_M"]],
        "encoders": [["KC_VOLD", "KC_VOLU", "KC_MUTE"]], "buttons": [["KC_MPLY"]],
        "joystick": {"type": "VENDOR", "sectors": []}}}
    doc = {"version": 1, "activeProfileId": 1,
           "profiles": [{"id": 0, "name": "zero", "layers": [layer], "macrosUsed": [], "multiActionsUsed": []},
                        {"id": 1, "name": "one " + "x" * 3600, "layers": [layer], "macrosUsed": [], "multiActionsUsed": []}],
           "macros": [], "multiActions": []}
    raw = json.dumps(doc, separators=(",", ":")).encode()
    b64 = base64.b64encode(raw).decode()
    chunks = [b64[i:i + 4096] for i in range(0, len(b64), 4096)]
    check("keymap test needs two chunks", len(chunks) == 2, str(len(chunks)))
    written = 0
    for i, c in enumerate(chunks):
        r = rpc(dev, asm, "fs.writebin", {"file": "keymap.json", "data": c, "append": True, "completed": i == len(chunks) - 1, "offset": written}, 4260 + i, timeout_s=4.0)
        written += r.get("result", {}).get("data_written", 0) if isinstance(r.get("result"), dict) else 0
    check("fs.writebin keymap.json in two chunks", written == len(raw), f"{written}/{len(raw)}")
    r = rpc(dev, asm, "fs.list", {"checksum": True}, 4262)
    got = {f["name"]: f for f in r.get("result", [])}.get("keymap.json", {})
    check("fs.list reports the new keymap", got.get("size") == str(len(raw)) and got.get("checksum") == hashlib.sha1(raw).hexdigest(), json.dumps(got))
    r = rpc(dev, asm, "device.status", {}, 4263)
    check("pad switched to profile 1 from the written keymap", r.get("result", {}).get("profile_index") == 1, json.dumps(r))
    # Every alignment of the final flash program (the driver programs whole
    # words): file sizes covering each residue mod 4 of the last piece.
    sizes_ok = []
    for extra in range(4):
        doc["profiles"][1]["name"] = "one " + "x" * (900 + extra)
        raw = json.dumps(doc, separators=(",", ":")).encode()
        r = rpc(dev, asm, "fs.writebin", {"file": "keymap.json", "data": base64.b64encode(raw).decode(), "append": True, "completed": True, "offset": 0}, 4270 + extra, timeout_s=4.0)
        sizes_ok.append(r.get("result", {}).get("data_written") == len(raw) if isinstance(r.get("result"), dict) else False)
    check("single-chunk writes of every size mod 4", all(sizes_ok), str(sizes_ok))
    r = rpc(dev, asm, "fs.delete", {"file": "keymap.json"}, 4264, timeout_s=4.0)
    check("fs.delete keymap.json", r.get("result") == {"ok": True}, json.dumps(r))
    r = rpc(dev, asm, "device.status", {}, 4265)
    check("pad back on the built-in keymap (profile 0)", r.get("result", {}).get("profile_index") == 0, json.dumps(r))

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
          "press something during the listen window" if not seen else ", ".join(sorted(str(m) for m in seen)))

    dev.close()
    print()
    if failures:
        print(f"FAILED: {', '.join(failures)}")
        sys.exit(1)
    print("all checks passed")


if __name__ == "__main__":
    main()
