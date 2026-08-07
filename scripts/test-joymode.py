#!/usr/bin/env python3
"""Hardware test for the joystick-mode vendor-HID commands (fw >= 0.3.0).

Exercises 0x09/0x0A (GET/SET_JOYMODE — mode 0 keys / 1 mouse / 2 grade,
grade needs fw >= 0.6.0), SAVE persistence, and checks the mouse HID
interface enumerates. Run with the pad connected over USB:

    python3 -m pip install hidapi   # once
    python3 scripts/test-joymode.py

The pad is left in keys mode with factory-equivalent joy settings.
"""

import sys
import time

try:
    import hid
except ImportError:
    sys.exit("python hidapi missing — run: python3 -m pip install hidapi")

VID, PID = 0x1209, 0x0001
RAW_USAGE_PAGE = 0xFF60

CMD_VERSION = 0x01
CMD_SAVE = 0x05
CMD_GET_JOYMODE = 0x09
CMD_SET_JOYMODE = 0x0A
CMD_GET_LED = 0x0B
CMD_SET_LED = 0x0C

failures = []


def check(name, ok, detail=""):
    print(f"  {'PASS' if ok else 'FAIL'}  {name}" + (f" — {detail}" if detail else ""))
    if not ok:
        failures.append(name)


def open_raw():
    for info in hid.enumerate(VID, PID):
        if info["usage_page"] == RAW_USAGE_PAGE:
            dev = hid.device()
            dev.open_path(info["path"])
            return dev
    sys.exit("OpenMicro raw-HID interface not found — is the pad plugged in?")


def command(dev, payload, timeout_s=1.0):
    """One command round-trip; skips interleaved 0x80 event reports."""
    dev.write(bytes([0]) + bytes(payload) + bytes(32 - len(payload)))
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        rep = dev.read(32, timeout_ms=100)
        if not rep:
            continue
        if rep[0] >= 0x80:
            continue  # unsolicited input event
        if rep[0] == payload[0]:
            return rep
    raise TimeoutError(f"no reply to 0x{payload[0]:02x}")


def mouse_interface_present():
    # usbd_hid MouseReport: Generic Desktop (0x01) / Mouse (0x02).
    return any(
        i["usage_page"] == 0x01 and i["usage"] == 0x02
        for i in hid.enumerate(VID, PID)
    )


def main():
    dev = open_raw()
    rep = command(dev, [CMD_VERSION])
    version = bytes(rep[2 : 2 + rep[1]]).decode()
    print(f"pad firmware: {version}")

    print("mouse HID interface:")
    check("mouse interface enumerates", mouse_interface_present())

    print("joystick mode protocol:")
    rep = command(dev, [CMD_GET_JOYMODE])
    check("GET_JOYMODE replies", True, f"mode={rep[1]} speed={rep[2]}")

    rep = command(dev, [CMD_SET_JOYMODE, 1, 8])
    check("SET_JOYMODE mouse/8 acks", rep[1] == 1)
    rep = command(dev, [CMD_GET_JOYMODE])
    check("mode readback", rep[1] == 1 and rep[2] == 8, f"mode={rep[1]} speed={rep[2]}")

    rep = command(dev, [CMD_SET_JOYMODE, 2, 4])
    check("SET_JOYMODE grade/4 acks", rep[1] == 1)
    rep = command(dev, [CMD_GET_JOYMODE])
    check(
        "grade mode readback (fw >= 0.6.0)",
        rep[1] == 2 and rep[2] == 4,
        f"mode={rep[1]} speed={rep[2]}",
    )

    rep = command(dev, [CMD_SET_JOYMODE, 1, 99])
    check("speed clamps to 10", command(dev, [CMD_GET_JOYMODE])[2] == 10)
    rep = command(dev, [CMD_SET_JOYMODE, 7, 5])
    check("bad mode degrades to keys", command(dev, [CMD_GET_JOYMODE])[1] == 0)

    print("LED brightness protocol:")
    rep = command(dev, [CMD_GET_LED])
    check("GET_LED replies", True, f"brightness={rep[1]}")
    initial_brightness = rep[1]
    rep = command(dev, [CMD_SET_LED, 64])
    check("SET_LED 64 acks (pad should dim now)", rep[1] == 1)
    check("brightness readback", command(dev, [CMD_GET_LED])[1] == 64)
    rep = command(dev, [CMD_SET_LED, 0])
    check("brightness 0 (lights off)", command(dev, [CMD_GET_LED])[1] == 0)
    time.sleep(0.5)

    print("persistence (SAVE + re-open):")
    # Grade (2) on purpose: exercises the flash blob's mode validation on
    # load, the newest acceptance path.
    command(dev, [CMD_SET_JOYMODE, 2, 3])
    command(dev, [CMD_SET_LED, 100])
    rep = command(dev, [CMD_SAVE, ord("S"), ord("A"), ord("V"), ord("E")], timeout_s=2.0)
    check("SAVE acks", rep[1] == 1)
    dev.close()
    time.sleep(0.5)
    dev = open_raw()
    rep = command(dev, [CMD_GET_JOYMODE])
    check("mode survives re-open", rep[1] == 2 and rep[2] == 3)
    check("brightness survives re-open", command(dev, [CMD_GET_LED])[1] == 100)
    print("  (for a full power-cycle test: unplug/replug, then GET_JOYMODE " "should still be grade/3)")

    # Leave the pad in keys mode / default speed / original brightness, saved.
    command(dev, [CMD_SET_JOYMODE, 0, 5])
    command(dev, [CMD_SET_LED, initial_brightness])
    command(dev, [CMD_SAVE, ord("S"), ord("A"), ord("V"), ord("E")], timeout_s=2.0)
    dev.close()

    print()
    if failures:
        sys.exit(f"{len(failures)} FAILURE(S): {', '.join(failures)}")
    print("all joystick-mode checks passed")


if __name__ == "__main__":
    main()
