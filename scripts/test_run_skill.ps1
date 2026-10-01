# Offline regression: imports only pure policy; parses driver without executing it.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$skill = Join-Path $root '.claude/skills/run-riviu-managers-phone'
. (Join-Path $skill 'process-policy.ps1')
function Assert($value, [string]$message) { if (-not $value) { throw $message } }
function Reject([scriptblock]$action) {
    $rejected = $false
    try { & $action | Out-Null } catch { $rejected = $true }
    Assert $rejected 'Expected fail-closed rejection'
}
function Item([int]$id, [int]$parent, [string]$name, [string]$created, [string]$path) {
    [pscustomobject]@{ Id=$id; ParentId=$parent; Name=$name; Created=$created; ExecutablePath=$path }
}
$launcher = Item 10 1 'cmd.exe' '1000' 'C:\Windows\cmd.exe'
$app = Item 12 11 'riviu-managers-phone.exe' '1200' 'C:\repo\target\debug\riviu-managers-phone.exe'
$node = Item 11 10 'node.exe' '1100' 'C:\node.exe'
$record = [pscustomobject]@{Schema=1;RepoRoot='C:\repo';Launcher=$launcher;App=$null;ExpectedPaths=@($app.ExecutablePath)}
$inventory=@($launcher,$node,$app)
Assert ((Select-RiviuProcess $record $inventory 'C:\repo').Id -eq 12) 'Owned descendant was not selected'
$foreign=Item 20 1 'riviu-managers-phone.exe' '1300' $app.ExecutablePath
Assert ((Select-RiviuProcess $record ($inventory+@($foreign)) 'C:\repo').Id -eq 12) 'Foreign process selected'
$other=Item 13 11 'riviu-managers-phone.exe' '1300' $app.ExecutablePath
Reject { Select-RiviuProcess $record ($inventory+@($other)) 'C:\repo' }
Reject { Select-RiviuProcess $record $inventory 'C:\other' }
Reject { Select-RiviuProcess $record @($node,$app) 'C:\repo' }
$record.App=$app
Reject { Select-RiviuProcess $record @((Item 12 11 'riviu-managers-phone.exe' '1400' $app.ExecutablePath)) 'C:\repo' }
Reject { Select-RiviuProcess $record @((Item 12 11 'riviu-managers-phone.exe' '1200' '')) 'C:\repo' }
Assert ($null -eq (Select-RiviuProcess $record @($launcher,$node) 'C:\repo')) 'Exited process should be absent'
Assert ((Get-RiviuLaunchMode @()) -eq 'smoke') 'Launch must default to isolation'
Assert ((Get-RiviuLaunchMode @('--mock')) -eq 'smoke') 'Mock must use real isolation'
Assert ((Get-RiviuLaunchMode @('--live-confirmed')) -eq 'live') 'Explicit live mode missing'
Reject { Get-RiviuLaunchMode @('--mock','--live-confirmed') }

