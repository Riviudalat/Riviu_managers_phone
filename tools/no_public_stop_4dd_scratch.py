"""Session-approved exact pre-import Publish scratch stop; fences remain."""
import json
import subprocess
from pathlib import Path
root=Path.cwd();a='4ddfb41e-8e13-4ace-b523-d60f7ae94351';r='bb348793-f506-4b7e-b75c-d2dc66483543'
s=json.loads((root/f'target/no-public-live/{a}.run/run-{r}/status.json').read_text())
if s['requestId']!=r or s['state']!='needsAttention' or s['outcome']['importId'] is not None or s['outcome']['mediaCleanup']!='notStarted' or s['outcome']['preflightDetail']!='tap_target_missing_or_overlapped':raise SystemExit('Reviewed pre-import receipt changed')
exe=root/'target/debug/riviu-managers-phone.exe'
command=("$ErrorActionPreference='Stop'; $p=Get-Process -Id 48644; "
 f"if($p.Path -ne '{exe}' -or $p.StartTime.ToUniversalTime().Ticks.ToString() -ne '639264946509447353'){{throw 'incarnation changed'}}; "
 "Stop-Process -InputObject $p; $p.WaitForExit(15000); if(@(Get-Process|Where-Object {$_.Id -eq 48644}).Count -ne 0){throw 'exit not proven'}")
subprocess.run(['powershell.exe','-NoProfile','-Command',command],capture_output=True,text=True,check=True)
receipt={'activationId':a,'pid':48644,'operatorSessionPreImportPublishApproval':True,'terminated':True,'phoneCleanup':'unresolved','fencePreserved':True,'deviceCommandsSent':False,'noReplay':True}
with(root/f'target/no-public-live/{a}.process-only-stop.json').open('x')as f:json.dump(receipt,f,indent=2)
print(json.dumps(receipt))
