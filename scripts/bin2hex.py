#!/usr/bin/env python3
"""Convert the raw firmware image to Intel HEX for factory programming.

Usage: bin2hex.py <input.bin> <output.hex> [base]

The raw .bin carries no load address, so manufacturing programmers
(STM32CubeProgrammer, J-Flash, gang programmers) can misplace it. Intel HEX
embeds the address: the image lands at `base` (default 0x08000000, the
STM32F072 flash origin) and a start-linear-address record carries the entry
point read from the Cortex-M vector table. The output is verified by decoding
it back and comparing against the input before the script succeeds.
"""

import pathlib
import struct
import sys


def record(rectype: int, addr: int, data: bytes) -> str:
    raw = bytes([len(data), (addr >> 8) & 0xFF, addr & 0xFF, rectype]) + data
    return f":{raw.hex().upper()}{-sum(raw) & 0xFF:02X}"


def encode(image: bytes, base: int) -> str:
    lines = []
    upper = None
    offset = 0
    while offset < len(image):
        addr = base + offset
        if addr >> 16 != upper:
            upper = addr >> 16
            lines.append(record(0x04, 0, struct.pack(">H", upper)))
        # Records never straddle a 64 KiB boundary, whatever the base.
        chunk = image[offset : offset + min(16, 0x10000 - (addr & 0xFFFF))]
        lines.append(record(0x00, addr & 0xFFFF, chunk))
        offset += len(chunk)
    entry = struct.unpack_from("<I", image, 4)[0]
    lines.append(record(0x05, 0, struct.pack(">I", entry)))
    lines.append(record(0x01, 0, b""))
    return "\n".join(lines) + "\n"


def decode(text: str) -> dict[int, int]:
    mem: dict[int, int] = {}
    upper = 0
    for line in text.splitlines():
        raw = bytes.fromhex(line[1:])
        assert (-sum(raw[:-1]) & 0xFF) == raw[-1], "checksum mismatch"
        count, addr, rectype = raw[0], (raw[1] << 8) | raw[2], raw[3]
        data = raw[4 : 4 + count]
        if rectype == 0x04:
            upper = struct.unpack(">H", data)[0] << 16
        elif rectype == 0x00:
            for i, byte in enumerate(data):
                mem[upper + addr + i] = byte
    return mem


def main() -> None:
    if len(sys.argv) not in (3, 4):
        raise SystemExit(f"usage: {sys.argv[0]} <input.bin> <output.hex> [base]")
    bin_path = pathlib.Path(sys.argv[1])
    hex_path = pathlib.Path(sys.argv[2])
    base = int(sys.argv[3], 0) if len(sys.argv) == 4 else 0x08000000

    image = bin_path.read_bytes()
    if len(image) < 8:
        raise SystemExit(f"{bin_path}: too small to hold a vector table")
    text = encode(image, base)

    mem = decode(text)
    expected = {base + i: byte for i, byte in enumerate(image)}
    if mem != expected:
        raise SystemExit(f"{hex_path}: round-trip mismatch against {bin_path}")

    hex_path.write_text(text)
    print(
        f"{hex_path}: {len(image)} bytes at "
        f"0x{base:08x}..0x{base + len(image) - 1:08x}"
    )


if __name__ == "__main__":
    main()
