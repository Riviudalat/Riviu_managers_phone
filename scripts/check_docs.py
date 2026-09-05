#!/usr/bin/env python3
"""Validate local Markdown links and anchors in tracked, non-vendored documents."""
from __future__ import annotations

from collections import Counter
from pathlib import Path
import re
import sys
import hashlib
import json
from urllib.parse import unquote, urlsplit

if __package__:
    from . import build_agents_index as index
else:
    import build_agents_index as index


def headings(body: str) -> set[str]:
    counts: Counter[str] = Counter()
    result = set()
    fence = ""
    for line in body.splitlines():
        opening = index.FENCE.match(line)
        if opening:
            marker = opening.group(1)
            if not fence:
                fence = marker
            elif marker[0] == fence[0] and len(marker) >= len(fence):
                fence = ""
            continue
        if fence:
            continue
        match = re.match(r"^ {0,3}#{1,6}\s+(.+?)(?:\s+#+)?$", line)
        if match:
            base = index.anchor(match.group(1))
            count = counts[base]
            counts[base] += 1
            result.add(base if not count else f"{base}-{count}")
        result.update(re.findall(r'<a\s+(?:id|name)=["\']([^"\']+)["\']', line))
    return result


def links(body: str) -> list[tuple[int, str]]:
    result = []
    fence = ""
    for number, line in enumerate(body.splitlines(), 1):
        opening = index.FENCE.match(line)
        if opening:
            marker = opening.group(1)
            if not fence:
                fence = marker
            elif marker[0] == fence[0] and len(marker) >= len(fence):
                fence = ""
            continue
        if fence or line.startswith("    "):
            continue
        # This repository uses inline links; angle brackets allow a path with spaces.
        for match in re.finditer(r'\]\((?:<([^>]+)>|([^\s)]+))(?:\s+"[^"]*")?\)', line):
            result.append((number, match.group(1) or match.group(2)))
    return result


def inspect(root: Path, paths: list[str]) -> list[str]:
    errors = []
    tracked = set(paths)
    for name in paths:
        if not name.endswith(".md") or name.startswith("sidecars/wda/WebDriverAgent/"):
            continue
        source = root / name
        if not source.is_file():
            continue
        for number, target in links(source.read_text(encoding="utf-8")):
            parts = urlsplit(target)
            if parts.scheme or parts.netloc:
                continue
            destination = (source.parent / unquote(parts.path)).resolve() if parts.path else source.resolve()
            try:
                relative = destination.relative_to(root.resolve()).as_posix()
            except ValueError:
                errors.append(f"{name}:{number}: link leaves repository: {target}")
                continue
            if not destination.exists():
                errors.append(f"{name}:{number}: missing link target: {target}")
            elif destination.is_file() and relative not in tracked:
                errors.append(f"{name}:{number}: link target is not tracked: {target}")
            elif parts.fragment and destination.suffix == ".md":
                fragment = unquote(parts.fragment)
                if fragment not in headings(destination.read_text(encoding="utf-8")):
                    errors.append(f"{name}:{number}: missing heading anchor: {target}")
    return errors


def main() -> int:
    paths = index.repository_files()
    errors = inspect(index.ROOT, paths)
    errors.extend(check_deleted_evidence(index.ROOT))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"documentation links valid ({sum(path.endswith('.md') for path in paths)} tracked Markdown files)")
    return 0


def check_deleted_evidence(root: Path) -> list[str]:
    manifest = root / "docs/archive/deletions-2026-09-06.json"
    if not manifest.is_file():
        return ["missing cleanup evidence manifest"]
    errors = []
    for entry in json.loads(manifest.read_text(encoding="utf-8"))["deletions"]:
        original = root / entry["path"]
        if original.exists():
            errors.append(f"deleted artifact reappeared: {entry['path']}")
        if entry["kind"] != "framed-jpeg-duplicate":
            continue
        jpeg = original.with_name(original.name.removeprefix(".")).with_suffix(".jpg")
        if not jpeg.is_file():
            errors.append(f"missing retained JPEG: {jpeg.relative_to(root)}")
            continue
        body = jpeg.read_bytes()
        reconstructed = len(body).to_bytes(4, "big") + body
        if len(reconstructed) != entry["bytes"] or hashlib.sha256(reconstructed).hexdigest() != entry["sha256"]:
            errors.append(f"framed evidence reconstruction mismatch: {entry['path']}")
    return errors


if __name__ == "__main__":
    raise SystemExit(main())
