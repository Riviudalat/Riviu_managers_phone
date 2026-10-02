"""Read actual app tuples through the one scoped Tauri process, never device input."""
import argparse
import json
import subprocess
from pathlib import Path

p=argparse.ArgumentParser();p.add_argument('--activation',required=True);p.add_argument('--port',required=True);a=p.parse_args()
root=Path('target/no-public-live');report=root/f'{a.activation}-reports'
status=json.loads((report/'status-status.json').read_text())
if status['activationId']!=a.activation or status['publicEffectsAllowed'] is not False:raise SystemExit('Wrong activation')
rows=[]
for serial in status['scopeDeviceIds']:
    path=report/f'metadata-{serial}.json'
    if not path.exists():
        result=subprocess.run(['node','tools/no_public_harness.mjs','--port',a.port,'--mode','metadata','--udid',serial,'--activation',a.activation,'--report',str(report)],capture_output=True,text=True)
        if result.returncode:raise SystemExit(result.stderr)
    rows.append(json.loads(path.read_text()))
with (report/'fleet-metadata.json').open('x',encoding='utf8') as f:json.dump(rows,f,indent=2)
print(json.dumps(rows,indent=2))
