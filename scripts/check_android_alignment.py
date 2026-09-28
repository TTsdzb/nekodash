#!/usr/bin/env python3
"""Check every arm64 ELF library in an APK for 16 KB LOAD and RELRO alignment."""

import argparse
import struct
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk")
    args = parser.parse_args()
    with zipfile.ZipFile(args.apk) as apk:
        libraries = [name for name in apk.namelist() if name.endswith(".so")]
        if "lib/arm64-v8a/libnekodash.so" not in libraries:
            raise SystemExit("Missing arm64 NekoDash library in APK")
        for name in libraries:
            data = apk.read(name)
            if data[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", data, 18)[0] != 183:
                raise SystemExit(f"{name}: expected a little-endian arm64 ELF library")
            offset = struct.unpack_from("<Q", data, 32)[0]
            size, count = struct.unpack_from("<HH", data, 54)
            loads = 0
            for index in range(count):
                kind, _, _, vaddr, _, _, memsz, alignment = struct.unpack_from(
                    "<IIQQQQQQ", data, offset + index * size,
                )
                if kind == 1:  # PT_LOAD
                    loads += 1
                    if alignment < 16384:
                        raise SystemExit(f"{name}: LOAD alignment {alignment} is below 16 KB")
                if kind == 0x6474E552 and (vaddr + memsz) % 16384:  # PT_GNU_RELRO
                    raise SystemExit(f"{name}: RELRO end is not 16 KB aligned")
            if loads == 0:
                raise SystemExit(f"{name}: missing LOAD segments")
            print(f"Verified 16 KB ELF alignment: {name}")


if __name__ == "__main__":
    main()
