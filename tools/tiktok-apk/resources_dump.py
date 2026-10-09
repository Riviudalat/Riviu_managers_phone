"""Parse ``aapt2 dump resources <apk>`` text into a resource-name table.

The dump lists every package, type and entry::

    Package name=com.zhiliaoapp.musically id=7f
      type id id=0a entryCount=38989
        resource 0x7f0a0000 id/rc
          () (id)
      type layout id=0d entryCount=11288
        resource 0x7f0d0038 layout/_tux_toast_layout
          () (file) res/b/ff.xml type=XML
          (land) (file) res/b/fg.xml type=XML
      type string id=11 entryCount=966
        resource 0x7f110000 string/app_name
          () "TikTok"

Only names, layout files and default-config string values are kept.
"""

from __future__ import annotations

import re
import subprocess
from dataclasses import dataclass, field
from pathlib import Path

PACKAGE = re.compile(r"^Package name=(\S+) id=([0-9a-fA-F]+)")
RESOURCE = re.compile(r"^\s+resource (0x[0-9a-fA-F]{8}) ([\w.]+)/(\S+)")
FILE_VALUE = re.compile(r"^\s+\(([^)]*)\) \(file\) (\S+)")
STRING_VALUE = re.compile(r'^\s+\(\) "(.*)"\s*$')


@dataclass
class ResourceTable:
    # res id -> (package name, type, entry name)
    names: dict[int, tuple[str, str, str]] = field(default_factory=dict)
    # (package id, type, entry name) -> res id
    ids: dict[tuple[int, str, str], int] = field(default_factory=dict)
    # layout res id -> [(config qualifier, path inside the APK)]
    layout_files: dict[int, list[tuple[str, str]]] = field(default_factory=dict)
    # string res id -> default-config value from resources.arsc
    strings: dict[int, str] = field(default_factory=dict)

    def name(self, res_id: int, main_package_id: int) -> str:
        """Entry name for the main package; ``<last package segment>:<name>`` for others
        (``df_ship:il0`` for a dynamic feature); hex when unknown (framework ids)."""
        entry = self.names.get(res_id)
        if entry is None:
            return f"0x{res_id:08x}"
        package, _, name = entry
        return name if res_id >> 24 == main_package_id else f"{package.rsplit('.', 1)[-1]}:{name}"

    def has_id(self, package_id: int, name: str) -> bool:
        return (package_id, "id", name) in self.ids


def parse(lines) -> ResourceTable:
    table = ResourceTable()
    package = ""
    current: tuple[int, str] | None = None
    for line in lines:
        line = line.lstrip("﻿").rstrip("\r\n")
        if match := PACKAGE.match(line):
            package = match.group(1)
            current = None
            continue
        if match := RESOURCE.match(line):
            res_id = int(match.group(1), 16)
            res_type, name = match.group(2), match.group(3)
            table.names[res_id] = (package, res_type, name)
            table.ids[(res_id >> 24, res_type, name)] = res_id
            current = (res_id, res_type)
            continue
        if current is None:
            continue
        res_id, res_type = current
        if res_type == "layout" and (match := FILE_VALUE.match(line)):
            table.layout_files.setdefault(res_id, []).append((match.group(1), match.group(2)))
        elif res_type == "string" and (match := STRING_VALUE.match(line)):
            table.strings.setdefault(res_id, match.group(1).replace('\\"', '"'))
    return table


def load(path: Path) -> ResourceTable:
    with open(path, encoding="utf-8", errors="replace") as src:
        return parse(src)


def dump(aapt2: Path, apk: Path, out: Path) -> None:
    """Run ``aapt2 dump resources`` once and save its text output."""
    result = subprocess.run([str(aapt2), "dump", "resources", str(apk)], capture_output=True, check=False, timeout=900)
    if result.returncode != 0:
        raise RuntimeError(f"aapt2 dump resources failed ({result.returncode}): {result.stderr[:400]!r}")
    out.write_bytes(result.stdout)
