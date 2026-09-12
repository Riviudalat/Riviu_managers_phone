"""Build the isolated perception service and attest every packaged file."""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = ROOT / "sidecars/gui-service"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=pathlib.Path, default=ROOT / "target/gui-service")
    args = parser.parse_args()
    output = args.output.resolve()
    subprocess.run(["uv", "sync", "--frozen", "--python", "3.12"], cwd=SOURCE, check=True)
    subprocess.run(["uv", "run", "--frozen", "pyinstaller", "--noconfirm", "--clean", "--onedir",
                    "--name", "riviu-gui-service", "--distpath", str(output),
                    "--workpath", str(ROOT / "target/gui-service-work"),
                    "--specpath", str(ROOT / "target/gui-service-work"),
                    "--collect-submodules", "uvicorn", "--collect-submodules", "riviu_gui", "serve.py"],
                   cwd=SOURCE, check=True)
    runtime = output / "riviu-gui-service"
    # Preserve upstream attribution from the exact locked environment.
    subprocess.run(["uv","run","--frozen","python","collect_licenses.py",str(runtime / "licenses")],cwd=SOURCE,check=True)
    files = [{"path": str(p.relative_to(runtime)).replace("\\", "/"), "bytes": p.stat().st_size,
              "sha256": hashlib.sha256(p.read_bytes()).hexdigest()}
             for p in sorted(runtime.rglob("*")) if p.is_file() and p.name != "gui-service-manifest.json"]
    manifest = {"schemaVersion": 1, "protocolVersion": 1, "serviceVersion": "0.2.30", "files": files}
    (runtime / "gui-service-manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf8")
    config = {"bundle": {"resources": {str(runtime).replace("\\", "/") + "/": "sidecars/gui-service/"}}}
    (ROOT / "target/tauri-gui-service.conf.json").write_text(json.dumps(config, indent=2), encoding="utf8")
    print(json.dumps({"ok": True, "files": len(files), "runtime": str(runtime)}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
