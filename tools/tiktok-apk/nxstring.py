"""Decode TikTok NxString (.xrsc) string tables (read-only analysis).

TikTok strips most strings from resources.arsc and ships them as NxString tables:
``assets/nxstring/nxstring_default.xrsc`` (English base) plus per-locale overrides at
``assets/strings#lang_<lang>/nxstring_<locale>.xrsc``. Layouts and the accessibility
tree reference them by resource id, which changes with every build.

Layout observed on 45.7.3 (2026-10-09):

- 0x00 ``NxString`` magic, then u32 package id, u32 format version, u32 file size.
- 0x14 12-byte NUL-padded locale (``default``, ``vi-VN``...).
- 0x20 up to four ``(u32 type id, u32 entry count)`` pairs, zero-terminated.
- 0x40 for each type in order, ``count`` 3-byte little-endian absolute offsets;
  0xFFFFFF marks an entry the table does not override.
- Each value starts with a raw marker byte. Text bytes are XOR 0x0F and end at a byte
  0x0F (a NUL after XOR). Markers 1 and 3 hold one string. Marker 2 is a plural:
  repeated ``(quantity code, 0x01, text)`` items up to the next entry's offset. The
  quantity-code meaning is not verified, so codes are kept as numbers.

CLI::

    py -3.12 -I nxstring.py --apk base.apk --list-locales
    py -3.12 -I nxstring.py --apk base.apk --locale vi-VN --out nxstring_vi-VN.tsv
    py -3.12 -I nxstring.py --xrsc nxstring_default.xrsc --out nxstring_default.tsv

TSV rows are ``0x<res id>\\t<marker>\\t<text>``; tab, newline, CR and backslash are
escaped so the file round-trips through ``load_tsv``. The first line is a ``#`` comment.
"""

from __future__ import annotations

import argparse
import re
import struct
import sys
import zipfile
from dataclasses import dataclass
from pathlib import Path

MAGIC = b"NxString"
HEADER_END = 0x40
ABSENT = 0xFFFFFF
XOR = 0x0F
PLURAL = 2
DEFAULT_ENTRY = "assets/nxstring/nxstring_default.xrsc"
LOCALE_ENTRY = re.compile(r"^assets/strings#lang_([^/]+)/nxstring_([^/]+)\.xrsc$")


@dataclass(frozen=True)
class NxEntry:
    res_id: int
    marker: int
    text: str


@dataclass(frozen=True)
class NxTable:
    package_id: int
    format_version: int
    locale: str
    types: tuple[tuple[int, int], ...]
    entries: tuple[NxEntry, ...]
    unknown_markers: int


def _read_text(data: bytes, start: int, limit: int) -> tuple[str, int]:
    end = data.find(bytes([XOR]), start, limit)
    if end < 0:
        raise ValueError(f"unterminated string at 0x{start:x}")
    raw = bytes(byte ^ XOR for byte in data[start:end])
    return raw.decode("utf-8", "replace"), end + 1


def decode(data: bytes) -> NxTable:
    """Decode one table. Raises ValueError on a layout this decoder does not know."""
    if len(data) < HEADER_END or data[:8] != MAGIC:
        raise ValueError("not an NxString table (bad magic or short header)")
    package_id, format_version, size = struct.unpack_from("<III", data, 8)
    if size != len(data):
        raise ValueError(f"header size {size} != file size {len(data)}")
    locale = data[0x14:0x20].split(b"\0")[0].decode("ascii", "replace")
    types: list[tuple[int, int]] = []
    pos = 0x20
    while pos < HEADER_END:
        type_id, count = struct.unpack_from("<II", data, pos)
        if type_id == 0:
            break
        if type_id > 0xFF:
            raise ValueError(f"type id 0x{type_id:x} does not fit a resource id")
        types.append((type_id, count))
        pos += 8
    table_end = HEADER_END + 3 * sum(count for _, count in types)
    if table_end > len(data):
        raise ValueError("offset tables run past the end of the file")

    slots: list[tuple[int, int]] = []
    pos = HEADER_END
    for type_id, count in types:
        for index in range(count):
            offset = data[pos] | (data[pos + 1] << 8) | (data[pos + 2] << 16)
            pos += 3
            if offset == ABSENT:
                continue
            if not table_end <= offset < len(data):
                raise ValueError(f"offset 0x{offset:x} outside the value area")
            slots.append(((package_id << 24) | (type_id << 16) | index, offset))

    # Values are not guaranteed to be stored in id order; bound each by the next offset.
    starts = sorted({offset for _, offset in slots})
    bound = {start: nxt for start, nxt in zip(starts, starts[1:] + [len(data)])}
    entries = []
    unknown = 0
    for res_id, offset in slots:
        marker = data[offset]
        limit = bound[offset]
        if marker == PLURAL:
            items = []
            cursor = offset + 1
            while cursor + 1 < limit and data[cursor + 1] == 0x01:
                quantity = data[cursor]
                text, cursor = _read_text(data, cursor + 2, limit)
                items.append(f"q{quantity}={text}")
            text = " | ".join(items)
        else:
            if marker not in (1, 3):
                unknown += 1
            text, _ = _read_text(data, offset + 1, limit)
        entries.append(NxEntry(res_id, marker, text))
    entries.sort(key=lambda entry: entry.res_id)
    return NxTable(package_id, format_version, locale, tuple(types), tuple(entries), unknown)


