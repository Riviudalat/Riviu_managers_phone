"""Generate declarations into a disposable file and compare their content."""
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="riviu-ipc-") as directory:
    generated = Path(directory) / "ipc.ts"
    subprocess.run(["cargo", "run", "--locked", "-p", "riviu-core", "--example", "export_ipc", "--", str(generated)], cwd=root, check=True)
    checked_in = root / "apps/desktop/src/generated-ipc.ts"
    assert generated.read_text(encoding="utf-8") == checked_in.read_text(encoding="utf-8"), "Generated IPC types drifted"
print("Generated IPC declarations match")
