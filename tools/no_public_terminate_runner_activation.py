"""Exact operator-approved scope switch; retains the quarantined run evidence/fence."""
import json
import subprocess
from pathlib import Path
root=Path.cwd();pid=4644;activation='3cb58329-3d81-496e-b200-ea8946a8cc3a';ticks='639264701512844320';exe=root/'target/debug/riviu-managers-phone.exe'
status=json.loads((root/f'target/no-public-live/{activation}.run/run-a5b93c5b-bef0-4d8d-85b2-8812cd1052e7/status.json').read_text())
if status['state']!='needsAttention' or status['phase']!='warmForegroundIntent' or status['kind']!='inspect' or status['publicEffectsAllowed'] is not False:raise SystemExit('Run differs from approved scope switch')
command=f"$ErrorActionPreference='Stop'; $p=Get-Process -Id {pid}; if($p.Path -ne '{str(exe)}' -or $p.StartTime.ToUniversalTime().Ticks.ToString() -ne '{ticks}'){{throw 'incarnation changed'}}; Stop-Process -InputObject $p; $p.WaitForExit(15000); if(@(Get-Process|Where-Object {{$_.Id -eq {pid}}}).Count -ne 0){{throw 'exit not proven'}}"
subprocess.run(['powershell.exe','-NoProfile','-Command',command],check=True,capture_output=True,text=True)
record={'pid':pid,'activationId':activation,'operatorApproved':True,'terminated':True,'evidenceAndFencePreserved':True,'scope':'scratch process only; no ADB/phone process stop'}
(root/f'target/no-public-live/{activation}.termination.json').write_text(json.dumps(record,indent=2));print(json.dumps(record))
