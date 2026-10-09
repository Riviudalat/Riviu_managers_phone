"""Binary XML parsing, aapt2 resource dumps and the layout index on synthetic input."""

import json
import struct
import tempfile
import unittest
import zipfile
from pathlib import Path

import axml
import layout_index
import resources_dump

NO_INDEX = 0xFFFFFFFF
ANDROID_ID, ANDROID_TEXT, ANDROID_DESC, ANDROID_VISIBILITY = 0x010100D0, 0x0101014F, 0x01010273, 0x010100DC


def string_pool(strings: list[str], utf8: bool) -> bytes:
    data = bytearray()
    offsets = []
    for text in strings:
        offsets.append(len(data))
        if utf8:
            raw = text.encode("utf-8")
            assert len(text) < 0x80 and len(raw) < 0x80
            data += bytes([len(text), len(raw)]) + raw + b"\0"
        else:
            raw = text.encode("utf-16-le")
            data += struct.pack("<H", len(text)) + raw + b"\0\0"
    while len(data) % 4:
        data.append(0)
    header_size = 28
    strings_start = header_size + 4 * len(strings)
    size = strings_start + len(data)
    head = struct.pack("<HHIIIIII", 0x0001, header_size, size, len(strings), 0, 0x100 if utf8 else 0, strings_start, 0)
    return head + struct.pack(f"<{len(strings)}I", *offsets) + bytes(data)


def compiled_xml(tree, utf8: bool = True) -> bytes:
    """``tree`` is (tag, [(attr, type, value)], [children]); attr is an android attr id
    (int) or a plain name (str). Typed values: ('ref', id), ('str', text), ('int', n),
    ('bool', b)."""
    attr_ids: list[int] = []

    def collect(node):
        for attr, _, _ in node[1]:
            if isinstance(attr, int) and attr not in attr_ids:
                attr_ids.append(attr)
        for child in node[2]:
            collect(child)

    collect(tree)
    # Names of resource-id attributes come first and line up with the resource map;
    # TikTok strips them, so they are empty strings here.
    names = [""] * len(attr_ids)

    def index(text: str) -> int:
        if text not in names[len(attr_ids):]:
            names.append(text)
        return names.index(text, len(attr_ids))

    def attr_name(attr) -> int:
        return attr_ids.index(attr) if isinstance(attr, int) else index(attr)

    body = bytearray()

    def element(node):
        tag, attrs, children = node
        encoded = bytearray()
        for attr, kind, value in attrs:
            raw = NO_INDEX
            if kind == "ref":
                data_type, data = 0x01, value
            elif kind == "str":
                raw = index(value)
                data_type, data = 0x03, raw
            elif kind == "bool":
                data_type, data = 0x12, 0xFFFFFFFF if value else 0
            else:
                data_type, data = 0x10, value & 0xFFFFFFFF
            encoded += struct.pack("<IIIHBBI", NO_INDEX, attr_name(attr), raw, 8, 0, data_type, data)
        ext = struct.pack("<IIHHHHHH", NO_INDEX, index(tag), 20, 20, len(attrs), 0, 0, 0)
        chunk = struct.pack("<II", 1, NO_INDEX) + ext + encoded
        body.extend(struct.pack("<HHI", 0x0102, 16, 8 + len(chunk)) + chunk)
        for child in children:
            element(child)
        end = struct.pack("<IIII", 1, NO_INDEX, NO_INDEX, index(tag))
        body.extend(struct.pack("<HHI", 0x0103, 16, 8 + len(end)) + end)

    element(tree)
    pool = string_pool(names, utf8)
    resource_map = struct.pack("<HHI", 0x0180, 8, 8 + 4 * len(attr_ids)) + struct.pack(f"<{len(attr_ids)}I", *attr_ids)
    payload = pool + resource_map + bytes(body)
    return struct.pack("<HHI", 0x0003, 8, 8 + len(payload)) + payload


SOUND_BAR = ("FrameLayout", [(ANDROID_ID, "ref", 0x7F0A0001)], [
    ("LinearLayout", [(ANDROID_ID, "ref", 0x7F0A0002), (ANDROID_DESC, "ref", 0x7F110002)], [
        ("X.1Ak0", [(ANDROID_TEXT, "ref", 0x7F110001), (ANDROID_VISIBILITY, "int", 2)], []),
    ]),
    ("include", [("layout", "ref", 0x7F0D0002)], []),
    ("TextView", [(ANDROID_ID, "ref", 0x7F0A0003), (ANDROID_TEXT, "str", "literal"),
                  ("0x7f060001", "bool", True)], []),
])

