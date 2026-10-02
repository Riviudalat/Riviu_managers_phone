"""Session-approved pre-input discovery scratch stop, preserve phone fence."""
import json
import subprocess
import xml.etree.ElementTree as ET
from pathlib import Path
root = Path.cwd()
activation = 'd3d0558f-180e-4d8b-ac21-e4f622d1f690'
request = '035f47fa-0d21-4ae8-8114-5455b2c0844c'
run = root / f'target/no-public-live/{activation}.run/run-{request}'
status = json.loads((run/'status.json').read_text())
if status['requestId'] != request or status['state'] != 'needsAttention' or status['phase'] != 'readingAccount' or status['outcome']['error'] != 'no-public prepare blocked (AccountUnproved); cleanup=NotCreated':
    raise SystemExit('Reviewed receipt changed')
nodes = list(ET.parse(run/'discovery-verifier.xml').iter())
if len([n for n in nodes if n.get('resource-id') == 'com.zhiliaoapp.musically:id/wr2']) != 2 or len([n for n in nodes if n.get('content-desc') == 'Profile']) != 1:
    raise SystemExit('Exact pre-input duplicate-wr2 refusal changed')
exe = root/'target/debug/riviu-managers-phone.exe'
command = (
    "$ErrorActionPreference='Stop'; $p=Get-Process -Id 6200; "
    f"if($p.Path -ne '{exe}' -or $p.StartTime.ToUniversalTime().Ticks.ToString() -ne '639264915566358070'){{throw 'incarnation changed'}}; "
    "Stop-Process -InputObject $p; $p.WaitForExit(15000); if(@(Get-Process|Where-Object {$_.Id -eq 6200}).Count -ne 0){throw 'exit not proven'}"
)
subprocess.run(['powershell.exe','-NoProfile','-Command',command],capture_output=True,text=True,check=True)
receipt = {'activationId':activation,'pid':6200,'operatorSessionDiscoveryOnlyApproval':True,'terminated':True,'phoneCleanup':'unresolved','fencePreserved':True,'deviceCommandsSent':False,'preInputSourceAndXmlRefusal':'duplicate global wr2'}
with (root/f'target/no-public-live/{activation}.process-only-stop.json').open('x') as file: json.dump(receipt,file,indent=2)
print(json.dumps(receipt))
