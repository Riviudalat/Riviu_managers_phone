"""Read process identities only; never closes or launches an application."""
import json
import subprocess
command = "$ErrorActionPreference='Stop'; $p=@(Get-Process | Where-Object { $_.ProcessName -eq 'riviu-managers-phone' }); @($p|ForEach-Object {@{pid=$_.Id;path=$_.Path;startTicks=$_.StartTime.ToUniversalTime().Ticks.ToString();hwnd=$_.MainWindowHandle.ToInt64()}})|ConvertTo-Json -Compress"
result = subprocess.run(['powershell.exe','-NoProfile','-Command',command],capture_output=True,text=True,check=True)
print(result.stdout.strip() or '[]')