DUMP = """﻿Binary APK
Package name=com.zhiliaoapp.musically.df_ship id=4d
  type layout id=0a entryCount=1
    resource 0x4d0a0000 layout/il0
      () (file) res/x/il0.xml type=XML
Package name=com.zhiliaoapp.musically id=7f
  type id id=0a entryCount=4
    resource 0x7f0a0001 id/dok
      () (id)
    resource 0x7f0a0002 id/dou
      () (id)
    resource 0x7f0a0003 id/tv_top_text
      () (id)
    resource 0x7f0a0004 id/s0v
      () (id)
  type layout id=0d entryCount=2
    resource 0x7f0d0001 layout/i74
      () (file) res/b/hjd.xml type=XML
      (land) (file) res/b/hje.xml type=XML
    resource 0x7f0d0002 layout/ga
      () (file) res/b/fr.xml type=XML
  type string id=11 entryCount=1
    resource 0x7f110002 string/sound_desc
      () "Sounds \\"tab\\""
      (vi) "Âm thanh"
"""


class AxmlTests(unittest.TestCase):
    def test_utf8_and_utf16_documents_parse_to_the_same_tree(self):
        for utf8 in (True, False):
            root = axml.parse(compiled_xml(SOUND_BAR, utf8))
            self.assertEqual(root.tag, "FrameLayout")
            self.assertEqual(root.attrs, {"id": "@0x7f0a0001"})
            bar, include, label = root.children
            self.assertEqual(bar.attrs, {"id": "@0x7f0a0002", "contentDescription": "@0x7f110002"})
            self.assertEqual(bar.children[0].attrs, {"text": "@0x7f110001", "visibility": "2"})
            self.assertEqual(include.attrs, {"layout": "@0x7f0d0002"})
            self.assertEqual(label.attrs, {"id": "@0x7f0a0003", "text": "literal", "0x7f060001": "true"})

    def test_malformed_documents_raise_value_error(self):
        data = compiled_xml(SOUND_BAR)
        extra_end = struct.pack("<HHIIIII", 0x0103, 16, 24, 1, NO_INDEX, NO_INDEX, 0)
        unbalanced = struct.pack("<HHI", 0x0003, 8, len(data) + 24) + data[8:] + extra_end
        for broken in (b"", bytes(16), data[:40], data[:120], unbalanced):
            with self.assertRaises(ValueError):
                axml.parse(broken)


class ResourcesDumpTests(unittest.TestCase):
    def test_dump_maps_names_layout_files_and_arsc_strings(self):
        table = resources_dump.parse(DUMP.splitlines(keepends=True))
        self.assertEqual(table.name(0x7F0A0002, 0x7F), "dou")
        self.assertEqual(table.name(0x4D0A0000, 0x7F), "df_ship:il0")
        self.assertEqual(table.name(0x01020014, 0x7F), "0x01020014")
        self.assertEqual(table.layout_files[0x7F0D0001], [("", "res/b/hjd.xml"), ("land", "res/b/hje.xml")])
        self.assertEqual(table.strings, {0x7F110002: 'Sounds "tab"'})
        self.assertTrue(table.has_id(0x7F, "s0v"))
        self.assertFalse(table.has_id(0x7F, "il0"))


class LayoutIndexTests(unittest.TestCase):
    def test_index_records_ids_includes_positions_and_resolved_strings(self):
        table = resources_dump.parse(DUMP.splitlines(keepends=True))
        with tempfile.TemporaryDirectory() as temp:
            apk = Path(temp) / "base.apk"
            with zipfile.ZipFile(apk, "w") as out:
                out.writestr("res/b/hjd.xml", compiled_xml(SOUND_BAR))
                out.writestr("res/b/hje.xml", compiled_xml(SOUND_BAR, utf8=False))
                out.writestr("res/b/fr.xml", b"not xml")
                out.writestr("res/x/il0.xml", compiled_xml(("View", [(ANDROID_ID, "ref", 0x7F0A0002)], [])))
            index = Path(temp) / "index.jsonl"
            counts = layout_index.build(apk, table, {0x7F110001: "Add sound"}, 0x7F, index)
            records = layout_index.load(index)
        self.assertEqual(counts, (3, 9, 1))  # fr.xml is unparsable and skipped
        portrait = [r for r in records if r["file"] == "res/b/hjd.xml"]
        self.assertEqual([r["id"] for r in portrait], ["dok", "dou", "", "tv_top_text"])
        dou = portrait[1]
        self.assertEqual((dou["layout"], dou["dir"], dou["depth"], dou["pos"], dou["tags"]),
                         ("i74", "layout", 1, [0], ["FrameLayout"]))
        self.assertEqual(dou["strings"], {"contentDescription": 'Sounds "tab"'})
        self.assertEqual(dou["parent"], ["FrameLayout", "dok"])
        self.assertEqual(dou["siblings"], ["tv_top_text"])
        self.assertEqual(dou["children"], ["X.1Ak0"])
        self.assertEqual(portrait[2]["attrs"], {"layout": "ga"})
        self.assertEqual(portrait[3]["strings"], {"text": "literal"})
        self.assertEqual({r["dir"] for r in records if r["layout"] == "i74"}, {"layout", "layout-land"})
        self.assertIn("df_ship:il0", {r["layout"] for r in records})
        json.dumps(records)  # every record stays JSON-serializable


if __name__ == "__main__":
    unittest.main()
