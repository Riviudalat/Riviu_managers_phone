"""Launch exactly one compiled debug AppState in a new no-public diagnostic root."""
import argparse
import json
from pathlib import Path
import subprocess
import os
import uuid
import hashlib

p=argparse.ArgumentParser()
p.add_argument('--exe',required=True)
p.add_argument('--serial',required=True,help='Explicit serial or comma-separated approved serials')
p.add_argument('--package',default='com.ss.android.ugc.trill')
p.add_argument('--port',type=int,default=9377)
p.add_argument('--allow-installed-runner',action='store_true',help='Approved start of installed UiAutomator on exact scoped devices; no install/restart')
p.add_argument('--allow-warm-launch',action='store_true',help='Approved exact-package foreground launch, no stop/unlock')
p.add_argument('--prepared-scope',help='Existing approved preparation scope; immutable activation/input snapshot')
a=p.parse_args()
exe=Path(a.exe).resolve(strict=True)
result=subprocess.run(['powershell.exe','-NoProfile','-Command',"$ErrorActionPreference='Stop'; $p=@(Get-Process | Where-Object { $_.ProcessName -eq 'riviu-managers-phone' }); @{querySucceeded=$true;ids=@($p|ForEach-Object {$_.Id})}|ConvertTo-Json -Compress"],capture_output=True,text=True,check=True)
processes=json.loads(result.stdout)
if processes.get('querySucceeded') is not True or not isinstance(processes.get('ids'),list):raise SystemExit('Process inventory not proven; no launch')
if processes['ids']:raise SystemExit('Existing app still running; no second controller')
root=Path('target/no-public-live').resolve();root.mkdir(parents=True,exist_ok=True)
activation=str(uuid.uuid4());scope_path=root/f'{activation}.scope.json';data_root=root/f'{activation}.run'
serials=a.serial.split(',')
if not serials or len(set(serials))!=len(serials) or any(not s or any(c.isspace() for c in s) for s in serials):raise SystemExit('Explicit unique serials required')
scope={'activationId':activation,'deviceScopes':[{'udid':s,'package':a.package,'expectedAccount':'','targetUrl':None,'draftText':None,'sourceRoot':None,'bundleId':None,'allowWarmLaunch':a.allow_warm_launch,'allowInstalledRunner':a.allow_installed_runner} for s in serials]}
if a.prepared_scope:
    supplied=Path(a.prepared_scope).resolve(strict=True)
    scope=json.loads(supplied.read_text(encoding='utf8'))
    if set(d['udid'] for d in scope['deviceScopes'])!=set(serials) or any(d['package']!=a.package or not d['expectedAccount'] for d in scope['deviceScopes']):raise SystemExit('Prepared scope differs from approved serial/package/account')
    activation=str(uuid.UUID(scope['activationId']));scope_path=root/f'{activation}.scope.json';data_root=root/f'{activation}.run'
if scope_path.exists() or data_root.exists():raise SystemExit('Activation already exists; no reuse')
scope_path.write_text(json.dumps(scope,indent=2),encoding='utf8')
env=os.environ.copy()
for key in ['RIVIU_MOCK_DEVICES','RIVIU_MOCK_DATA_DIR','RIVIU_UI_SMOKE','RIVIU_UI_SMOKE_DIR','WEBVIEW2_USER_DATA_FOLDER','WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS','WEBVIEW2_BROWSER_EXECUTABLE_FOLDER','WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER']:
    if key in env:raise SystemExit(f'Clear inherited {key}; no silent override')
if env.get('RIVIU_ANDROID_OBSERVATION_MODE', 'legacy') != 'legacy':raise SystemExit('Discovery requires legacy observation mode; no silent override')
env.update(RIVIU_ANDROID_OBSERVATION_MODE='legacy',RIVIU_NO_PUBLIC_REHEARSAL='1',RIVIU_NO_PUBLIC_SCOPE=str(scope_path),RIVIU_NO_PUBLIC_DIR=str(data_root),RIVIU_DEV_BACKGROUND='1',RIVIU_DEV_CDP_PORT=str(a.port))
log=root/f'{activation}.stdout.log';error=root/f'{activation}.stderr.log'
with log.open('xb') as stdout,error.open('xb') as stderr:
    proc=subprocess.Popen([str(exe)],env=env,stdout=stdout,stderr=stderr,creationflags=0x08000000)
record={'activationId':activation,'pid':proc.pid,'exe':str(exe),'exeSha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'scope':str(scope_path),'dataRoot':str(data_root),'cdpPort':a.port,'publicEffectsAllowed':False}
(root/f'{activation}.process.json').write_text(json.dumps(record,indent=2),encoding='utf8')
print(json.dumps(record,indent=2))
