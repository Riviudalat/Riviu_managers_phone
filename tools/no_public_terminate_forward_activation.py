"""Session-authorized termination of exact setup-failed scratch; preserves its evidence."""
import json
import subprocess
from pathlib import Path
root=Path.cwd();pid=13940;ticks='639264882742657449';activation='41f4e58c-8870-481b-9b09-339eeb1ca155';request='fd6f1d78-1b98-497d-9f16-1acb05087cac';exe=root/'target/debug/riviu-managers-phone.exe'
status=json.loads((root/f'target/no-public-live/{activation}.run/run-{request}/status.json').read_text())
if status['requestId']!=request or status['phase']!='warmForegroundIntent' or status['state']!='needsAttention' or status['publicEffectsAllowed'] is not False:raise SystemExit('Reviewed setup failure changed')
command=f"$ErrorActionPreference='Stop'; $p=Get-Process -Id {pid}; if($p.Path -ne '{str(exe)}' -or $p.StartTime.ToUniversalTime().Ticks.ToString() -ne '{ticks}'){{throw 'incarnation changed'}}; Stop-Process -InputObject $p; $p.WaitForExit(15000); if(@(Get-Process|Where-Object {{$_.Id -eq {pid}}}).Count -ne 0){{throw 'exit not proven'}}"
subprocess.run(['powershell.exe','-NoProfile','-Command',command],check=True,capture_output=True,text=True)
record={'pid':pid,'activationId':activation,'operatorSessionApproval':True,'setupFailedBeforeSessionInput':True,'terminated':True,'fenceAndEvidencePreserved':True}
(root/f'target/no-public-live/{activation}.termination.json').write_text(json.dumps(record,indent=2));print(json.dumps(record))
