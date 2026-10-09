"""NxString decoding and @<decimal> token resolution on synthetic tables."""

import json
import struct
import tempfile
import unittest
import zipfile
from collections import defaultdict
from pathlib import Path

import find_string
import nxstring
import resolve_desc


def xor(text: str) -> bytes:
    return bytes(byte ^ 0x0F for byte in text.encode("utf-8")) + b"\x0f"


def table(locale: str, types: list[tuple[int, list]], package: int = 0x7F) -> bytes:
    """Encode an NxString table; an entry is None, (marker, text) or (2, [(q, text)])."""
    count = sum(len(entries) for _, entries in types)
    values = bytearray()
    offsets = []
    value_base = 0x40 + 3 * count
    for _, entries in types:
        for entry in entries:
            if entry is None:
                offsets.append(0xFFFFFF)
                continue
            offsets.append(value_base + len(values))
            marker, body = entry
            values.append(marker)
            if marker == 2:
                for quantity, text in body:
                    values += bytes([quantity, 0x01]) + xor(text)
            else:
                values += xor(body)
    header = bytearray(b"NxString" + struct.pack("<III", package, 1, 0) + locale.encode().ljust(12, b"\0"))
    for type_id, entries in types:
        header += struct.pack("<II", type_id, len(entries))
    header = header.ljust(0x40, b"\0")
    for offset in offsets:
        header += offset.to_bytes(3, "little")
    data = bytearray(header + values)
    struct.pack_into("<I", data, 16, len(data))
    return bytes(data)


STRINGS = 0x11
PLURALS = 0x10


class DecodeTests(unittest.TestCase):
    def test_strings_absent_slots_and_plurals_decode_to_resource_ids(self):
        data = table("vi-VN", [
            (STRINGS, [(1, "Đăng"), None, (3, "Q&A\nline"), (1, "")]),
            (PLURALS, [(2, [(4, "%d styles"), (6, "%d style")])]),
        ])
        decoded = nxstring.decode(data)
        self.assertEqual(decoded.locale, "vi-VN")
        self.assertEqual(decoded.types, ((STRINGS, 4), (PLURALS, 1)))
        self.assertEqual(
            [(e.res_id, e.marker, e.text) for e in decoded.entries],
            [(0x7F100000, 2, "q4=%d styles | q6=%d style"),
             (0x7F110000, 1, "Đăng"),
             (0x7F110002, 3, "Q&A\nline"),
             (0x7F110003, 1, "")],
        )
        self.assertEqual(decoded.unknown_markers, 0)

    def test_unknown_marker_is_counted_not_dropped(self):
        decoded = nxstring.decode(table("default", [(STRINGS, [(9, "x")])]))
        self.assertEqual((decoded.entries[0].text, decoded.unknown_markers), ("x", 1))

    def test_malformed_tables_are_refused(self):
        good = table("default", [(STRINGS, [(1, "Back")])])
        with self.assertRaisesRegex(ValueError, "magic"):
            nxstring.decode(b"NotAString" + good[10:])
        with self.assertRaisesRegex(ValueError, "size"):
            nxstring.decode(good + b"\0")
        broken = bytearray(good)
        broken[0x40:0x43] = (len(good) + 5).to_bytes(3, "little")
        with self.assertRaisesRegex(ValueError, "offset"):
            nxstring.decode(bytes(broken))
        unterminated = good[:-1] + b"A"
        with self.assertRaisesRegex(ValueError, "unterminated"):
            nxstring.decode(unterminated)

    def test_tsv_round_trips_escaped_text(self):
        decoded = nxstring.decode(table("en", [(STRINGS, [(1, "a\tb\\n\nc\r")])]))
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "t.tsv"
            nxstring.write_tsv(decoded, path, "synthetic")
            self.assertEqual(nxstring.load_tsv(path), {0x7F110000: "a\tb\\n\nc\r"})
            self.assertEqual(len(path.read_text(encoding="utf-8").splitlines()), 2)


