"""Check which TikTok view ids Riviu uses exist in one build (read-only).

Scans the repo's Rust sources for ``:id/<name>`` selectors (bare ``:id/x`` or
``com.zhiliaoapp.musically:id/x``; other packages such as Trill are skipped) and looks
each id up in a build's ``layout_index.py`` output and, optionally, its
``aapt2 dump resources`` text:

- ``unique``: declared in exactly one layout;
- ``ambiguous``: declared in several layouts, so a selector needs an ancestor/text scope;
- ``code-only``: in the resource table but in no layout (set from code at runtime);
- ``missing``: not in the resource table, the id does not exist in this build;
- ``not-in-layouts``: no layout and no ``--resources-dump`` to tell the last two apart.

Test code is skipped unless ``--include-tests``. Ids written as bare strings
(``"s0v"``) are not found by the scan; add them with ``--id``. ``--version 45.7.3`` keeps only ids with an occurrence within ``--window``
lines of that version literal, which approximates "the table for that build".
The TSV output can be fed to ``diff_ids.py --ids-file``.

    py -3.12 -I riviu_ids.py --index target\\tiktok-X\\layout_index.jsonl ^
        --resources-dump target\\tiktok-X\\aapt2-resources.txt --version 45.7.3 --out ids.tsv
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import resources_dump  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
DEFAULT_ROOTS = ("crates/core/src", "apps/desktop/src-tauri/src")
SELECTOR = re.compile(r"([A-Za-z_][\w.]*)?:id/([A-Za-z0-9_]+)")
PACKAGE = "com.zhiliaoapp.musically"


@dataclass
class Usage:
    locations: list[str] = field(default_factory=list)
    near_version: int = 0


def scan(files: list[tuple[str, str]], package: str, version: str | None, window: int) -> dict[str, Usage]:
    """``files`` is [(display path, text)]. Returns id -> usage."""
    found: dict[str, Usage] = defaultdict(Usage)
    literal = f'"{version}"' if version else None
    for path, text in files:
        lines = text.splitlines()
        version_lines = [n for n, line in enumerate(lines) if literal and literal in line]
        for number, line in enumerate(lines):
            for match in SELECTOR.finditer(line):
                prefix = match.group(1) or ""
                if prefix not in ("", package):
                    continue
                usage = found[match.group(2)]
                usage.locations.append(f"{path}:{number + 1}")
                if any(abs(number - v) <= window for v in version_lines):
                    usage.near_version += 1
    return found


def classify(name: str, layouts: dict[str, set[str]], table: resources_dump.ResourceTable | None,
             package_id: int) -> str:
    count = len(layouts.get(name, ()))
    if count == 1:
        return "unique"
    if count > 1:
        return "ambiguous"
    if table is None:
        return "not-in-layouts"
    return "code-only" if table.has_id(package_id, name) else "missing"


def index_layouts(path: Path) -> dict[str, set[str]]:
    layouts: dict[str, set[str]] = defaultdict(set)
    with open(path, encoding="utf-8") as src:
        for line in src:
            if line.strip():
                record = json.loads(line)
                if record["id"]:
                    layouts[record["id"]].add(f"{record['dir']}/{record['layout']}")
    return layouts


def production_text(path: str, text: str) -> str | None:
    """Drop test code: ``*tests.rs`` files, ``tests/`` dirs and everything from the
    first ``#[cfg(test)]`` line (Riviu keeps inline test modules at the file end)."""
    if path.endswith(("tests.rs", "test.rs")) or "/tests/" in f"/{path}":
        return None
    lines = text.splitlines()
    for number, line in enumerate(lines):
        if line.strip() == "#[cfg(test)]":
            return "\n".join(lines[:number])
    return text


def source_files(repo: Path, roots: list[str], include_tests: bool) -> list[tuple[str, str]]:
    files = []
    for root in roots:
        for path in sorted((repo / root).rglob("*.rs")):
            display = path.relative_to(repo).as_posix()
            text = path.read_text(encoding="utf-8", errors="replace")
            kept = text if include_tests else production_text(display, text)
            if kept is not None:
                files.append((display, kept))
    return files


HEADER = ["id", "status", "layouts", "near_version", "uses", "locations", "layout_names"]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Audit Riviu's TikTok view ids against one build.")
    parser.add_argument("--index", type=Path, required=True, help="layout_index.jsonl of the build")
    parser.add_argument("--resources-dump", type=Path, help="aapt2 dump resources of the same build")
    parser.add_argument("--repo", type=Path, default=REPO, help="Riviu checkout (default: this one)")
    parser.add_argument("--root", action="append", help=f"source roots to scan (default {', '.join(DEFAULT_ROOTS)})")
    parser.add_argument("--package", default=PACKAGE)
    parser.add_argument("--package-id", default="7f")
    parser.add_argument("--version", help="keep ids used near this version literal, e.g. 45.7.3")
    parser.add_argument("--window", type=int, default=30, help="lines around the version literal (default 30)")
    parser.add_argument("--include-tests", action="store_true", help="also scan test code and fixtures")
    parser.add_argument("--id", action="append", default=[], help="extra id not written as :id/<name>")
    parser.add_argument("--out", type=Path, help="TSV output (stdout by default)")
    args = parser.parse_args(argv)
    for stream in (sys.stdout, sys.stderr):
        getattr(stream, "reconfigure", lambda **_: None)(encoding="utf-8")

    try:
        usages = scan(source_files(args.repo, args.root or list(DEFAULT_ROOTS), args.include_tests), args.package, args.version, args.window)
        layouts = index_layouts(args.index)
        table = resources_dump.load(args.resources_dump) if args.resources_dump else None
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    for extra in args.id:
        usages.setdefault(extra, Usage(["--id"], 1))
    package_id = int(args.package_id, 16)
    rows = [HEADER]
    totals: dict[str, int] = defaultdict(int)
    for name in sorted(usages):
        usage = usages[name]
        if args.version and not usage.near_version:
            continue
        status = classify(name, layouts, table, package_id)
        totals[status] += 1
        rows.append([name, status, str(len(layouts.get(name, ()))), str(usage.near_version),
                     str(len(usage.locations)), ",".join(usage.locations[:4]),
                     ",".join(sorted(layouts.get(name, ()))[:4])])
    text = "\n".join("\t".join(row) for row in rows) + "\n"
    if args.out:
        args.out.write_text(text, encoding="utf-8", newline="\n")
    else:
        sys.stdout.write(text)
    print(" ".join(f"{key}={value}" for key, value in sorted(totals.items())), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