def escape(text: str) -> str:
    return text.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n").replace("\r", "\\r")


def unescape(text: str) -> str:
    return re.sub(r"\\([\\tnr])", lambda m: {"\\": "\\", "t": "\t", "n": "\n", "r": "\r"}[m.group(1)], text)


def write_tsv(table: NxTable, path: Path, source: str) -> None:
    with open(path, "w", encoding="utf-8", newline="\n") as out:
        out.write(f"# source={source} locale={table.locale} pkg=0x{table.package_id:x} "
                  f"format={table.format_version} types={list(table.types)}\n")
        for entry in table.entries:
            out.write(f"0x{entry.res_id:08x}\t{entry.marker}\t{escape(entry.text)}\n")


def load_tsv(path: Path) -> dict[int, str]:
    values: dict[int, str] = {}
    with open(path, encoding="utf-8") as src:
        for line in src:
            if line.startswith("#") or not line.strip():
                continue
            res_id, _, text = line.rstrip("\n").split("\t", 2)
            values[int(res_id, 16)] = unescape(text)
    return values


def apk_locales(apk: zipfile.ZipFile) -> dict[str, str]:
    """Map locale name ('default', 'vi', 'vi-VN', ...) to its zip entry."""
    found = {}
    for name in apk.namelist():
        if name == DEFAULT_ENTRY:
            found["default"] = name
        elif match := LOCALE_ENTRY.match(name):
            found[match.group(2)] = name
    return found


def locale_chain(locale: str) -> list[str]:
    """Lookup order, weakest first: default, language, language-region.

    The precedence (region over language over default) mirrors Android's resource
    fallback; it is assumed, not read from TikTok's loader.
    """
    chain = ["default"]
    language = locale.split("-")[0]
    if language and language != "default":
        chain.append(language)
    if locale not in chain:
        chain.append(locale)
    return chain


def merged_strings(tables: list[NxTable]) -> dict[int, str]:
    merged: dict[int, str] = {}
    for table in tables:
        merged.update((entry.res_id, entry.text) for entry in table.entries)
    return merged


def load_apk_strings(apk_path: Path, locale: str, with_default: bool = True) -> dict[int, str]:
    """Strings as a phone set to ``locale`` would see them (``default`` = English base).

    ``with_default=False`` keeps only the locale's own overrides, i.e. what is translated.
    """
    with zipfile.ZipFile(apk_path) as apk:
        entries = apk_locales(apk)
        if "default" not in entries:
            raise ValueError(f"{apk_path} has no {DEFAULT_ENTRY}")
        chain = [name for name in locale_chain(locale) if with_default or name != "default"]
        tables = [decode(apk.read(entries[name])) for name in chain if name in entries]
    return merged_strings(tables)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Decode TikTok NxString (.xrsc) tables to TSV.")
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--apk", type=Path, help="APK containing the tables")
    source.add_argument("--xrsc", type=Path, help="a single extracted .xrsc table")
    parser.add_argument("--locale", default="default",
                        help="table to decode from --apk (default, vi, vi-VN, ...); "
                             "decodes that one table only, without fallback")
    parser.add_argument("--list-locales", action="store_true", help="list tables in --apk")
    parser.add_argument("--out", type=Path, help="TSV output path")
    args = parser.parse_args(argv)

    try:
        if args.apk:
            with zipfile.ZipFile(args.apk) as apk:
                entries = apk_locales(apk)
                if args.list_locales:
                    for name in sorted(entries):
                        print(f"{name}\t{entries[name]}")
                    return 0
                if args.locale not in entries:
                    parser.error(f"no table for locale {args.locale!r}; try --list-locales")
                data = apk.read(entries[args.locale])
                source_name = f"{args.apk.name}!{entries[args.locale]}"
        else:
            data = args.xrsc.read_bytes()
            source_name = args.xrsc.name
        table = decode(data)
    except (OSError, ValueError, zipfile.BadZipFile) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    if not args.out:
        parser.error("--out is required unless --list-locales is given")
    write_tsv(table, args.out, source_name)
    print(f"{table.locale}: {len(table.entries)} entries, types={list(table.types)}, "
          f"unknown markers={table.unknown_markers}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