$errors=$null; $tokens=$null
$ast=[System.Management.Automation.Language.Parser]::ParseFile((Join-Path $skill 'driver.ps1'),[ref]$tokens,[ref]$errors)
Assert ($errors.Count -eq 0) 'Driver parse failed'
$stop=$ast.Find({param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Invoke-Stop'},$true)
# Execute the actual stop function with fake OS boundaries, not a source-only assertion.
Invoke-Expression $stop.Extent.Text
$script:post=0; $script:kills=0; $script:lookups=0
Add-Type 'public class RiviuWin32 { public static int WM_CLOSE=16; public static bool CaptureResult=false; public static bool PostMessage(System.IntPtr h,int m,System.IntPtr w,System.IntPtr l){ return true; } public static bool PrintWindow(System.IntPtr h,System.IntPtr d,uint flags){ return CaptureResult; } }'
function Write-Step($message) { }
function Start-Sleep { param($Seconds,$Milliseconds) }
function Stop-Process { $script:kills++; throw 'Unexpected kill' }
function Get-AppProcessAny { $script:lookups++; [pscustomobject]@{Id=12} }
function Get-AppWindow { [pscustomobject]@{Handle=[IntPtr]1;Process=[pscustomobject]@{Id=12}} }
function Assert-OwnedWindow($window) { }
Reject { Invoke-Stop }
Assert ($script:kills -eq 0) 'Shutdown timeout killed a process'
Assert ($script:lookups -ge 30) 'Did not wait for graceful drain'
function Get-AppProcessAny { $null }
Invoke-Stop
Assert ($script:kills -eq 0) 'Absent app must not reap launcher'

# Scout rejects missing scope before resolving or invoking ADB.
$failed=$false
try { & (Join-Path $skill 'hunt_badge_4642.ps1') } catch { $failed=$true }
Assert $failed 'Scout must reject missing serial/permission'
# Actual coordinate guard with a fake Win32 recipient, no OS input.
# The type used above is deliberately minimal; the policy rejection before hit-test
# can still prove out-of-window inputs do not reach global input APIs.
$pointGuard=$ast.Find({param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Assert-InputPoint'},$true)
Invoke-Expression $pointGuard.Extent.Text
$window=[pscustomobject]@{Left=100;Top=100;Width=500;Height=400;Handle=[IntPtr]1}
Reject { Assert-InputPoint $window -1 150 }
Reject { Assert-InputPoint $window 600 150 }
Reject { Assert-InputPoint $window 150 500 }
# Ensure retired unsafe paths cannot silently dispatch again.
foreach($name in @('Invoke-Drag','Invoke-CtrlScroll')) {
    $fn=$ast.Find({param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name},$true)
    Invoke-Expression $fn.Extent.Text
    Reject { & $name }
}
Add-Type -AssemblyName System.Drawing
foreach($name in @('Test-BitmapBlank','Save-WindowPng')) {
    $fn=$ast.Find({param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name},$true)
    Invoke-Expression $fn.Extent.Text
}
$temp=Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString('N')+'.png')
try {
    [RiviuWin32]::CaptureResult=$false
    Reject { Save-WindowPng $window $temp }
    Assert (-not (Test-Path $temp)) 'Failed HWND capture saved a screenshot'
    [RiviuWin32]::CaptureResult=$true
    Reject { Save-WindowPng $window $temp }
    Assert (-not (Test-Path $temp)) 'Blank HWND capture saved a screenshot'
    function Assert-OwnedWindow($window) { throw 'identity changed' }
    Reject { Save-WindowPng $window $temp }
    Assert (-not (Test-Path $temp)) 'Stale ownership saved a screenshot'
} finally { if(Test-Path $temp){Remove-Item -LiteralPath $temp} }
# Exercise production fill dispatch with fake input boundaries; exactly one send.
$fn=$ast.Find({param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq 'Invoke-Fill'},$true)
Invoke-Expression $fn.Extent.Text
$script:sent=0; $script:clicked=0
function Use-RaisedWindow { param($SettleMs,[switch]$Activate,[scriptblock]$Body) & $Body ([pscustomobject]@{Left=100;Top=100}) }
function Invoke-RawClick { param($ScreenX,$ScreenY) $script:clicked++ }
function Send-OwnedKeys { param($Window,$Sequence) $script:sent++; $script:lastSequence=$Sequence }
$Rest=@('20','30','fixture-text')
Invoke-Fill
Assert ($script:sent -eq 1 -and $script:clicked -eq 1) 'Fill must dispatch once without visual retry'
$cases=@(
    @('{ENTER}','{{}ENTER{}}'), @('hello~','hello{~}'), @('^a','{^}a'),
    @('C++','C{+}{+}'), @('%','{%}'), @('(x)[y]','{(}x{)}{[}y{]}')
)
foreach($case in $cases) {
    Assert ((ConvertTo-RiviuLiteralKeys $case[0]) -ceq $case[1]) 'Literal SendKeys escaping changed'
    $Rest=@('20','30',$case[0]); $script:sent=0
    Invoke-Fill
    Assert ($script:sent -eq 1 -and $script:lastSequence -ceq $case[1]) 'Fill did not send escaped literal once'
}
foreach($name in @('Invoke-Type','Invoke-Key')) {
    $fn=$ast.Find({param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $name},$true)
    Invoke-Expression $fn.Extent.Text
}
function Send-AppKeys { param($Sequence) $script:lastSequence=$Sequence }
$Rest=@('{ENTER}'); Invoke-Type
Assert ($script:lastSequence -ceq '{{}ENTER{}}') 'Type interpreted a literal Enter as a key'
Invoke-Key
Assert ($script:lastSequence -ceq '{ENTER}') 'Explicit key command must preserve intentional grammar'
Reject { ConvertTo-RiviuLiteralKeys "line`nnext" }
Reject { ConvertTo-RiviuLiteralKeys "field`tnext" }
Write-Output 'run-skill offline policy, actual graceful stop, and scout refusal: PASS'
