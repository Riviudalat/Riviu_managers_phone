"""Authorized exact discovery-only scratch stop; phone cleanup remains unresolved."""
import json
import subprocess
import xml.etree.ElementTree as ET
from pathlib import Path

root = Path.cwd()
activation = 'c2929516-ce24-4d55-849d-fdf095d10320'
request = 'db2f2cd2-0d8d-41ae-af64-ae77f7a12911'
run = root / f'target/no-public-live/{activation}.run/run-{request}'
status = json.loads((run / 'status.json').read_text())
if status['requestId'] != request or status['state'] != 'needsAttention' or status['phase'] != 'readingAccount' or status['outcome']['error'] != 'no-public prepare blocked (AccountUnproved); cleanup=NotCreated':
    raise SystemExit('Reviewed receipt changed')
# This exact binary refuses Profile before dispatch when the measured background
# overlaps. Persisted input proves that branch; no inferred success after a tap.
tree = ET.parse(run / 'discovery-verifier.xml')
profile = [n for n in tree.iter() if n.get('content-desc') == 'Profile']
background = [n for n in tree.iter() if n.get('resource-id') == 'com.zhiliaoapp.musically:id/hpk']
if len(profile) != 1 or len(background) != 1 or background[0].get('clickable') != 'true' or background[0].get('bounds') != '[0,0][1080,2094]' or profile[0].get('bounds') != '[864,1965][1080,2094]':
    raise SystemExit('Pre-tap refusal evidence changed')
exe = root / 'target/debug/riviu-managers-phone.exe'
command = (
    "$ErrorActionPreference='Stop'; $p=Get-Process -Id 16004; "
    f"if($p.Path -ne '{exe}' -or $p.StartTime.ToUniversalTime().Ticks.ToString() -ne '639264908982105448'){{throw 'incarnation changed'}}; "
    "Stop-Process -InputObject $p; $p.WaitForExit(15000); "
    "if(@(Get-Process | Where-Object {$_.Id -eq 16004}).Count -ne 0){throw 'exit not proven'}"
)
subprocess.run(['powershell.exe', '-NoProfile', '-Command', command], capture_output=True, text=True, check=True)
receipt = {'activationId': activation, 'pid': 16004, 'operatorSessionDiscoveryOnlyApproval': True, 'terminated': True, 'zeroUiInputSourceAndXmlProof': True, 'phoneSessionCleanup': 'unresolved', 'streamCleanup': 'unresolved', 'fencePreserved': True, 'deviceCommandsSent': False}
with (root / f'target/no-public-live/{activation}.process-only-stop.json').open('x', encoding='utf8') as file:
    json.dump(receipt, file, indent=2)
print(json.dumps(receipt))
