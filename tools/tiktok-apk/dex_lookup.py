"""Find which APK entry holds an id constant or defines a class (read-only).

A full jadx run over TikTok silently drops classes, so decompile only the dex that
matters: first find which dex embeds the resource id (``const``), then which dex
defines the class that uses it and its superclass (``classdef``).

    py -3.12 -I dex_lookup.py const base.apk 0x7f0a1505 0x7f0a6318
    py -3.12 -I dex_lookup.py classdef base.apk "LX/1Avu;"

Decompiled output stays under the git-ignored ``target/`` directory.
"""

from __future__ import annotations

import argparse
import struct
import sys
import zipfile
from pathlib import Path

NO_INDEX = 0xFFFFFFFF


def searched(name: str) -> bool:
    """Entries that can embed a resource id: dex files, assets, arm64 dex libs, arsc."""
    return (name.endswith(".dex") or name.startswith("assets/") or name == "resources.arsc"
            or ("libdex_" in name and "arm64" in name))


def find_constants(apk: zipfile.ZipFile, ids: list[int]):
    """Yield (id, entry, count) for little-endian u32 occurrences of each id."""
    patterns = {struct.pack("<I", res_id): res_id for res_id in ids}
    for info in apk.infolist():
        if not searched(info.filename):
            continue
        data = apk.read(info)
        for pattern, res_id in patterns.items():
            if count := data.count(pattern):
                yield res_id, info.filename, count


def uleb128(data: bytes, offset: int) -> tuple[int, int]:
    value = shift = 0
    while True:
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if byte < 0x80:
            return value, offset


def class_defs(dex: bytes):
    """Yield (class descriptor, superclass descriptor or None) for one dex file."""
    if not dex.startswith(b"dex\n"):
        raise ValueError("not a dex file")
    _string_count, string_ids, _type_count, type_ids = struct.unpack_from("<IIII", dex, 0x38)
    class_count, class_offset = struct.unpack_from("<II", dex, 0x60)

    def string(index: int) -> str:
        offset = struct.unpack_from("<I", dex, string_ids + 4 * index)[0]
        _, start = uleb128(dex, offset)
        return dex[start:dex.index(b"\0", start)].decode("utf-8", "replace")

    def type_name(index: int) -> str:
        return string(struct.unpack_from("<I", dex, type_ids + 4 * index)[0])

    for number in range(class_count):
        cls, _flags, superclass = struct.unpack_from("<III", dex, class_offset + 32 * number)
        yield type_name(cls), None if superclass == NO_INDEX else type_name(superclass)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Locate id constants and class definitions in an APK.")
    commands = parser.add_subparsers(dest="command", required=True)
    const = commands.add_parser("const", help="entries embedding resource ids")
    const.add_argument("apk", type=Path)
    const.add_argument("ids", nargs="+", help="hex resource ids, e.g. 0x7f0a1505")
    classdef = commands.add_parser("classdef", help="dex defining classes, with superclass")
    classdef.add_argument("apk", type=Path)
    classdef.add_argument("classes", nargs="+", help='descriptors, e.g. "LX/1Avu;"')
    args = parser.parse_args(argv)

    try:
        with zipfile.ZipFile(args.apk) as apk:
            if args.command == "const":
                for res_id, entry, count in find_constants(apk, [int(value, 16) for value in args.ids]):
                    print(f"0x{res_id:08x}\t{entry}\t{count}")
            else:
                wanted = set(args.classes)
                for name in sorted(n for n in apk.namelist() if n.endswith(".dex")):
                    for cls, superclass in class_defs(apk.read(name)):
                        if cls in wanted:
                            print(f"{name}\t{cls}\textends\t{superclass or '-'}")
    except (OSError, ValueError, struct.error, IndexError, zipfile.BadZipFile) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
