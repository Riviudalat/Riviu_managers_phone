#!/usr/bin/env python3
"""Prepare the pinned WDA baseline and apply the Riviu-owned patch series."""

from __future__ import annotations

import argparse
import base64
import binascii
import hashlib
import hmac
import json
import os
import shutil
import stat
import subprocess
import tarfile
import tempfile
from pathlib import Path, PurePosixPath
from typing import Any, Mapping, NamedTuple


MAX_ARCHIVE_MEMBERS = 50_000
MAX_ARCHIVE_FILE_BYTES = 32 * 1024 * 1024
MAX_ARCHIVE_TOTAL_BYTES = 512 * 1024 * 1024


class PrepareError(RuntimeError):
    pass


class PreparationResult(NamedTuple):
    baseline_source_sha256: str
    output_source_sha256: str
    patch_count: int
    output: Path

    def as_json(self) -> dict[str, Any]:
        return {
            "baselineSourceSha256": self.baseline_source_sha256,
            "outputSourceSha256": self.output_source_sha256,
            "patchCount": self.patch_count,
            "output": str(self.output),
        }


def _load_json(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise PrepareError(f"failed to read {label}: {path}") from exc
    if not isinstance(value, dict):
        raise PrepareError(f"{label} must contain a JSON object")
    return value


def _required_string(value: dict[str, Any], key: str, label: str) -> str:
    result = value.get(key)
    if not isinstance(result, str) or not result.strip():
        raise PrepareError(f"{label} field {key} must be a nonblank string")
    return result


def _safe_child(root: Path, relative: str, label: str) -> Path:
    posix = PurePosixPath(relative)
    if (
        not relative
        or "\\" in relative
        or posix.is_absolute()
        or any(part in {"", ".", ".."} for part in posix.parts)
    ):
        raise PrepareError(f"unsafe {label}: {relative}")
    candidate = root.joinpath(*posix.parts)
    try:
        candidate.resolve(strict=False).relative_to(root.resolve(strict=False))
    except ValueError as exc:
        raise PrepareError(f"unsafe {label}: {relative}") from exc
    return candidate


def _verify_project_lock(lock_path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    project = _load_json(lock_path, "project baseline lock")
    if project.get("schemaVersion") != 1:
        raise PrepareError("unsupported project baseline lock schemaVersion")

    upstream_relative = _required_string(project, "upstreamLock", "project lock")
    upstream_reference = PurePosixPath(upstream_relative)
    if "\\" in upstream_relative or upstream_reference.is_absolute():
        raise PrepareError(f"unsafe upstream lock path: {upstream_relative}")
    # The project lock intentionally references the repository-level Gate A lock.
    upstream_path = lock_path.parent.joinpath(*upstream_reference.parts).resolve(strict=False)
    upstream = _load_json(upstream_path, "upstream baseline lock")

    for key in ("package", "version", "gitHead", "integrity"):
        project_value = _required_string(project, key, "project lock")
        upstream_value = _required_string(upstream, key, "upstream lock")
        if project_value != upstream_value:
            raise PrepareError(f"project lock {key} does not match upstream lock")

    archive_sha256 = _required_string(project, "archiveSha256", "project lock")
    if len(archive_sha256) != 64 or any(c not in "0123456789abcdefABCDEF" for c in archive_sha256):
        raise PrepareError("project lock archiveSha256 must be 64 hexadecimal characters")

    for field in ("baselineSourceSha256", "outputSourceSha256"):
        expected_source = project.get(field)
        if expected_source is not None and (
            not isinstance(expected_source, str)
            or len(expected_source) != 64
            or any(c not in "0123456789abcdefABCDEF" for c in expected_source)
        ):
            raise PrepareError(
                f"project lock {field} must be null or 64 hexadecimal characters"
            )

    patches = project.get("patches")
    if not isinstance(patches, list):
        raise PrepareError("project lock patches must be an array")
    return project, upstream


def _verify_archive_bytes(archive_bytes: bytes, project: dict[str, Any]) -> None:
    expected_sha256 = _required_string(project, "archiveSha256", "project lock")
    actual_sha256 = hashlib.sha256(archive_bytes).hexdigest()
    if not _constant_time_ascii_equal(actual_sha256, expected_sha256.lower()):
        raise PrepareError("archive SHA-256 mismatch")

    integrity = _required_string(project, "integrity", "project lock")
    if not integrity.startswith("sha512-"):
        raise PrepareError("baseline integrity must use sha512")
    try:
        expected_sha512 = base64.b64decode(integrity[7:], validate=True)
    except (ValueError, binascii.Error) as exc:
        raise PrepareError("baseline integrity is not valid base64") from exc
    actual_sha512 = hashlib.sha512(archive_bytes).digest()
    if not hmac.compare_digest(actual_sha512, expected_sha512):
        raise PrepareError("baseline integrity mismatch")


def _constant_time_ascii_equal(left: str, right: str) -> bool:
    return hmac.compare_digest(left.encode("ascii"), right.encode("ascii"))


def _archive_relative_path(name: str) -> PurePosixPath:
    path = PurePosixPath(name)
    if (
        not name
        or "\\" in name
        or path.is_absolute()
        or any(part in {"", ".", ".."} for part in path.parts)
        or len(path.parts) < 2
        or path.parts[0] != "package"
    ):
        raise PrepareError(f"unsafe archive path: {name}")
    return PurePosixPath(*path.parts[1:])


def _normalized_file_mode(mode: int) -> int:
    return 0o755 if mode & 0o111 else 0o644


def _extract_archive(archive_path: Path, destination: Path) -> tuple[Path, dict[str, int]]:
    package_root = destination / "package"
    package_root.mkdir(parents=True, exist_ok=False)
    archive_modes: dict[str, int] = {}
    seen: set[str] = set()
    total_bytes = 0

    try:
        archive = tarfile.open(archive_path, mode="r:gz")
    except (OSError, tarfile.TarError) as exc:
        raise PrepareError(f"failed to open baseline archive: {archive_path}") from exc

    with archive:
        for index, member in enumerate(archive):
            if index >= MAX_ARCHIVE_MEMBERS:
                raise PrepareError("baseline archive has too many members")
            relative = _archive_relative_path(member.name)
            normalized = relative.as_posix()
            if normalized in seen:
                raise PrepareError(f"duplicate archive member: {member.name}")
            seen.add(normalized)

            target = _safe_child(package_root, normalized, "archive path")
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                target.chmod(0o755)
                continue
            if not member.isfile():
                raise PrepareError(f"unsupported archive member: {member.name}")
            if member.size < 0 or member.size > MAX_ARCHIVE_FILE_BYTES:
                raise PrepareError(f"archive member exceeds size limit: {member.name}")
            total_bytes += member.size
            if total_bytes > MAX_ARCHIVE_TOTAL_BYTES:
                raise PrepareError("baseline archive exceeds total size limit")

            source = archive.extractfile(member)
            if source is None:
                raise PrepareError(f"failed to read archive member: {member.name}")
            target.parent.mkdir(parents=True, exist_ok=True)
            written = 0
            with source, target.open("xb") as output:
                while True:
                    chunk = source.read(min(1024 * 1024, member.size - written + 1))
                    if not chunk:
                        break
                    written += len(chunk)
                    if written > member.size:
                        raise PrepareError(f"archive member size mismatch: {member.name}")
                    output.write(chunk)
            if written != member.size:
                raise PrepareError(f"archive member size mismatch: {member.name}")
            normalized_mode = _normalized_file_mode(member.mode)
            target.chmod(normalized_mode)
            archive_modes[normalized] = normalized_mode

    return package_root, archive_modes


def _validate_package(package_root: Path, project: dict[str, Any]) -> None:
    package = _load_json(package_root / "package.json", "npm package metadata")
    for key in ("name", "version"):
        expected_key = "package" if key == "name" else key
        expected = _required_string(project, expected_key, "project lock")
        actual = package.get(key)
        if actual != expected:
            raise PrepareError(f"npm package {key} does not match locked {expected_key}")


def source_documents_sha256(
    root: Path, archive_modes: Mapping[str, int] | None = None
) -> str:
    documents: list[tuple[str, int, bytes]] = []
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        relative = path.relative_to(root).as_posix()
        filesystem_mode = _normalized_file_mode(stat.S_IMODE(path.stat().st_mode))
        mode = (
            filesystem_mode
            if os.name == "posix" or archive_modes is None
            else archive_modes.get(relative, filesystem_mode)
        )
        documents.append((relative, mode, path.read_bytes()))
    documents.sort(key=lambda item: item[0])

    digest = hashlib.sha256()
    for relative, mode, contents in documents:
        path_bytes = relative.encode("utf-8")
        digest.update(len(path_bytes).to_bytes(8, "little"))
        digest.update(path_bytes)
        digest.update(mode.to_bytes(4, "little"))
        digest.update(len(contents).to_bytes(8, "little"))
        digest.update(hashlib.sha256(contents).digest())
    return digest.hexdigest()


def _patch_paths(project: dict[str, Any], lock_path: Path) -> list[Path]:
    result: list[Path] = []
    for index, item in enumerate(project["patches"]):
        if not isinstance(item, dict):
            raise PrepareError(f"patch entry {index} must be an object")
        relative = _required_string(item, "path", f"patch entry {index}")
        expected = _required_string(item, "sha256", f"patch entry {index}")
        if len(expected) != 64 or any(c not in "0123456789abcdefABCDEF" for c in expected):
            raise PrepareError(f"patch entry {index} sha256 must be hexadecimal")
        path = _safe_child(lock_path.parent, relative, "patch path")
        try:
            contents = path.read_bytes()
        except OSError as exc:
            raise PrepareError(f"failed to read patch: {relative}") from exc
        actual = hashlib.sha256(contents).hexdigest()
        if not _constant_time_ascii_equal(actual, expected.lower()):
            raise PrepareError(f"patch checksum mismatch: {relative}")
        result.append(path)
    return result


def _apply_patches(package_root: Path, patches: list[Path]) -> None:
    git_environment = os.environ.copy()
    # Generated sources normally live below the repository's ignored target/ tree.
    # Stop Git discovery at the staging parent so patches cannot be silently skipped
    # or redirected to the enclosing worktree.
    git_environment["GIT_CEILING_DIRECTORIES"] = str(package_root.parent.resolve())
    for patch in patches:
        check = subprocess.run(
            [
                "git",
                "-c",
                "core.autocrlf=false",
                "apply",
                "--check",
                "--whitespace=nowarn",
                str(patch),
            ],
            cwd=package_root,
            env=git_environment,
            capture_output=True,
            text=True,
            shell=False,
        )
        if check.returncode != 0:
            raise PrepareError(f"patch does not apply cleanly: {patch.name}: {check.stderr.strip()}")
        apply = subprocess.run(
            [
                "git",
                "-c",
                "core.autocrlf=false",
                "apply",
                "--whitespace=nowarn",
                str(patch),
            ],
            cwd=package_root,
            env=git_environment,
            capture_output=True,
            text=True,
            shell=False,
        )
        if apply.returncode != 0:
            raise PrepareError(f"failed to apply patch: {patch.name}: {apply.stderr.strip()}")


def _replace_directory(candidate: Path, output: Path) -> None:
    backup = output.with_name(f".{output.name}.backup-{os.getpid()}")
    if backup.exists():
        shutil.rmtree(backup)
    if output.exists():
        output.replace(backup)
    try:
        candidate.replace(output)
    except BaseException:
        if backup.exists() and not output.exists():
            backup.replace(output)
        raise
    if backup.exists():
        shutil.rmtree(backup)


def prepare_source(archive: Path, lock_path: Path, output: Path) -> PreparationResult:
    archive = Path(archive).resolve()
    lock_path = Path(lock_path).resolve()
    output = Path(output).resolve()
    project, _upstream = _verify_project_lock(lock_path)
    try:
        archive_bytes = archive.read_bytes()
    except OSError as exc:
        raise PrepareError(f"failed to read baseline archive: {archive}") from exc
    _verify_archive_bytes(archive_bytes, project)
    patches = _patch_paths(project, lock_path)

    output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=".riviu-agent-prepare-", dir=output.parent))
    try:
        package_root, archive_modes = _extract_archive(archive, staging)
        _validate_package(package_root, project)
        baseline_digest = source_documents_sha256(package_root, archive_modes)
        expected_baseline = project.get("baselineSourceSha256")
        if expected_baseline is not None and not _constant_time_ascii_equal(
            baseline_digest, expected_baseline.lower()
        ):
            raise PrepareError("baseline source digest mismatch")

        _apply_patches(package_root, patches)
        output_digest = source_documents_sha256(package_root, archive_modes)
        expected_output = project.get("outputSourceSha256")
        if expected_output is not None and not _constant_time_ascii_equal(
            output_digest, expected_output.lower()
        ):
            raise PrepareError("output source digest mismatch")
        _replace_directory(package_root, output)
        return PreparationResult(baseline_digest, output_digest, len(patches), output)
    finally:
        if staging.exists():
            shutil.rmtree(staging)


def _default_lock_path() -> Path:
    return Path(__file__).resolve().parents[1] / "baseline-lock.json"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--lock", type=Path, default=_default_lock_path())
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = prepare_source(args.archive, args.lock, args.output)
    except PrepareError as exc:
        print(json.dumps({"ok": False, "error": str(exc)}, ensure_ascii=True))
        return 1
    print(json.dumps({"ok": True, **result.as_json()}, ensure_ascii=True, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
