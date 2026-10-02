"""Exact approved pre-input capture refusal stop; no phone cleanup claim."""
import json
import subprocess
from pathlib import Path
root=Path.cwd(); a='582c8ebd-7df4-45b0-9791-955a88fc91b9'; r='d0a0565d-dc33-4f72-aff8-6ca3ca4b7ba0'
s=json.loads((root/f'target/no-public-live/{a}.run/run-{r}/status.json').read_text())
if s['requestId']!=r or s['state']!='needsAttention' or s['phase']!='readingAccount' or s['outcome']['error']!='baseline hierarchy unavailable within diagnostic budget': raise SystemExit('Reviewed pre-verifier receipt changed')
exe=root/'target/debug/riviu-managers-phone.exe'
command=("$ErrorActionPreference='Stop'; $p=Get-Process -Id 27512; "
 f"if($p.Path -ne '{exe}' -or $p.StartTime.ToUniversalTime().Ticks.ToString() -ne '639264932313681935'){{throw 'incarnation changed'}}; "
 "Stop-Process -InputObject $p; $p.WaitForExit(15000); if(@(Get-Process|Where-Object {$_.Id -eq 27512}).Count -ne 0){throw 'exit not proven'}")
subprocess.run(['powershell.exe','-NoProfile','-Command',command],capture_output=True,text=True,check=True)
receipt={'activationId':a,'pid':27512,'operatorSessionDiscoveryOnlyApproval':True,'terminated':True,'phoneCleanup':'unresolved','fencePreserved':True,'deviceCommandsSent':False,'preInputRefusal':'supporting hierarchy capture budget before verifier'}
with (root/f'target/no-public-live/{a}.process-only-stop.json').open('x') as f:json.dump(receipt,f,indent=2)
print(json.dumps(receipt))
