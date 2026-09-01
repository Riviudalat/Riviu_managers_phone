from __future__ import annotations

import argparse
import shutil
from pathlib import Path


BINARY_NAME = "riviu-deployment-check"


def stage_binary(source: Path, output_dir: Path, target: str) -> Path:
    if not source.is_file():
        raise FileNotFoundError(f"deployment checker binary is missing: {source}")
    output_dir.mkdir(parents=True, exist_ok=True)
    suffix = ".exe" if target.endswith("windows-msvc") else ""
    destination = output_dir / f"{BINARY_NAME}-{target}{suffix}"
    shutil.copy2(source, destination)
    return destination


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--target", required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    staged = stage_binary(args.input, args.output_dir, args.target)
    print(staged)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
