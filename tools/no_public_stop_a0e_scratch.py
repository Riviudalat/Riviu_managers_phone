"""Exact user-approved Publish scratch-only stop; no phone cleanup or replay."""
import json
import subprocess
from pathlib import Path
root=Path.cwd();a='a0e124bf-0700-4cd7-be17-0e7effb1089f';r='8a3efc0b-a09b-4e9e-a7d2-9dc023fe77b2'
s=json.loads((root/f'target/no-public-live/{a}.run/run-{r}/status.json').read_text())
if s['requestId']!=r or s['state']!='needsAttention' or s['outcome']['failureCode']!='account_unreadable' or s['outcome']['importId'] is not None:raise SystemExit('Reviewed Publish receipt changed')
exe=root/'target/debug/riviu-managers-phone.exe'
command=("$ErrorActionPreference='Stop'; $p=Get-Process -Id 36156; "
 f"if($p.Path -ne '{exe}' -or $p.StartTime.ToUniversalTime().Ticks.ToString() -ne '639264936693217475'){{throw 'incarnation changed'}}; "
 "Stop-Process -InputObject $p; $p.WaitForExit(15000); if(@(Get-Process|Where-Object {$_.Id -eq 36156}).Count -ne 0){throw 'exit not proven'}")
subprocess.run(['powershell.exe','-NoProfile','-Command',command],capture_output=True,text=True,check=True)
receipt={'activationId':a,'pid':36156,'explicitPublishScratchStopApproval':True,'terminated':True,'phoneCleanup':'unresolved','fencePreserved':True,'deviceCommandsSent':False,'noReplay':True}
with(root/f'target/no-public-live/{a}.process-only-stop.json').open('x')as f:json.dump(receipt,f,indent=2)
print(json.dumps(receipt))
