#!/usr/bin/env python3
"""Stage pinned Bundletool and Temurin JRE for the Windows installer."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import subprocess
import tempfile
import urllib.request
import zipfile


BUNDLETOOL_VERSION = "1.18.3"
TEMURIN_VERSION = "21.0.12.1+1"
ANDROID_PACKAGE_TOOLS_TREE_SHA256 = (
    "f24951701beb69fe74ef073196c249d6df153749722f82260d79fc6687a7d57f"
)


@dataclass(frozen=True)
class PinnedArtifact:
    name: str
    url: str
    size: int
    sha256: str


BUNDLETOOL = PinnedArtifact(
    name="bundletool-all-1.18.3.jar",
    url="https://github.com/google/bundletool/releases/download/1.18.3/bundletool-all-1.18.3.jar",
    size=32_520_401,
    sha256="a099cfa1543f55593bc2ed16a70a7c67fe54b1747bb7301f37fdfd6d91028e29",
)
TEMURIN_JRE = PinnedArtifact(
    name="OpenJDK21U-jre_x64_windows_hotspot_21.0.12.1_1.zip",
    url=(
        "https://github.com/adoptium/temurin21-binaries/releases/download/"
        "jdk-21.0.12.1%2B1/OpenJDK21U-jre_x64_windows_hotspot_21.0.12.1_1.zip"
    ),
    size=48_999_141,
    sha256="d35f31e712f0fcf6ac5a093edc90204fbff22f720ba3950bd09d331d5e621636",
)


class StageError(RuntimeError):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify_pinned(path: Path, artifact: PinnedArtifact) -> None:
    size = path.stat().st_size
    if size != artifact.size:
        raise StageError(
            f"{artifact.name} size mismatch: expected {artifact.size}, got {size}"
        )
    digest = sha256_file(path)
    if digest != artifact.sha256:
        raise StageError(
            f"{artifact.name} SHA-256 mismatch: expected {artifact.sha256}, got {digest}"
        )


def download_pinned(cache: Path, artifact: PinnedArtifact) -> Path:
    cache.mkdir(parents=True, exist_ok=True)
    destination = cache / artifact.name
    if destination.is_file():
        verify_pinned(destination, artifact)
        return destination
    with tempfile.NamedTemporaryFile(dir=cache, delete=False) as handle:
        temporary = Path(handle.name)
    try:
        request = urllib.request.Request(
            artifact.url, headers={"User-Agent": "Riviu-package-stager/1"}
        )
        with urllib.request.urlopen(request, timeout=120) as response, temporary.open(
            "wb"
        ) as output:
            shutil.copyfileobj(response, output, length=1024 * 1024)
        verify_pinned(temporary, artifact)
        os.replace(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)
    return destination


def _safe_member_path(name: str) -> PurePosixPath:
    normalized = PurePosixPath(name.replace("\\", "/"))
    if normalized.is_absolute() or not normalized.parts or ".." in normalized.parts:
        raise StageError(f"unsafe JRE ZIP entry: {name!r}")
    if any(part in ("", ".") for part in normalized.parts):
        raise StageError(f"ambiguous JRE ZIP entry: {name!r}")
    if any(":" in part or part.endswith((" ", ".")) for part in normalized.parts):
        raise StageError(f"non-portable JRE ZIP entry: {name!r}")
    return normalized


def extract_jre(archive: Path, output: Path) -> None:
    with zipfile.ZipFile(archive) as source:
        members = source.infolist()
        if not members or len(members) > 4096:
            raise StageError(f"JRE ZIP entry count outside 1..4096: {len(members)}")
        roots: set[str] = set()
        seen: set[PurePosixPath] = set()
        seen_casefolded: set[str] = set()
        total = 0
        validated: list[tuple[zipfile.ZipInfo, PurePosixPath]] = []
        for member in members:
            path = _safe_member_path(member.filename)
            if path in seen:
                raise StageError(f"JRE ZIP contains a duplicate entry: {member.filename!r}")
            seen.add(path)
            casefolded = path.as_posix().casefold()
            if casefolded in seen_casefolded:
                raise StageError(
                    f"JRE ZIP contains a case-colliding entry: {member.filename!r}"
                )
            seen_casefolded.add(casefolded)
            roots.add(path.parts[0])
            mode = member.external_attr >> 16
            if stat.S_ISLNK(mode):
                raise StageError(f"JRE ZIP contains a symlink: {member.filename!r}")
            total += member.file_size
            if total > 512 * 1024 * 1024:
                raise StageError("expanded JRE exceeds 512 MiB")
            validated.append((member, path))
        if len(roots) != 1:
            raise StageError(f"JRE ZIP must have one top-level directory, got {sorted(roots)!r}")
        output.mkdir(parents=True, exist_ok=False)
        for member, path in validated:
            relative = Path(*path.parts[1:])
            if not relative.parts:
                continue
            destination = output / relative
            if member.is_dir():
                destination.mkdir(parents=True, exist_ok=True)
                continue
            destination.parent.mkdir(parents=True, exist_ok=True)
            with source.open(member) as input_file, destination.open("xb") as output_file:
                shutil.copyfileobj(input_file, output_file, length=1024 * 1024)
    java = output / "bin" / "java.exe"
    if not java.is_file():
        raise StageError("Temurin JRE archive does not contain bin/java.exe")


def tree_files(root: Path, *, ignored: set[str] | None = None) -> list[dict[str, object]]:
    ignored = ignored or set()
    entries: list[dict[str, object]] = []
    candidates = sorted(root.rglob("*"))
    symlinks = [path for path in candidates if path.is_symlink()]
    if symlinks:
        raise StageError(f"package-tools tree contains a symlink: {symlinks[0]}")
    for path in (candidate for candidate in candidates if candidate.is_file()):
        relative = path.relative_to(root).as_posix()
        if relative in ignored:
            continue
        entries.append(
            {"path": relative, "bytes": path.stat().st_size, "sha256": sha256_file(path)}
        )
    return entries


def tree_sha256(entries: list[dict[str, object]]) -> str:
    digest = hashlib.sha256()
    for entry in sorted(entries, key=lambda item: str(item["path"])):
        digest.update(str(entry["path"]).encode("utf-8"))
        digest.update(b"\0")
        digest.update(str(entry["bytes"]).encode("ascii"))
        digest.update(b"\0")
        digest.update(str(entry["sha256"]).encode("ascii"))
        digest.update(b"\n")
    return digest.hexdigest()


def verify_tree_manifest(root: Path, manifest: dict[str, object]) -> None:
    declared = manifest.get("files")
    if not isinstance(declared, list) or not declared:
        raise StageError("package-tools manifest has no file list")
    actual = tree_files(root, ignored={"android-package-tools-manifest.json"})
    if declared != actual:
        raise StageError("package-tools tree differs from its complete manifest")


def run_checked(command: list[str]) -> str:
    result = subprocess.run(command, capture_output=True, text=True, timeout=120)
    output = (result.stdout + result.stderr).strip()
    if result.returncode != 0:
        raise StageError(f"command failed ({result.returncode}): {command!r}: {output}")
    return output


def verify_tool_versions(java_version: str, bundletool_version: str) -> None:
    if TEMURIN_VERSION not in java_version:
        raise StageError(f"unexpected Java version output: {java_version!r}")
    if bundletool_version.strip() != BUNDLETOOL_VERSION:
        raise StageError(
            f"unexpected Bundletool version output: {bundletool_version!r}"
        )


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8", newline="\n")


def stage(output: Path, cache: Path, overlay: Path) -> dict[str, object]:
    if output.exists():
        raise StageError(f"staging output already exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    bundletool_source = download_pinned(cache, BUNDLETOOL)
    jre_source = download_pinned(cache, TEMURIN_JRE)
    with tempfile.TemporaryDirectory(prefix="android-package-tools-", dir=output.parent) as temporary:
        staged = Path(temporary) / "payload"
        staged.mkdir()
        shutil.copyfile(bundletool_source, staged / "bundletool.jar")
        extract_jre(jre_source, staged / "jre")
        java = staged / "jre" / "bin" / "java.exe"
        java_version = run_checked([str(java), "-version"])
        bundletool_version = run_checked([str(java), "-jar", str(staged / "bundletool.jar"), "version"])
        verify_tool_versions(java_version, bundletool_version)
        files = tree_files(staged)
        tree_digest = tree_sha256(files)
        if tree_digest != ANDROID_PACKAGE_TOOLS_TREE_SHA256:
            raise StageError(
                "Android package-tools extracted tree mismatch: "
                f"expected {ANDROID_PACKAGE_TOOLS_TREE_SHA256}, got {tree_digest}"
            )
        manifest: dict[str, object] = {
            "schemaVersion": 1,
            "fileCount": len(files),
            "payloadBytes": sum(int(entry["bytes"]) for entry in files),
            "treeSha256": tree_digest,
            "bundletool": {
                "version": BUNDLETOOL_VERSION,
                "source": BUNDLETOOL.url,
                "sourceBytes": BUNDLETOOL.size,
                "sourceSha256": BUNDLETOOL.sha256,
                "path": "bundletool.jar",
            },
            "jre": {
                "version": TEMURIN_VERSION,
                "source": TEMURIN_JRE.url,
                "sourceBytes": TEMURIN_JRE.size,
                "sourceSha256": TEMURIN_JRE.sha256,
                "javaPath": "jre/bin/java.exe",
                "versionOutput": java_version,
            },
            "files": files,
        }
        write_json(staged / "android-package-tools-manifest.json", manifest)
        verify_tree_manifest(staged, manifest)
        os.replace(staged, output)
    write_json(
        overlay,
        {
            "bundle": {
                "resources": {
                    output.resolve().as_posix() + "/": "sidecars/android-package-tools/"
                }
            }
        },
    )
    return manifest


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--tauri-config", type=Path, required=True)
    args = parser.parse_args()
    try:
        manifest = stage(args.output.resolve(), args.cache.resolve(), args.tauri_config.resolve())
    except (OSError, StageError, zipfile.BadZipFile) as error:
        print(json.dumps({"ok": False, "error": str(error)}, sort_keys=True))
        return 1
    print(json.dumps({"ok": True, "files": len(manifest["files"])}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
