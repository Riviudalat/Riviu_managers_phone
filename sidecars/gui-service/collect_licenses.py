"""Copy license files from the locked Python distribution into the sidecar."""

import importlib.metadata
import json
import shutil
import sys
from pathlib import Path

out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)
records = []
for distribution in importlib.metadata.distributions():
    name = distribution.metadata.get("Name", "unknown")
    for file in distribution.files or []:
        if any(part.lower().startswith(("license", "copying", "notice")) for part in file.parts):
            source = Path(distribution.locate_file(file))
            if source.is_file():
                folder = out / name
                folder.mkdir(exist_ok=True)
                shutil.copyfile(source, folder / source.name)
    records.append(
        {
            "name": name,
            "version": distribution.version,
            "license": distribution.metadata.get("License-Expression")
            or distribution.metadata.get("License", ""),
        }
    )
python_license = Path(sys.base_prefix) / "LICENSE.txt"
if python_license.exists():
    shutil.copyfile(python_license, out / "CPython-LICENSE.txt")
(out / "packages.json").write_text(json.dumps(records, indent=2), encoding="utf8")
