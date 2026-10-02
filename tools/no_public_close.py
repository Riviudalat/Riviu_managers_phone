"""Graceful-close one previously identified idle process; never terminate."""
import argparse
import ctypes
import json
import subprocess
import time

parser=argparse.ArgumentParser()
parser.add_argument('--pid',type=int,required=True)
parser.add_argument('--expected-path',required=True)
parser.add_argument('--expected-start-ticks',required=True)
parser.add_argument('--operator-confirmed-idle',action='store_true')
a=parser.parse_args()
if not a.operator_confirmed_idle:raise SystemExit('No idle authorization')
command=f"$ErrorActionPreference='Stop'; $p=@(Get-Process | Where-Object {{ $_.Id -eq {a.pid} }}); if($p.Count -eq 0){{@{{state='absent'}}|ConvertTo-Json -Compress}}elseif($p.Count -eq 1){{@{{state='present';id=$p[0].Id;path=$p[0].Path;startTicks=$p[0].StartTime.ToUniversalTime().Ticks.ToString();hwnd=$p[0].MainWindowHandle.ToInt64()}}|ConvertTo-Json -Compress}}else{{throw 'ambiguous process'}}"
def process():
    result=subprocess.run(['powershell.exe','-NoProfile','-Command',command],capture_output=True,text=True,check=True)
    record=json.loads(result.stdout)
    if record.get('state') not in ('absent','present'):raise RuntimeError('Process inventory invalid')
    if record['state']=='present' and (not record.get('path') or not record.get('startTicks')):raise RuntimeError('Process identity unreadable')
    return record
record=process()
if record['state']!='present' or record['path'].lower()!=a.expected_path.lower() or record['startTicks']!=a.expected_start_ticks:raise SystemExit('Process incarnation differs; no close')
handle=record['hwnd']
user32=ctypes.windll.user32
user32.GetWindowThreadProcessId.argtypes=[ctypes.c_void_p,ctypes.POINTER(ctypes.c_ulong)]
if not handle:
    # Background diagnostic windows are hidden, so MainWindowHandle may be zero.
    # Match exactly one owned product window; never close other WebView/helper HWNDs.
    handles=[]
    callback_type=ctypes.WINFUNCTYPE(ctypes.c_bool,ctypes.c_void_p,ctypes.c_void_p)
    user32.GetWindowTextW.argtypes=[ctypes.c_void_p,ctypes.c_wchar_p,ctypes.c_int]
    def collect(hwnd,_):
        pid=ctypes.c_ulong();user32.GetWindowThreadProcessId(hwnd,ctypes.byref(pid))
        if pid.value==a.pid:
            title=ctypes.create_unicode_buffer(256);user32.GetWindowTextW(hwnd,title,256)
            if title.value=='Riviu Manager':handles.append(hwnd)
        return True
    user32.EnumWindows(callback_type(collect),None)
    if len(handles)!=1:raise SystemExit('No unique owned product window; no close')
    handle=handles[0]
if process()!=record:raise SystemExit('Identity changed; no close')
owner=ctypes.c_ulong();user32.GetWindowThreadProcessId(handle,ctypes.byref(owner))
if owner.value!=a.pid:raise SystemExit('Window owner differs; no close')
user32.PostMessageW.argtypes=[ctypes.c_void_p,ctypes.c_uint,ctypes.c_void_p,ctypes.c_void_p]
if not user32.PostMessageW(handle,0x0010,None,None):raise SystemExit('WM_CLOSE failed')
for _ in range(90):
    current=process()
    if current['state']=='absent':
        print(json.dumps({'pid':a.pid,'gracefulExit':True,'forceKill':False}));break
    if current['path']!=record['path'] or current['startTicks']!=record['startTicks']:raise SystemExit('PID reused; do not act on replacement')
    time.sleep(1)
else:raise SystemExit('Shutdown still pending; no force kill or second controller')
