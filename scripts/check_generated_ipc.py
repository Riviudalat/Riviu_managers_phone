"""Generate declarations into a disposable file and compare exact bytes."""
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="riviu-ipc-") as directory:
    generated = Path(directory) / "ipc.ts"
    subprocess.run(["cargo", "run", "--locked", "-p", "riviu-core", "--example", "export_ipc", "--", str(generated)], cwd=root, check=True)
    assert generated.read_bytes() == (root / "apps/desktop/src/generated-ipc.ts").read_bytes(), "Generated IPC types drifted"
print("Generated IPC declarations match")
