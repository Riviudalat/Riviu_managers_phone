"""Resolve ``@<decimal>`` text/content-desc tokens to strings for one build.

When TikTok's accessibility tree exposes a string it could not resolve, uiautomator
shows the raw resource id as ``@2131823090``. The number is a string id of that exact
build: the same in every language, different in the next build. Tokens come from the
command line or are collected from uiautomator dumps/fixtures with ``--scan``.

    py -3.12 -I resolve_desc.py --apk base.apk --locale vi-VN @2131823090 @2131823130
    py -3.12 -I resolve_desc.py --apk base.apk --locale vi-VN --scan path\\to\\dumps

Output TSV: token, hex id, English base value, ``--locale`` value (empty when the
locale table has no entry, i.e. the phone falls back to English), and where it was seen.
"""

from __future__ import annotations

import argparse
import re
import sys
import zipfile
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import nxstring  # noqa: E402

TOKEN = re.compile(r"^@(\d{9,10})$")
# Any start tag: plain dumps use <node>, the class-name form (UiAutomator2 page source)
# names each element after its class, e.g. <android.widget.ImageView ...>.
NODE = re.compile(r"<(?![?!/])[\w.$-]+([^>]*)>")
ATTR = re.compile(r'([\w-]+)="([^"]*)"')
SCAN_SUFFIXES = (".xml", ".fixture", ".txt")


def parse_token(token: str) -> int:
    match = TOKEN.match(token.strip())
    if not match:
        raise ValueError(f"not an @<decimal> token: {token!r}")
    return int(match.group(1))


def scan_text(text: str, source: str, found: dict[int, set[str]]) -> None:
    """Collect tokens from text/content-desc attributes of uiautomator elements in either tag form."""
    for node in NODE.finditer(text):
        attrs = dict(ATTR.findall(node.group(1)))
        resource_id = attrs.get("resource-id", "").split(":id/")[-1] or "-"
        for attr in ("text", "content-desc"):
            if match := TOKEN.match(attrs.get(attr, "")):
                found[int(match.group(1))].add(f"{source}:{resource_id}:{attr}")


def rows(found: dict[int, set[str]], english: dict[int, str], localized: dict[int, str]):
    for res_id in sorted(found):
        yield (f"@{res_id}", f"0x{res_id:08x}", english.get(res_id), localized.get(res_id),
               ",".join(sorted(found[res_id])[:6]))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Resolve @<decimal> tokens to strings for one build.")
    parser.add_argument("tokens", nargs="*", help="tokens such as @2131823090")
    parser.add_argument("--apk", type=Path, required=True, help="the exact build the tokens came from")
    parser.add_argument("--locale", default="vi-VN", help="phone locale (default vi-VN)")
    parser.add_argument("--scan", type=Path, action="append", default=[],
                        help="uiautomator dump file or directory to collect tokens from")
    parser.add_argument("--out", type=Path, help="TSV output (stdout by default)")
    args = parser.parse_args(argv)
    for stream in (sys.stdout, sys.stderr):
        getattr(stream, "reconfigure", lambda **_: None)(encoding="utf-8")

    found: dict[int, set[str]] = defaultdict(set)
    try:
        for token in args.tokens:
            found[parse_token(token)].add("argv")
        for root in args.scan:
            files = [root] if root.is_file() else sorted(p for p in root.rglob("*") if p.suffix in SCAN_SUFFIXES)
            for path in files:
                scan_text(path.read_text(encoding="utf-8", errors="replace"), path.name, found)
        english = nxstring.load_apk_strings(args.apk, "default")
        # Only the locale's own tables say whether a translation exists.
        localized = nxstring.load_apk_strings(args.apk, args.locale, with_default=False)
    except (OSError, ValueError, zipfile.BadZipFile) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    lines = ["token\tid\ten\t" + args.locale + "\tseen"]
    for token, hex_id, en, loc, seen in rows(found, english, localized):
        lines.append("\t".join(nxstring.escape(value or "") for value in (token, hex_id, en, loc, seen)))
    text = "\n".join(lines) + "\n"
    if args.out:
        args.out.write_text(text, encoding="utf-8", newline="\n")
    else:
        sys.stdout.write(text)
    missing = [row for row in rows(found, english, localized) if row[2] is None]
    if missing:
        print(f"{len(missing)} token không có trong bảng của build này (sai build?)", file=sys.stderr)
    return 0 if not missing else 1


if __name__ == "__main__":
    sys.exit(main())
