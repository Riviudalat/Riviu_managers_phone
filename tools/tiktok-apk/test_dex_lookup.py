"""Id-constant and class-definition lookup on a synthetic APK and dex."""

import struct
import tempfile
import unittest
import zipfile
from pathlib import Path

import dex_lookup


def dex(strings: list[str], classes: list[tuple[str, str | None]]) -> bytes:
    """Minimal dex: header, string ids, type ids (one per string), class defs, string data."""
    string_ids = 0x70
    type_ids = string_ids + 4 * len(strings)
    class_defs = type_ids + 4 * len(strings)
    data_start = class_defs + 32 * len(classes)
    data = bytearray()
    offsets = []
    for text in strings:
        offsets.append(data_start + len(data))
        raw = text.encode()
        assert len(raw) < 0x80
        data += bytes([len(raw)]) + raw + b"\0"
    out = bytearray(b"dex\n035\0".ljust(0x70, b"\0"))
    struct.pack_into("<IIII", out, 0x38, len(strings), string_ids, len(strings), type_ids)
    struct.pack_into("<II", out, 0x60, len(classes), class_defs)
    out += struct.pack(f"<{len(strings)}I", *offsets)
    out += struct.pack(f"<{len(strings)}I", *range(len(strings)))
    for cls, superclass in classes:
        sup = dex_lookup.NO_INDEX if superclass is None else strings.index(superclass)
        out += struct.pack("<III", strings.index(cls), 1, sup) + bytes(20)
    return bytes(out + data)


class DexLookupTests(unittest.TestCase):
    def test_constants_are_found_only_in_searched_entries(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "base.apk"
            needle = struct.pack("<I", 0x7F0A1505)
            with zipfile.ZipFile(path, "w") as apk:
                apk.writestr("classes2.dex", b"x" + needle + needle)
                apk.writestr("lib/arm64-v8a/libdex_a.so", needle)
                apk.writestr("res/b/hjd.xml", needle)
            with zipfile.ZipFile(path) as apk:
                found = sorted(dex_lookup.find_constants(apk, [0x7F0A1505, 0x7F0A0000]))
        self.assertEqual(found, [(0x7F0A1505, "classes2.dex", 2), (0x7F0A1505, "lib/arm64-v8a/libdex_a.so", 1)])

    def test_class_defs_report_superclass(self):
        data = dex(["LX/1Avu;", "Landroid/view/View;", "LX/Root;"],
                   [("LX/1Avu;", "Landroid/view/View;"), ("LX/Root;", None)])
        self.assertEqual(list(dex_lookup.class_defs(data)),
                         [("LX/1Avu;", "Landroid/view/View;"), ("LX/Root;", None)])
        with self.assertRaises(ValueError):
            list(dex_lookup.class_defs(b"PK\x03\x04"))


if __name__ == "__main__":
    unittest.main()
