"""Read-only prerequisites on the remaining exact roster; no candidate installation."""
import json
import subprocess
from pathlib import Path
from rehearsal_inventory import inventory

root = Path.cwd()
canary = 'ce021712aaf9533405'
apk = root / 'sidecars/riviu-android-agent/build/candidate/riviu-agent.apk'
digest = 'f30b11c86311b2940f08a1e2b7b2816a496b6238f16d3cb5685729cf9e29284f'
rows = inventory()
if len(rows) != 10 or any(row['state'] != 'device' for row in rows):
    raise SystemExit('Exact ten-device roster changed; no probe')
records = []
for row in rows:
    serial = row['serial']
    if serial == canary:
        records.append({'serial':serial,'state':'canaryNeedsAttention','probeSkipped':True})
        continue
    report = root / 'target/helper-live' / f'probe-{serial}-01'
    result = subprocess.run([str(root/'target/debug/examples/helper_canary.exe'), '--probe-helper-only', serial, str(apk), digest, str(report)], capture_output=True,text=True)
    records.append({'serial':serial,'state':'prerequisitesEligible' if result.returncode == 0 else 'prerequisitesBlocked','exitCode':result.returncode,'output':result.stdout.strip(),'error':result.stderr.strip(),'candidateInstalled':False})
path = root / 'target/helper-live/fleet-prerequisites.json'
with path.open('x',encoding='utf8') as file:json.dump(records,file,ensure_ascii=False,indent=2)
print(json.dumps(records,ensure_ascii=False,indent=2))
