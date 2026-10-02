"""Require the completed tracked native build before one approved discovery."""
import hashlib
import json
from pathlib import Path
import subprocess

root = Path.cwd()
log = Path('C:/Users/cattfan/AppData/Local/Temp/claude/C--Users-cattfan-Desktop-Riviu-managers-phone/6aed2316-7096-43f8-8c5b-537cb6e5a566/tasks/bym15z9l8.output')
expected = json.loads((root/'target/stability-empty-widget-source.json').read_text())
text = log.read_text(encoding='utf8', errors='replace')
if 'error:' in text or 'error[' in text or 'Finished `dev`' not in text:
    raise SystemExit('Native build not proved complete; no canary launch')
if any(hashlib.sha256((root/path).read_bytes()).hexdigest() != digest for path,digest in expected.items()):
    raise SystemExit('Frozen source changed; no canary')
subprocess.run(['python','tools/no_public_discovery_ce031cd.py'],check=True)
