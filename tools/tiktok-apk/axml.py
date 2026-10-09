"""Minimal parser for compiled Android binary XML (layouts inside an APK).

TikTok strips attribute names from compiled layouts, so ``aapt2 dump xmltree`` prints
only framework attribute ids such as ``(0x010100d0)=@0x7f0a814d``. This parser reads
the same structure straight from the APK (one aapt2 process per layout would take over
an hour for ~11k layouts) and names the framework attributes Riviu cares about from
``FRAMEWORK_ATTRS``. Everything else keeps its ``0x...`` attribute id.

Only what the layout index needs is decoded: elements, their attributes and nesting.
Malformed input raises ValueError.
"""

from __future__ import annotations

import struct
from dataclasses import dataclass, field

RES_STRING_POOL = 0x0001
RES_XML = 0x0003
RES_XML_START_ELEMENT = 0x0102
RES_XML_END_ELEMENT = 0x0103
RES_XML_RESOURCE_MAP = 0x0180
UTF8_FLAG = 1 << 8
NO_INDEX = 0xFFFFFFFF

# Stable android.R.attr ids (public framework API).
FRAMEWORK_ATTRS = {
    0x010100AB: "ellipsize",
    0x010100D0: "id",
    0x010100DA: "focusable",
    0x010100DC: "visibility",
    0x010100E5: "clickable",
    0x0101014F: "text",
    0x01010150: "hint",
    0x0101015F: "singleLine",
    0x0101021D: "marqueeRepeatLimit",
    0x01010273: "contentDescription",
    0x010103AA: "importantForAccessibility",
    0x010103EE: "accessibilityLiveRegion",
}

TYPE_REFERENCE = 0x01
TYPE_ATTRIBUTE = 0x02
TYPE_STRING = 0x03
TYPE_FLOAT = 0x04
TYPE_INT_HEX = 0x11
TYPE_INT_BOOLEAN = 0x12


@dataclass
class Element:
    tag: str
    attrs: dict[str, str]
    children: list["Element"] = field(default_factory=list)


def _u16(data: bytes, offset: int) -> int:
    return struct.unpack_from("<H", data, offset)[0]


def _u32(data: bytes, offset: int) -> int:
    return struct.unpack_from("<I", data, offset)[0]


def _string_pool(data: bytes, start: int, header_size: int, size: int) -> list[str]:
    count, _styles, flags, strings_start = struct.unpack_from("<IIII", data, start + 8)
    utf8 = bool(flags & UTF8_FLAG)
    base = start + strings_start
    end = start + size
    out = []
    for index in range(count):
        pos = base + _u32(data, start + header_size + 4 * index)
        if not base <= pos < end:
            raise ValueError("string offset outside the pool")
        if utf8:
            # UTF-16 length then UTF-8 byte length, each 1 or 2 bytes (high bit = long).
            pos += 2 if data[pos] & 0x80 else 1
            length = data[pos]
            if length & 0x80:
                length = ((length & 0x7F) << 8) | data[pos + 1]
                pos += 2
            else:
                pos += 1
            out.append(data[pos:pos + length].decode("utf-8", "replace"))
        else:
            length = _u16(data, pos)
            pos += 2
            if length & 0x8000:
                length = ((length & 0x7FFF) << 16) | _u16(data, pos)
                pos += 2
            out.append(data[pos:pos + 2 * length].decode("utf-16-le", "replace"))
    return out


def format_value(data_type: int, value: int, raw: str | None) -> str:
    """Render a typed value the way the layout index stores it."""
    if data_type == TYPE_REFERENCE:
        return f"@0x{value:08x}"
    if data_type == TYPE_ATTRIBUTE:
        return f"?0x{value:08x}"
    if data_type == TYPE_STRING:
        return raw or ""
    if data_type == TYPE_INT_BOOLEAN:
        return "true" if value else "false"
    if data_type == TYPE_FLOAT:
        return repr(struct.unpack("<f", struct.pack("<I", value))[0])
    if data_type == TYPE_INT_HEX:
        return f"0x{value:08x}"
    if 0x10 <= data_type <= 0x1F:
        return str(struct.unpack("<i", struct.pack("<I", value))[0])
    return f"0x{value:08x}" if raw is None else raw


def parse(data: bytes) -> Element:
    """Return the root element of a compiled XML document."""
    try:
        return _parse(data)
    except (struct.error, IndexError) as error:
        raise ValueError(f"truncated or malformed document: {error}") from error


def _parse(data: bytes) -> Element:
    if len(data) < 8 or _u16(data, 0) != RES_XML:
        raise ValueError("not a compiled XML document")
    pos = _u16(data, 2)
    strings: list[str] = []
    resource_map: list[int] = []
    stack: list[Element] = []
    root: Element | None = None

    def string(index: int) -> str | None:
        if index == NO_INDEX:
            return None
        if index >= len(strings):
            raise ValueError("string index outside the pool")
        return strings[index]

    while pos + 8 <= len(data):
        chunk_type, header_size, size = struct.unpack_from("<HHI", data, pos)
        if size < 8 or pos + size > len(data):
            raise ValueError(f"bad chunk size at 0x{pos:x}")
        if chunk_type == RES_STRING_POOL:
            strings = _string_pool(data, pos, header_size, size)
        elif chunk_type == RES_XML_RESOURCE_MAP:
            resource_map = list(struct.unpack_from(f"<{(size - header_size) // 4}I", data, pos + header_size))
        elif chunk_type == RES_XML_START_ELEMENT:
            ext = pos + header_size
            _ns, name, attr_start, attr_size, attr_count = struct.unpack_from("<IIHHH", data, ext)
            attrs = {}
            for index in range(attr_count):
                at = ext + attr_start + index * attr_size
                _attr_ns, attr_name, raw_value = struct.unpack_from("<III", data, at)
                data_type, value = data[at + 15], _u32(data, at + 16)
                attr_id = resource_map[attr_name] if attr_name < len(resource_map) else 0
                if attr_id:
                    key = FRAMEWORK_ATTRS.get(attr_id, f"0x{attr_id:08x}")
                else:
                    key = string(attr_name) or f"#{attr_name}"
                attrs[key] = format_value(data_type, value, string(raw_value))
            element = Element(string(name) or "", attrs)
            if stack:
                stack[-1].children.append(element)
            elif root is None:
                root = element
            stack.append(element)
        elif chunk_type == RES_XML_END_ELEMENT:
            if not stack:
                raise ValueError("unbalanced end element")
            stack.pop()
        pos += size
    if root is None:
        raise ValueError("document has no element")
    return root
