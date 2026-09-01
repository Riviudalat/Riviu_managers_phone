#!/usr/bin/env python3
"""Generate an ICE-clean WiX fragment for Tauri's per-user MSI resources.

Tauri emits one file-keyed component per resource.  That is valid for its
per-machine template, but WiX ICE38 rejects those components when INSTALLDIR
lives below LocalAppDataFolder.  A per-user component must instead use an HKCU
registry value as its key path.  This generator keeps Tauri's resource mapping
semantics while authoring those registry-keyed components explicitly.

The generated JSON overlay replaces Tauri's resource map for the MSI build
only and attaches the generated component group.  NSIS continues to use the
normal Tauri resource map.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import uuid
from dataclasses import dataclass
from pathlib import Path, PurePosixPath, PureWindowsPath
from typing import Iterable
from xml.sax.saxutils import quoteattr


WIX_NAMESPACE = "http://schemas.microsoft.com/wix/2006/wi"
COMPONENT_NAMESPACE = uuid.UUID("d81f39f9-63fc-48ff-8ce8-d68b2a30b66b")
COMPONENT_GROUP_ID = "RiviuPerUserResources"
REGISTRY_KEY = r"Software\riviu\Riviu Manager Full\Components"


@dataclass(frozen=True, order=True)
class ResourceFile:
    destination: PurePosixPath
    source: Path


def _stable_id(prefix: str, value: str) -> str:
    return prefix + hashlib.sha256(value.encode("utf-8")).hexdigest()[:30]


def _load_resource_map(config_path: Path) -> dict[str, str]:
    data = json.loads(config_path.read_text(encoding="utf-8"))
    resources = data.get("bundle", {}).get("resources", {})
    if resources in (None, []):
        return {}
    if not isinstance(resources, dict):
        raise ValueError(f"{config_path}: bundle.resources must be an object")
    return {str(source): str(destination) for source, destination in resources.items()}


def _safe_destination(value: str) -> PurePosixPath:
    raw = value.replace("\\", "/")
    windows_path = PureWindowsPath(raw)
    if raw.startswith("/") or windows_path.is_absolute() or windows_path.drive:
        raise ValueError(f"unsafe resource destination: {value!r}")
    normalized = raw.strip("/")
    destination = PurePosixPath(normalized)
    if (
        not normalized
        or normalized == "."
        or destination.is_absolute()
        or ".." in destination.parts
        or any(":" in part for part in destination.parts)
    ):
        raise ValueError(f"unsafe resource destination: {value!r}")
    return destination


def _expand_mapping(config_path: Path, source_text: str, destination_text: str) -> Iterable[ResourceFile]:
    source = Path(source_text)
    if not source.is_absolute():
        source = config_path.parent / source
    source = source.resolve()
    destination = _safe_destination(destination_text)
    if source.is_file():
        yield ResourceFile(destination, source)
        return
    if not source.is_dir():
        raise FileNotFoundError(f"resource source does not exist: {source}")
    for child in sorted((item for item in source.rglob("*") if item.is_file()), key=lambda item: item.as_posix().lower()):
        relative = PurePosixPath(*child.relative_to(source).parts)
        yield ResourceFile(destination / relative, child.resolve())


def collect_resources(config_paths: Iterable[Path]) -> list[ResourceFile]:
    by_destination: dict[PurePosixPath, Path] = {}
    for config_path in config_paths:
        for source_text, destination_text in _load_resource_map(config_path).items():
            for resource in _expand_mapping(config_path, source_text, destination_text):
                existing = by_destination.get(resource.destination)
                if existing is not None and existing != resource.source:
                    raise ValueError(
                        f"duplicate resource destination {resource.destination}: {existing} and {resource.source}"
                    )
                by_destination[resource.destination] = resource.source
    return [ResourceFile(destination, source) for destination, source in sorted(by_destination.items())]


def _directory_tree(resources: Iterable[ResourceFile]) -> dict[PurePosixPath, list[ResourceFile]]:
    directories: dict[PurePosixPath, list[ResourceFile]] = {PurePosixPath("."): []}
    for resource in resources:
        parent = resource.destination.parent
        directories.setdefault(parent, []).append(resource)
        while parent != PurePosixPath("."):
            parent = parent.parent
            directories.setdefault(parent, [])
    return directories


def render_fragment(resources: list[ResourceFile], *, win64: bool) -> str:
    directories = _directory_tree(resources)
    component_ids: list[str] = []
    win64_text = "yes" if win64 else "no"

    def render_directory(path: PurePosixPath, indent: str) -> list[str]:
        lines: list[str] = []
        for resource in sorted(directories[path], key=lambda item: item.destination.as_posix()):
            destination_text = resource.destination.as_posix()
            component_id = _stable_id("Resource_", destination_text)
            file_id = _stable_id("File_", destination_text)
            component_ids.append(component_id)
            guid = str(uuid.uuid5(COMPONENT_NAMESPACE, "file:" + destination_text))
            lines.extend(
                [
                    f'{indent}<Component Id={quoteattr(component_id)} Guid={quoteattr(guid)} Win64={quoteattr(win64_text)}>',
                    f'{indent}  <File Id={quoteattr(file_id)} Source={quoteattr(str(resource.source))} KeyPath="no" />',
                    f'{indent}  <RegistryValue Root="HKCU" Key={quoteattr(REGISTRY_KEY)} Name={quoteattr(component_id)} Type="integer" Value="1" KeyPath="yes" />',
                    f"{indent}</Component>",
                ]
            )

        children = sorted(
            (candidate for candidate in directories if candidate.parent == path and candidate != path),
            key=lambda candidate: candidate.name.lower(),
        )
        for child in children:
            directory_id = _stable_id("Directory_", child.as_posix())
            cleanup_id = _stable_id("Cleanup_", child.as_posix())
            component_ids.append(cleanup_id)
            guid = str(uuid.uuid5(COMPONENT_NAMESPACE, "directory:" + child.as_posix()))
            lines.append(f'{indent}<Directory Id={quoteattr(directory_id)} Name={quoteattr(child.name)}>')
            lines.extend(
                [
                    f'{indent}  <Component Id={quoteattr(cleanup_id)} Guid={quoteattr(guid)} Win64={quoteattr(win64_text)}>',
                    f'{indent}    <RemoveFolder Id={quoteattr(_stable_id("Remove_", child.as_posix()))} On="uninstall" />',
                    f'{indent}    <RegistryValue Root="HKCU" Key={quoteattr(REGISTRY_KEY)} Name={quoteattr(cleanup_id)} Type="integer" Value="1" KeyPath="yes" />',
                    f"{indent}  </Component>",
                ]
            )
            lines.extend(render_directory(child, indent + "  "))
            lines.append(f"{indent}</Directory>")
        return lines

    body = render_directory(PurePosixPath("."), "      ")
    refs = [f'      <ComponentRef Id={quoteattr(component_id)} />' for component_id in component_ids]
    return "\n".join(
        [
            '<?xml version="1.0" encoding="UTF-8"?>',
            f'<Wix xmlns={quoteattr(WIX_NAMESPACE)}>',
            "  <Fragment>",
            '    <DirectoryRef Id="INSTALLDIR">',
            *body,
            "    </DirectoryRef>",
            "  </Fragment>",
            "  <Fragment>",
            f'    <ComponentGroup Id="{COMPONENT_GROUP_ID}">',
            *refs,
            "    </ComponentGroup>",
            "  </Fragment>",
            "</Wix>",
            "",
        ]
    )


def write_outputs(
    config_paths: list[Path], fragment_output: Path, config_output: Path, *, target: str
) -> tuple[int, str]:
    resources = collect_resources(config_paths)
    fragment_output.parent.mkdir(parents=True, exist_ok=True)
    fragment_output.write_text(
        render_fragment(resources, win64=target != "i686-pc-windows-msvc"),
        encoding="utf-8",
        newline="\n",
    )
    overlay = {
        "bundle": {
            "resources": [],
            "windows": {
                "wix": {
                    "fragmentPaths": [fragment_output.resolve().as_posix()],
                    "componentGroupRefs": [COMPONENT_GROUP_ID],
                }
            },
        }
    }
    config_output.parent.mkdir(parents=True, exist_ok=True)
    config_output.write_text(json.dumps(overlay, indent=2) + "\n", encoding="utf-8", newline="\n")
    digest = hashlib.sha256(fragment_output.read_bytes()).hexdigest()
    return len(resources), digest


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", action="append", required=True, type=Path)
    parser.add_argument("--fragment-output", required=True, type=Path)
    parser.add_argument("--config-output", required=True, type=Path)
    parser.add_argument("--target", default="x86_64-pc-windows-msvc")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    count, digest = write_outputs(
        [path.resolve() for path in args.config],
        args.fragment_output.resolve(),
        args.config_output.resolve(),
        target=args.target,
    )
    print(json.dumps({"resourceFileCount": count, "fragmentSha256": digest}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
