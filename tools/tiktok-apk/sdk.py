"""Locate Android SDK build tools (aapt2, apksigner) from an SDK root."""

from __future__ import annotations

import os
from pathlib import Path

EXECUTABLE_NAMES = {
    "aapt2": ("aapt2.exe", "aapt2"),
    "apksigner": ("apksigner.bat", "apksigner"),
}


def _version_key(path: Path) -> tuple[int, ...]:
    return tuple(int(part) if part.isdigit() else -1 for part in path.name.replace("-", ".").split("."))


def build_tool(sdk_root: Path, tool: str) -> Path:
    """Newest ``build-tools/<version>/<tool>`` under ``sdk_root``.

    ``sdk_root`` may also point straight at one build-tools version directory.
    """
    names = EXECUTABLE_NAMES[tool]
    if os.name != "nt":
        names = tuple(reversed(names))
    candidates = [sdk_root] + sorted((sdk_root / "build-tools").glob("*"), key=_version_key, reverse=True)
    for directory in candidates:
        for name in names:
            path = directory / name
            if path.is_file():
                return path
    raise FileNotFoundError(f"{tool} not found under {sdk_root} or {sdk_root / 'build-tools'}")
