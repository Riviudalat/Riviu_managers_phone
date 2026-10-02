"""Session-authorized stop of exact pre-input discovery refusal; retain phone fence."""
import json
import subprocess
import xml.etree.ElementTree as ET
from pathlib import Path
root = Path.cwd()
a = 'af500e2d-2f71-4488-a349-0f26ed4b3f39'
r = '1a943a7c-e34b-44e2-a7b4-8f1f2fc30d90'
run = root/f'target/no-public-live/{a}.run/run-{r}'
s = json.loads((run/'status.json').read_text())
if s['requestId'] != r or s['state'] != 'needsAttention' or s['outcome']['error'] != 'no-public prepare blocked (StaleDraft); cleanup=NotCreated':
    raise SystemExit('Reviewed refusal changed')
fields = [n for n in ET.parse(run/'discovery-verifier.xml').iter() if n.get('class') == 'android.widget.EditText']
if len(fields) != 2 or any(n.get('text') != '' or n.get('focused') != 'false' for n in fields):
    raise SystemExit('Exact pre-navigation empty-widget refusal changed')
exe = root/'target/debug/riviu-managers-phone.exe'
command = (
    "$ErrorActionPreference='Stop'; $p=Get-Process -Id 42696; "
    f"if($p.Path -ne '{exe}' -or $p.StartTime.ToUniversalTime().Ticks.ToString() -ne '639264923934447321'){{throw 'incarnation changed'}}; "
    "Stop-Process -InputObject $p; $p.WaitForExit(15000); if(@(Get-Process|Where-Object {$_.Id -eq 42696}).Count -ne 0){throw 'exit not proven'}"
)
subprocess.run(['powershell.exe','-NoProfile','-Command',command],capture_output=True,text=True,check=True)
receipt = {'activationId':a,'pid':42696,'operatorSessionDiscoveryOnlyApproval':True,'terminated':True,'phoneCleanup':'unresolved','fencePreserved':True,'deviceCommandsSent':False,'preInputSourceAndXmlRefusal':'empty unfocused EditText'}
with (root/f'target/no-public-live/{a}.process-only-stop.json').open('x') as f: json.dump(receipt,f,indent=2)
print(json.dumps(receipt))
