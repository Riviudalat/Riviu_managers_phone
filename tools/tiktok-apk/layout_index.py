"""Build a layout/id index for one build (read-only analysis).

Inputs are the unmodified APK and its ``aapt2 dump resources`` text (created with
``--sdk`` when the file does not exist yet). Every compiled layout is parsed straight from the APK (see ``axml.py``); one JSON object is written per
element that carries an id, plus every ``<include>``:

    {"id": "dou", "res": "0x7f0a....", "layout": "...", "dir": "layout", "file": "res/b/x.xml",
     "tag": "...", "depth": 3, "pos": [0, 2, 1], "tags": ["FrameLayout", ...],
     "attrs": {"text": "@0x7f11....", "visibility": "2"},
     "strings": {"text": "Add sound"},
     "parent": ["LinearLayout", "dop"], "siblings": ["doi"], "children": ["TextView"]}

Ids and layouts of the main package (``--package-id``) keep their bare name; those of
dynamic-feature packages shipped in the same APK are written ``df_ship:il0``.
``pos`` is the child-index path from the layout root and ``tags`` the ancestor tags;
``strings`` holds text/hint/contentDescription references resolved with the NxString
tables of ``--locale`` (English base by default) or resources.arsc.

    py -3.12 -I layout_index.py --apk base.apk --sdk %LOCALAPPDATA%\\Android\\Sdk ^
        --resources-dump target\\tiktok-X\\aapt2-resources.txt --out target\\tiktok-X\\layout_index.jsonl
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import axml  # noqa: E402
import nxstring  # noqa: E402
import resources_dump  # noqa: E402
import sdk  # noqa: E402

KEEP = ("text", "contentDescription", "hint", "importantForAccessibility",
        "accessibilityLiveRegion", "visibility", "clickable", "focusable",
        "singleLine", "ellipsize", "marqueeRepeatLimit", "layout")
STRING_ATTRS = ("text", "contentDescription", "hint")
REF = re.compile(r"^@0x([0-9a-f]{8})$")


def layout_dir(config: str) -> str:
    return "layout" if not config else "layout-" + config.replace(" ", "-")


def records(root: axml.Element, layout: str, config: str, path: str,
            table: resources_dump.ResourceTable, strings: dict[int, str], package_id: int):
    """Yield index records for one parsed layout."""

    def ref_name(value: str | None) -> str:
        match = REF.match(value or "")
        return table.name(int(match.group(1), 16), package_id) if match else ""

    def resolve(value: str) -> str | None:
        match = REF.match(value)
        if not match:
            return value
        res_id = int(match.group(1), 16)
        return strings.get(res_id, table.strings.get(res_id))

    def walk(element: axml.Element, parent: axml.Element | None, pos: list[int], tags: list[str]):
        element_id = ref_name(element.attrs.get("id"))
        if element_id or element.tag in ("include", "ViewStub"):
            attrs = {key: element.attrs[key] for key in KEEP if key in element.attrs}
            if "layout" in attrs:
                attrs["layout"] = ref_name(attrs["layout"]) or attrs["layout"]
            resolved = {key: resolve(attrs[key]) for key in STRING_ATTRS if key in attrs}
            siblings = [] if parent is None else [
                ref_name(other.attrs.get("id")) for other in parent.children
                if other is not element and other.attrs.get("id")]
            yield {
                "id": element_id,
                "res": element.attrs.get("id", "")[1:],
                "layout": layout,
                "dir": layout_dir(config),
                "file": path,
                "tag": element.tag,
                "depth": len(pos),
                "pos": pos,
                "tags": tags,
                "attrs": attrs,
                "strings": {key: value for key, value in resolved.items() if value is not None},
                "parent": None if parent is None else [parent.tag, ref_name(parent.attrs.get("id"))],
                "siblings": siblings[:12],
                "children": [ref_name(child.attrs.get("id")) or child.tag for child in element.children][:12],
            }
        for index, child in enumerate(element.children):
            yield from walk(child, element, pos + [index], tags + [element.tag])

    yield from walk(root, None, [], [])


def build(apk_path: Path, table: resources_dump.ResourceTable, strings: dict[int, str],
          package_id: int, out_path: Path) -> tuple[int, int, int]:
    layouts = written = failed = 0
    with zipfile.ZipFile(apk_path) as apk, open(out_path, "w", encoding="utf-8", newline="\n") as out:
        members = set(apk.namelist())
        for res_id in sorted(table.layout_files):
            # Dynamic-feature packages ship in base.apk too and reuse main-package ids.
            name = table.name(res_id, package_id)
            for config, path in table.layout_files[res_id]:
                if path not in members:
                    failed += 1
                    continue
                try:
                    root = axml.parse(apk.read(path))
                except (ValueError, IndexError, UnicodeError) as error:
                    print(f"skip {name} ({path}): {error}", file=sys.stderr)
                    failed += 1
                    continue
                layouts += 1
                for record in records(root, name, config, path, table, strings, package_id):
                    out.write(json.dumps(record, ensure_ascii=False) + "\n")
                    written += 1
    return layouts, written, failed


def load(path: Path) -> list[dict]:
    with open(path, encoding="utf-8") as src:
        return [json.loads(line) for line in src if line.strip()]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Index layout ids of one APK build.")
    parser.add_argument("--apk", type=Path, required=True)
    parser.add_argument("--resources-dump", type=Path, required=True,
                        help="aapt2 dump resources output; created with --sdk if missing")
    parser.add_argument("--sdk", type=Path, help="Android SDK root, needed to create the dump")
    parser.add_argument("--locale", default="default", help="NxString locale used to resolve strings")
    parser.add_argument("--package-id", default="7f", help="main package id in hex (default 7f); names ids and layouts")
    parser.add_argument("--out", type=Path, required=True, help="JSONL output")
    args = parser.parse_args(argv)
    for stream in (sys.stdout, sys.stderr):
        getattr(stream, "reconfigure", lambda **_: None)(encoding="utf-8")

    try:
        if not args.resources_dump.exists():
            if not args.sdk:
                parser.error("--resources-dump does not exist; pass --sdk to create it")
            resources_dump.dump(sdk.build_tool(args.sdk, "aapt2"), args.apk, args.resources_dump)
        table = resources_dump.load(args.resources_dump)
        strings = nxstring.load_apk_strings(args.apk, args.locale)
        layouts, written, failed = build(args.apk, table, strings, int(args.package_id, 16), args.out)
    except (OSError, RuntimeError, ValueError, zipfile.BadZipFile) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"{layouts} layouts, {written} records, {failed} skipped")
    return 0 if layouts else 1


if __name__ == "__main__":
    sys.exit(main())
