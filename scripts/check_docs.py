#!/usr/bin/env python3
"""Validate local Markdown links and anchors in tracked, non-vendored documents."""
from __future__ import annotations

from collections import Counter
from pathlib import Path
import re
import sys
import hashlib
import html
import json
import subprocess
import unicodedata
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})")


def repository_files() -> list[str]:
    result = subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, check=True, capture_output=True)
    return result.stdout.decode("utf-8").rstrip("\0").split("\0")


def anchor(title: str) -> str:
    """GFM heading slug, including punctuation-created adjacent hyphens."""
    title = html.unescape(re.sub(r"<[^>]+>", "", title)).strip().lower()
    title = re.sub(r"\[([^]]+)\]\([^)]*\)", r"\1", title)
    title = "".join(char for char in title if char in "-_" or not unicodedata.category(char).startswith(("P", "S")))
    return re.sub(r"\s", "-", title)


def headings(body: str) -> set[str]:
    counts: Counter[str] = Counter()
    result = set()
    fence = ""
    for line in body.splitlines():
        opening = FENCE.match(line)
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
            base = anchor(match.group(1))
            count = counts[base]
            counts[base] += 1
            result.add(base if not count else f"{base}-{count}")
        result.update(re.findall(r'<a\s+(?:id|name)=["\']([^"\']+)["\']', line))
    return result


def links(body: str) -> list[tuple[int, str]]:
    result = []
    fence = ""
    definitions = {}
    references = []
    for number, line in enumerate(body.splitlines(), 1):
        opening = FENCE.match(line)
        if opening:
            marker = opening.group(1)
            if not fence:
                fence = marker
            elif marker[0] == fence[0] and len(marker) >= len(fence):
                fence = ""
            continue
        if fence or line.startswith("    "):
            continue
        definition = re.match(r'^ {0,3}\[([^]]+)\]:\s*(?:<([^>]+)>|(\S+))', line)
        if definition:
            definitions[' '.join(definition.group(1).casefold().split())] = definition.group(2) or definition.group(3)
            continue
        for match in re.finditer(r'\]\((?:<([^>]+)>|([^\s)]+))(?:\s+"[^"]*")?\)', line):
            result.append((number, match.group(1) or match.group(2)))
        for match in re.finditer(r'\[([^]]+)\]\[([^]]*)\]', line):
            references.append((number, ' '.join((match.group(2) or match.group(1)).casefold().split())))
    for number, label in references:
        # Unknown labels render as plain text, not links; known definitions are checked.
        if label in definitions:
            result.append((number, definitions[label]))
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
        body = source.read_text(encoding="utf-8")
        current = (name in {"README.md", "AGENTS.md", "docs/README.md", "docs/operator-guide.md", "docs/developer-guide.md"}
                   or name.startswith(("docs/agents/", "docs/operator/", "docs/development/", ".claude/skills/")))
        if current:
            for number, line in enumerate(body.splitlines(), 1):
                if re.search(r"§\s*9\.\d+", line):
                    errors.append(f"{name}:{number}: replace historical section shorthand with an explicit source link")
        for number, target in links(body):
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


def main() -> int:
    paths = repository_files()
    if "--include-untracked" in sys.argv:
        # Review new documentation before staging without changing the user's index.
        result = subprocess.run(["git", "ls-files", "--others", "--exclude-standard", "-z"], cwd=ROOT, check=True, capture_output=True)
        paths = sorted(set(paths) | set(result.stdout.decode("utf8").rstrip("\0").split("\0")))
    errors = inspect(ROOT, paths)
    errors.extend(check_deleted_evidence(ROOT))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    label = "tracked + new" if "--include-untracked" in sys.argv else "tracked"
    print(f"documentation links valid ({sum(path.endswith('.md') for path in paths)} {label} Markdown files)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
