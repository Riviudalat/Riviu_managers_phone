"""Pinned dependency and native SQLite checks; does not install packages."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--toolchains", action="store_true")
    parser.add_argument("--python-runtime", action="store_true")
    args = parser.parse_args()
    cargo = tomllib.loads((ROOT / "Cargo.lock").read_text())
    versions = {p["name"]: p["version"] for p in cargo["package"]}
    assert versions["rusqlite"] == "0.40.2"
    assert versions["libsqlite3-sys"] == "0.38.2"
    assert versions["ts-rs"] == "12.0.1"
    package = json.loads((ROOT / "apps/desktop/package.json").read_text())
    lock = json.loads((ROOT / "apps/desktop/package-lock.json").read_text())
    for name, expected in [("vitest", "4.1.11"), ("@tanstack/react-query", "5.103.1")]:
        assert (package["dependencies"] | package["devDependencies"])[name] == expected
        assert lock["packages"]["node_modules/" + name]["version"] == expected
    if args.python_runtime:
        from scripts.build_desktop_sidecar import dependency_closure
        closure = dependency_closure()
        assert closure["tornado"] == "6.5.10"
        assert closure["cryptography"] == ("50.0.1" if sys.platform == "win32" else "48.0.0")
    if args.toolchains:
        rust = subprocess.check_output(["rustc", "--version"], text=True).split()[1]
        expected_rust = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        assert rust == expected_rust, (rust, expected_rust)
        assert subprocess.check_output(["node", "--version"], text=True).strip() == "v24.15.0"
        assert sys.version_info[:3] == (3, 12, 10), sys.version
    print("Dependency manifests and lockfiles match")


if __name__ == "__main__":
    sys.path.insert(0, str(ROOT))
    main()