def synthetic_apk(path: Path) -> None:
    with zipfile.ZipFile(path, "w") as apk:
        apk.writestr(nxstring.DEFAULT_ENTRY, table("default", [(STRINGS, [(1, "Back"), (1, "Hot"), (1, "Post")])]))
        apk.writestr("assets/strings#lang_vi/nxstring_vi.xrsc", table("vi", [(STRINGS, [(1, "Lùi"), None, (1, "Đăng (vi)")])]))
        apk.writestr("assets/strings#lang_vi/nxstring_vi-VN.xrsc", table("vi-VN", [(STRINGS, [(1, "Quay lại"), None, None])]))


class LocaleTests(unittest.TestCase):
    def test_region_overrides_language_overrides_english_base(self):
        with tempfile.TemporaryDirectory() as temp:
            apk = Path(temp) / "base.apk"
            synthetic_apk(apk)
            self.assertEqual(nxstring.load_apk_strings(apk, "vi-VN"),
                             {0x7F110000: "Quay lại", 0x7F110001: "Hot", 0x7F110002: "Đăng (vi)"})
            self.assertEqual(nxstring.load_apk_strings(apk, "vi-VN", with_default=False),
                             {0x7F110000: "Quay lại", 0x7F110002: "Đăng (vi)"})
            with zipfile.ZipFile(apk) as opened:
                self.assertEqual(sorted(nxstring.apk_locales(opened)), ["default", "vi", "vi-VN"])

    def test_resolve_reports_untranslated_and_unknown_tokens(self):
        with tempfile.TemporaryDirectory() as temp:
            apk = Path(temp) / "base.apk"
            synthetic_apk(apk)
            out = Path(temp) / "out.tsv"
            hot, back = 0x7F110001, 0x7F110000
            code = resolve_desc.main([f"@{hot}", f"@{back}", "--apk", str(apk), "--out", str(out)])
            self.assertEqual(code, 0)
            rows = [line.split("\t") for line in out.read_text(encoding="utf-8").splitlines()]
            self.assertEqual(rows[1:], [[f"@{back}", "0x7f110000", "Back", "Quay lại", "argv"],
                                        [f"@{hot}", "0x7f110001", "Hot", "", "argv"]])
            self.assertEqual(resolve_desc.main(["@2131000000", "--apk", str(apk), "--out", str(out)]), 1)

    def test_find_string_matches_labels_in_tiers_and_lists_layout_refs(self):
        with tempfile.TemporaryDirectory() as temp:
            apk = Path(temp) / "base.apk"
            synthetic_apk(apk)
            index = Path(temp) / "layout_index.jsonl"
            index.write_text(json.dumps({"id": "dop", "dir": "layout", "layout": "top_bar", "tag": "TextView",
                                         "attrs": {"contentDescription": "@0x7f110000"}}) + "\n", encoding="utf-8")
            out = Path(temp) / "out.tsv"
            args = ["--apk", str(apk), "--index", str(index), "--out", str(out)]
            self.assertEqual(find_string.main(["Quay lại", "post", "o", *args]), 0)
            rows = [line.split("\t") for line in out.read_text(encoding="utf-8").splitlines()]
            self.assertEqual(rows[1:], [["Quay lại", "exact", "0x7f110000", "Back", "Quay lại", "layout/top_bar/dop:contentDescription"],
                                        ["post", "ci", "0x7f110002", "Post", "Đăng (vi)", ""],
                                        ["o", "contains", "0x7f110001", "Hot", "", ""],
                                        ["o", "contains", "0x7f110002", "Post", "Đăng (vi)", ""]])
            self.assertEqual(find_string.main(["Missing", *args]), 1)
            self.assertEqual(out.read_text(encoding="utf-8").splitlines()[1], "Missing\tnone\t\t\t\t")


class TokenScanTests(unittest.TestCase):
    def test_scan_collects_desc_and_text_tokens_only(self):
        found = defaultdict(set)
        xml = ('<node resource-id="com.zhiliaoapp.musically:id/bix" content-desc="@2131823215" text=""/>'
               '<node resource-id="" text="@2131823090" content-desc="@notanid"/>'
               '<node hint="@2131800000"/>')
        resolve_desc.scan_text(xml, "dump.xml", found)
        self.assertEqual(dict(found), {2131823215: {"dump.xml:bix:content-desc"},
                                       2131823090: {"dump.xml:-:text"}})
        with self.assertRaises(ValueError):
            resolve_desc.parse_token("2131823090")


if __name__ == "__main__":
    unittest.main()
