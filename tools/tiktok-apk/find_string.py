"""Find the string ids behind UI labels of one build, and the layouts using them.

The reverse of ``resolve_desc.py``: start from a label seen on screen (English or the
phone's locale), get its string id(s), the English and ``--locale`` values, and, with
``--index``, the layout elements whose text/contentDescription/hint reference the id.
A label set from code has no layout reference; look the id up with
``dex_lookup.py const`` instead.

    py -3.12 -I find_string.py --apk base.apk --locale vi-VN "Add sound" "Thêm âm thanh"
    py -3.12 -I find_string.py --apk base.apk --index layout_index.jsonl --labels-file labels.txt

Matching per label, first tier that hits wins: ``exact``, then case-insensitive
``ci``, then ``contains`` (the shortest few). Several ids for one label are normal;
the label alone never picks one. Output TSV: label, match, id, English base value,
``--locale`` value (empty = the phone shows English), layout references.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import zipfile
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import nxstring  # noqa: E402

CONTAINS_LIMIT = 6
REFS_LIMIT = 6
STRING_ATTRS = ("text", "contentDescription", "hint")
REF = re.compile(r"^@0x([0-9a-f]{8})$")


def match(label: str, tables: list[dict[int, str]]) -> tuple[str, list[int]]:
    """Ids whose value in any table matches ``label`` in the first tier that hits."""
    folded = label.casefold()
    tiers = (("exact", lambda value: value == label),
             ("ci", lambda value: value.casefold() == folded),
             ("contains", lambda value: folded in value.casefold()))
    for kind, test in tiers:
        hits = {res_id: len(value) for table in tables for res_id, value in table.items() if test(value)}
        if hits:
            ids = sorted(hits, key=lambda res_id: (hits[res_id], res_id))
            return kind, ids[:CONTAINS_LIMIT] if kind == "contains" else sorted(ids)
    return "none", []


def layout_refs(records) -> dict[int, list[str]]:
    """Map string id -> ``layout/id:attr`` for index records that reference it."""
    refs: dict[int, list[str]] = defaultdict(list)
    for record in records:
        for attr in STRING_ATTRS:
            if found := REF.match(record.get("attrs", {}).get(attr, "")):
                refs[int(found.group(1), 16)].append(f"{record['dir']}/{record['layout']}/{record['id'] or record['tag']}:{attr}")
    return refs


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Find string ids of UI labels for one build.")
    parser.add_argument("labels", nargs="*", help="label text as seen on screen")
    parser.add_argument("--labels-file", type=Path, help="one label per line")
    parser.add_argument("--apk", type=Path, required=True, help="the exact build to search")
    parser.add_argument("--locale", default="vi-VN", help="phone locale (default vi-VN)")
    parser.add_argument("--index", type=Path, help="layout_index.jsonl of the same build")
    parser.add_argument("--out", type=Path, help="TSV output (stdout by default)")
    args = parser.parse_args(argv)
    for stream in (sys.stdout, sys.stderr):
        getattr(stream, "reconfigure", lambda **_: None)(encoding="utf-8")

    labels = list(args.labels)
    try:
        if args.labels_file:
            labels += [line.strip() for line in args.labels_file.read_text(encoding="utf-8").splitlines() if line.strip()]
        if not labels:
            parser.error("pass labels or --labels-file")
        english = nxstring.load_apk_strings(args.apk, "default")
        localized = nxstring.load_apk_strings(args.apk, args.locale, with_default=False)
        refs: dict[int, list[str]] = {}
        if args.index:
            with open(args.index, encoding="utf-8") as src:
                refs = layout_refs(json.loads(line) for line in src if line.strip())
    except (OSError, ValueError, zipfile.BadZipFile) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    lines = ["label\tmatch\tid\ten\t" + args.locale + "\tlayouts"]
    unmatched = 0
    for label in dict.fromkeys(labels):
        kind, ids = match(label, [english, localized])
        if not ids:
            unmatched += 1
            lines.append(f"{nxstring.escape(label)}\tnone\t\t\t\t")
        for res_id in ids:
            used = refs.get(res_id, [])
            shown = ",".join(used[:REFS_LIMIT]) + (f" +{len(used) - REFS_LIMIT}" if len(used) > REFS_LIMIT else "")
            lines.append("\t".join(nxstring.escape(value) for value in (
                label, kind, f"0x{res_id:08x}", english.get(res_id, ""), localized.get(res_id, ""), shown)))
    text = "\n".join(lines) + "\n"
    if args.out:
        args.out.write_text(text, encoding="utf-8", newline="\n")
    else:
        sys.stdout.write(text)
    if unmatched:
        print(f"{unmatched} nhãn không có trong bảng chuỗi của build này", file=sys.stderr)
    return 0 if not unmatched else 1


if __name__ == "__main__":
    sys.exit(main())
