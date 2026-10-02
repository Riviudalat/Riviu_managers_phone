"""One-off operator-approved termination of the already-windowless scratch activation."""
import json
import subprocess
from pathlib import Path

pid = 35984
activation = 'aa7c1c37-728f-4fdc-bc35-5dd60af7f9f0'
root = Path.cwd()
exe = root/'target/debug/riviu-managers-phone.exe'
command = f"$ErrorActionPreference='Stop'; $p=Get-Process -Id {pid}; @{{id=$p.Id;path=$p.Path;startTicks=$p.StartTime.ToUniversalTime().Ticks.ToString();hwnd=$p.MainWindowHandle.ToInt64()}}|ConvertTo-Json -Compress"
result = subprocess.run(['powershell.exe','-NoProfile','-Command',command],capture_output=True,text=True,check=True)
identity = json.loads(result.stdout)
if identity['id'] != pid or Path(identity['path']).resolve() != exe.resolve() or identity['startTicks'] != '639264612189896481' or identity['hwnd'] != 0:
    raise SystemExit('Approved scratch incarnation changed; no termination')
runroot = root/f'target/no-public-live/{activation}.run'
if list(runroot.glob('**/intent.json')):
    raise SystemExit('Device diagnostic intent exists; no termination')
# Identity rechecked in the same command immediately before stopping this process object.
stop = f"$ErrorActionPreference='Stop'; $p=Get-Process -Id {pid}; if($p.Path -ne '{str(exe)}' -or $p.StartTime.ToUniversalTime().Ticks.ToString() -ne '639264612189896481'){{throw 'incarnation changed'}}; Stop-Process -InputObject $p; $p.WaitForExit(15000); if(@(Get-Process|Where-Object {{$_.Id -eq {pid}}}).Count -ne 0){{throw 'exit not proven'}}"
subprocess.run(['powershell.exe','-NoProfile','-Command',stop],capture_output=True,text=True,check=True)
record = {'pid':pid,'activationId':activation,'operatorApproved':True,'scope':'windowless scratch process only','noDeviceRunIntents':True,'terminated':True}
(root/'target/no-public-live/scratch-termination.json').write_text(json.dumps(record,indent=2))
print(json.dumps(record))
