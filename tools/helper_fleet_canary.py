"""Expand the exact approved fleet only after the first canary's clean receipt."""
import hashlib
import json
import subprocess
from pathlib import Path
from rehearsal_inventory import inventory

root = Path.cwd()
log = root/'target/helper-live'
first = 'ce021712aaf9533405'
receipt = json.loads((log/f'canary-{first}-03/receipt.json').read_text())
if receipt.get('serial') != first or not all(receipt.get(key) is True for key in ['bootstrapVerified','clipboardReadSucceeded','imeRestored','ownedReleaseVerified']) or receipt.get('publicEffectsAllowed') is not False:
    raise SystemExit('First canary is not clean; no fleet expansion')
apk = root/'sidecars/riviu-android-agent/build/candidate/riviu-agent.apk'
digest = '66635b39805ebe573bd32a51ee1c4a260affbb59da52d174911ab29f8cddb886'
if hashlib.sha256(apk.read_bytes()).hexdigest() != digest:raise SystemExit('Candidate changed')
approved = {row['serial'] for row in json.loads((log/'fleet-prerequisites.json').read_text())}
rows = inventory()
if {row['serial'] for row in rows} != approved or len(rows) != 10 or any(row['state'] != 'device' for row in rows):raise SystemExit('Approved roster changed')
records = [{'serial':first,'state':'preparedReadRestoredReleased','receipt':f'canary-{first}-03/receipt.json'}]
for row in rows:
    serial = row['serial']
    if serial == first:continue
    report = log/f'canary-{serial}-01'
    result = subprocess.run([str(root/'target/debug/examples/helper_canary.exe'),'--approved-helper-only',serial,str(apk),digest,str(report)],capture_output=True,text=True)
    record = {'serial':serial,'exitCode':result.returncode,'state':'passed' if result.returncode == 0 else 'needsAttention','output':result.stdout.strip(),'error':result.stderr.strip(),'report':str(report)}
    if (report/'receipt.json').is_file():record['receipt']=json.loads((report/'receipt.json').read_text())
    records.append(record)
    # Persist each completion; do not lose earlier devices if the controller exits.
    with (log/'fleet-canary-results.json').open('w',encoding='utf8') as file:json.dump(records,file,ensure_ascii=False,indent=2)
    print(json.dumps(record,ensure_ascii=False),flush=True)
    if result.returncode != 0:
        print('Fleet expansion paused at first failure; no replay',flush=True)
        break
