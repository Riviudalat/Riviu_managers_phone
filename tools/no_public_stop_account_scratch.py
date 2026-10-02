"""Exact user-approved process-only stop; no phone cleanup claim or commands."""
import json
import subprocess
from pathlib import Path

root = Path.cwd()
activation = 'ddea5d06-9991-48de-8557-a13e39195aea'
request = '50b74ffd-1ae1-4b68-bf43-9d5c2a75be21'
status = json.loads((root / f'target/no-public-live/{activation}.run/run-{request}/status.json').read_text())
if status['requestId'] != request or status['state'] != 'needsAttention' or status['phase'] != 'readingAccount' or status['outcome']['error'] != 'no-public prepare blocked (ComposerUnproved); cleanup=NotCreated':
    raise SystemExit('Reviewed receipt changed; no stop')
exe = root / 'target/debug/riviu-managers-phone.exe'
command = (
    "$ErrorActionPreference='Stop'; $p=Get-Process -Id 35488; "
    f"if($p.Path -ne '{exe}' -or $p.StartTime.ToUniversalTime().Ticks.ToString() -ne '639264891585141270'){{throw 'incarnation changed'}}; "
    "Stop-Process -InputObject $p; $p.WaitForExit(15000); "
    "if(@(Get-Process | Where-Object {$_.Id -eq 35488}).Count -ne 0){throw 'exit not proven'}"
)
subprocess.run(['powershell.exe', '-NoProfile', '-Command', command], capture_output=True, text=True, check=True)
receipt = {'activationId': activation, 'pid': 35488, 'startTicks': '639264891585141270', 'processOnlyStopApproved': True, 'terminated': True, 'phoneSessionCleanup': 'unresolved', 'streamCleanup': 'unresolved', 'fencePreserved': True, 'deviceCommandsSent': False}
with (root / f'target/no-public-live/{activation}.process-only-stop.json').open('x', encoding='utf8') as file:
    json.dump(receipt, file, indent=2)
print(json.dumps(receipt))
