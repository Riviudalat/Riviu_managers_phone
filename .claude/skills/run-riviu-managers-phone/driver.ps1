#Requires -Version 5.1
<#
    Driver for the Riviu Manager Tauri 2 desktop app (Windows).

    Debug background mode offers loopback CDP; prefer it for isolated renderer tests.
    This optional native driver has no Playwright _electron handle. It can raise the
    window by z-order, capture the screen rectangle, and inject real mouse /
    keyboard input. SetForegroundWindow is refused for a non-foreground caller,
    which is why every visual command goes through SetWindowPos(HWND_TOPMOST).

    Usage:
      powershell -NoProfile -ExecutionPolicy Bypass `
        -File .claude/skills/run-riviu-managers-phone/driver.ps1 <command> [args]

    Commands:
      launch [--smoke|--live-confirmed]  isolated UI smoke by default; live requires authorization
      wait [seconds]       block until the window reports Responding (default 300)
      status               processes, ports, usbmux, sidecars, python/cargo resolution
      shot <name>          PNG of the app window -> target/run-skill/<name>.png
      click <x> <y>        left click at window-relative coords
      fill <x> <y> <text>  click that point and type into it, in one process
      type <text>          SendKeys into the app (refuses unless the app is foreground)
      key <keys>           SendKeys sequence, e.g. "{ENTER}" or "^a"
      log [lines]          tail the tauri dev log (default 40)
      devices              run the pymobiledevice3 sidecar `list` under a hard timeout
      usbmux               start Apple's usbmux provider and report port 27015
      stop                 WM_CLOSE only the recorded owned app; never force-kill

    Screenshots and the dev log land in target/run-skill/ (target/ is gitignored).

    Coordinates are window-relative and assume 100% display scaling; `status`
    prints the window rect so a mismatch against 1456x939 is visible.
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Command,

    [Parameter(Position = 1, ValueFromRemainingArguments = $true)]
    [string[]]$Rest
)

$ErrorActionPreference = 'Stop'

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$AppDir   = Join-Path $RepoRoot 'apps\desktop'
$OutDir   = Join-Path $RepoRoot 'target\run-skill'
$DevLog   = Join-Path $OutDir 'tauri-dev.log'
$OwnerFile = Join-Path $OutDir 'owner.json'
. (Join-Path $PSScriptRoot 'process-policy.ps1')
$ProcName = 'riviu-managers-phone'
$AppTitle = 'Riviu Manager'

if (-not (Test-Path $OutDir)) { New-Item -ItemType Directory -Force -Path $OutDir | Out-Null }

# ---------------------------------------------------------------- Win32 interop

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
if (-not ('RiviuWin32' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public class RiviuWin32 {
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, IntPtr extra);
    // For Ctrl+wheel, the app's zoom gesture. SendKeys cannot hold a modifier *across* a
    // separate mouse event, so the key has to be pressed and released explicitly.
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
    public const byte VK_CONTROL = 0x11;
    public const uint KEYEVENTF_KEYUP = 0x0002;
    [DllImport("user32.dll")] public static extern uint GetDoubleClickTime();
    public const uint MOUSEEVENTF_MOVE = 0x0001;
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll", CharSet = CharSet.Auto)] public static extern IntPtr PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
    // Save-WindowPng's occluded path calls this, and it was being called without ever
    // being declared — so a `shot` that hit a covering window died on
    // "[RiviuWin32] does not contain a method named 'PrintWindow'" instead of falling back.
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint flags);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc cb, IntPtr lParam);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    delegate bool EnumWindowsProc(IntPtr h, IntPtr lParam);

    // Process.MainWindowHandle is "the first top-level window found", which for a
    // Tauri/WebView2 process can be an invisible 16x16 helper - acting on that
    // samples the top-left of the SCREEN instead of the app.
    //
    // Identify by WINDOW CLASS, not by size: tao/Tauri names the real window
    // 'Tauri Window', and a MINIMISED window reports rect -32000,-32000 160x28, so any
    // area threshold silently loses the app the moment someone minimises it.
    public static IntPtr FindAppWindow(uint pid, string titleContains) {
        IntPtr best = IntPtr.Zero;
        long bestScore = 0;
        EnumWindows(delegate(IntPtr h, IntPtr l) {
            uint wpid;
            GetWindowThreadProcessId(h, out wpid);
            if (wpid != pid) return true;
            StringBuilder cls = new StringBuilder(256);
            GetClassName(h, cls, 256);
            string c = cls.ToString();
            if (c == "Tao Thread Event Target" || c == "MSCTFIME UI" || c == "IME") return true;
            StringBuilder sb = new StringBuilder(512);
            GetWindowText(h, sb, 512);
            RECT r;
            GetWindowRect(h, out r);
            long score = (long)(r.Right - r.Left) * (r.Bottom - r.Top);
            if (c == "Tauri Window") { score += 1000000000L; }
            if (sb.ToString().Contains(titleContains)) { score += 100000000L; }
            if (IsWindowVisible(h)) { score += 10000000L; }
            if (score > bestScore) { bestScore = score; best = h; }
            return true;
        }, IntPtr.Zero);
        return best;
    }

    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }

    public const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
    public const uint MOUSEEVENTF_LEFTUP   = 0x0004;
    public const uint MOUSEEVENTF_RIGHTDOWN = 0x0008;
    public const uint MOUSEEVENTF_RIGHTUP   = 0x0010;
    public const uint WM_CLOSE             = 0x0010;
    public const int  SW_RESTORE           = 9;
    // SWP_NOSIZE | SWP_NOMOVE | SWP_SHOWWINDOW
    public const uint SWP_RAISE            = 0x0043;
    public const uint GA_ROOT               = 2;
}
'@
}

$HWND_TOPMOST   = [IntPtr]-1
$HWND_NOTOPMOST = [IntPtr]-2

# ---------------------------------------------------------------------- helpers

function Write-Step([string]$Message) { Write-Host "[driver] $Message" }

function Resolve-Python312 {
    # find_python() in crates/ios-driver/src/pmd.rs tries "python3" BEFORE "python"
    # and never checks the version. On a box where `python` is 3.14, the 3.12
    # directory must therefore win for the name python3.
    foreach ($candidate in @(
            "$env:LOCALAPPDATA\Programs\Python\Python312",
            'C:\Program Files\Python312',
            'C:\Python312')) {
        if (Test-Path (Join-Path $candidate 'python.exe')) { return $candidate }
    }
    return $null
}

function Resolve-Adb {
    # Same precedence the app uses (AGENTS.md 10): RIVIU_ADB_PATH points at the
    # executable itself, then the SDK roots, then PATH. The extra LOCALAPPDATA probe
    # covers a bare platform-tools unzip, which is not an SDK layout.
    if ($env:RIVIU_ADB_PATH -and (Test-Path $env:RIVIU_ADB_PATH)) { return $env:RIVIU_ADB_PATH }
    foreach ($root in @($env:ANDROID_SDK_ROOT, $env:ANDROID_HOME)) {
        if ($root) {
            $candidate = Join-Path $root 'platform-tools\adb.exe'
            if (Test-Path $candidate) { return $candidate }
        }
    }
    $onPath = Get-Command adb -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    $fallback = "$env:LOCALAPPDATA\Android\platform-tools\adb.exe"
    if (Test-Path $fallback) { return $fallback }
    return $null
}

function Get-DriverPath {
    $parts = @()
    $py = Resolve-Python312
    if ($py) { $parts += $py; $parts += (Join-Path $py 'Scripts') }
    $cargo = Join-Path $env:USERPROFILE '.cargo\bin'
    if (Test-Path $cargo) { $parts += $cargo }
    # detect_driver() shells out to `adb version`; without this the Android backend
    # sits out the fleet with android_unavailable_reason set.
    $adb = Resolve-Adb
    if ($adb) { $parts += (Split-Path $adb) }
    return (($parts + $env:PATH) -join ';')
}

function Get-ProcessInventory {
    @(Get-CimInstance Win32_Process -ErrorAction Stop | ForEach-Object {
        [pscustomobject]@{
            Id = [int]$_.ProcessId; ParentId = [int]$_.ParentProcessId; Name = $_.Name
            Created = if ($_.CreationDate) { ([long]($_.CreationDate.ToUniversalTime().Ticks / 10000)).ToString() } else { '' }
            ExecutablePath = $_.ExecutablePath
        }
    })
}

function Save-Owner($Record) {
    $temporary = "$OwnerFile.$([guid]::NewGuid().ToString('N')).tmp"
    $Record | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $temporary -Encoding UTF8
    Move-Item -LiteralPath $temporary -Destination $OwnerFile -Force
}

function Get-AppProcessAny {
    if (-not (Test-Path -LiteralPath $OwnerFile)) {
        throw 'No owned application record. Use launch --smoke; do not attach by process name.'
    }
    $record = Get-Content -LiteralPath $OwnerFile -Raw | ConvertFrom-Json
    $actual = Select-RiviuProcess $record (Get-ProcessInventory) $RepoRoot
    if (-not $actual) { return $null }
    if (-not $record.App) { $record.App = $actual; Save-Owner $record }
    $proc = Get-Process -Id $actual.Id -ErrorAction Stop
    if (([long]($proc.StartTime.ToUniversalTime().Ticks / 10000)).ToString() -cne $actual.Created -or
        -not [string]::Equals($proc.Path, $actual.ExecutablePath, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Application identity changed during lookup.'
    }
    return $proc
}

function Get-AppProcess {
    $proc = Get-AppProcessAny
    if ($proc -and [RiviuWin32]::FindAppWindow([uint32]$proc.Id, $AppTitle) -ne [IntPtr]::Zero) { return $proc }
}

function Assert-OwnedWindow($Window) {
    $proc = Get-AppProcessAny
    if (-not $proc -or $proc.Id -ne $Window.Process.Id -or
        [RiviuWin32]::FindAppWindow([uint32]$proc.Id, $AppTitle) -ne $Window.Handle) {
        throw 'Window ownership changed; no input or close is allowed.'
    }
}

function Assert-InputPoint($Window, [int]$X, [int]$Y) {
    Assert-OwnedWindow $Window
    if ($X -lt $Window.Left -or $Y -lt $Window.Top -or
        $X -ge ($Window.Left + $Window.Width) -or $Y -ge ($Window.Top + $Window.Height)) {
        throw 'Input is outside the owned window.'
    }
    $point = New-Object 'RiviuWin32+POINT'; $point.X = $X; $point.Y = $Y
    $hit = [RiviuWin32]::GetAncestor([RiviuWin32]::WindowFromPoint($point), [RiviuWin32]::GA_ROOT)
    if ($hit -ne $Window.Handle) { throw 'Input target is occluded or belongs to another window.' }
}
function Assert-CursorTarget($Window) {
    $point = New-Object 'RiviuWin32+POINT'
    [void][RiviuWin32]::GetCursorPos([ref]$point)
    Assert-InputPoint $Window $point.X $point.Y
}

function Get-AppWindow {
    $proc = Get-AppProcess
    if (-not $proc) { throw "app window not found - is it running? try: driver.ps1 launch" }
    $handle = [RiviuWin32]::FindAppWindow([uint32]$proc.Id, $AppTitle)
    if ($handle -eq [IntPtr]::Zero) { throw "process $($proc.Id) has no visible app window yet" }
    $rect = New-Object 'RiviuWin32+RECT'
    [void][RiviuWin32]::GetWindowRect($handle, [ref]$rect)
    [pscustomobject]@{
        Process = $proc
        Handle  = $handle
        Left    = $rect.Left
        Top     = $rect.Top
        Width   = $rect.Right - $rect.Left
        Height  = $rect.Bottom - $rect.Top
    }
}

function Get-ForegroundTitle {
    $sb = New-Object System.Text.StringBuilder 512
    [void][RiviuWin32]::GetWindowText([RiviuWin32]::GetForegroundWindow(), $sb, 512)
    return $sb.ToString()
}

function Test-AppForeground {
    param([Parameter(Mandatory = $true)]$Window)
    return ([RiviuWin32]::GetForegroundWindow() -eq $Window.Handle)
}

function Invoke-RawClick {
    param([int]$ScreenX, [int]$ScreenY)
    Assert-InputPoint $script:OperationWindow $ScreenX $ScreenY
    [void][RiviuWin32]::SetCursorPos($ScreenX, $ScreenY)
    Start-Sleep -Milliseconds 250
    Assert-OwnedWindow $script:OperationWindow
    Assert-CursorTarget $script:OperationWindow
    [RiviuWin32]::mouse_event([RiviuWin32]::MOUSEEVENTF_LEFTDOWN, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 90
    [RiviuWin32]::mouse_event([RiviuWin32]::MOUSEEVENTF_LEFTUP, 0, 0, 0, [IntPtr]::Zero)
}

function Enable-AppActive {
    # A click on a window that is not foreground is swallowed by activation, so
    # the element never sees it. Activate with a click on the (inert) title bar
    # first, then the real click lands. Do NOT activate by clicking the target -
    # that double-fires whatever is under it.
    param([Parameter(Mandatory = $true)]$Window)
    Assert-OwnedWindow $Window
    if (Test-AppForeground -Window $Window) { return }
    Invoke-RawClick -ScreenX ($Window.Left + [int]($Window.Width / 2)) -ScreenY ($Window.Top + 15)
    Start-Sleep -Milliseconds 600
    if (-not (Test-AppForeground -Window $Window)) {
        Write-Warning "could not activate the app window (foreground is '$(Get-ForegroundTitle)')"
    }
}

function Get-Occluder {
    # HWND_TOPMOST is not a guarantee: another topmost window (browsers, chat popups,
    # overlays) can still sit above ours, and CopyFromScreen would then silently
    # capture THAT window. Ask Windows what is actually on top at our own pixels.
    param([Parameter(Mandatory = $true)]$Window)
    $samples = @(
        @(0.5, 0.5), @(0.2, 0.25), @(0.8, 0.25), @(0.2, 0.8), @(0.8, 0.8)
    )
    foreach ($s in $samples) {
        $point = New-Object 'RiviuWin32+POINT'
        $point.X = $Window.Left + [int]($Window.Width * $s[0])
        $point.Y = $Window.Top + [int]($Window.Height * $s[1])
        $hit = [RiviuWin32]::WindowFromPoint($point)
        if ($hit -eq [IntPtr]::Zero) { continue }
        $root = [RiviuWin32]::GetAncestor($hit, [RiviuWin32]::GA_ROOT)
        if ($root -ne $Window.Handle) {
            $sb = New-Object System.Text.StringBuilder 512
            [void][RiviuWin32]::GetWindowText($root, $sb, 512)
            return [pscustomobject]@{
                Handle = $root
                Title  = $sb.ToString()
                Point  = "$($point.X),$($point.Y)"
            }
        }
    }
    return $null
}

function Use-RaisedWindow {
    param(
        [Parameter(Mandatory = $true)][scriptblock]$Body,
        [int]$SettleMs = 1500,
        [switch]$Activate,
        # Capture can survive an occluder via PrintWindow; input cannot, because the
        # click would land in the window that is actually on top. So only `shot`
        # passes this, and it gets told whether it must use the fallback.
        [switch]$AllowOccluded
    )
    $win = Get-AppWindow
    Assert-OwnedWindow $win
    $script:OperationWindow = $win
    if ([RiviuWin32]::IsIconic($win.Handle)) {
        Write-Step 'window was minimised - restoring'
        [void][RiviuWin32]::ShowWindow($win.Handle, [RiviuWin32]::SW_RESTORE)
        Start-Sleep -Milliseconds 700
    }
    [void][RiviuWin32]::SetWindowPos($win.Handle, $HWND_TOPMOST, 0, 0, 0, 0, [RiviuWin32]::SWP_RAISE)
    Start-Sleep -Milliseconds $SettleMs

    # The rect was read before the restore, when a minimised window still reports
    # -32000,-32000 160x28. Capturing that yields a 147-byte PNG that "succeeds".
    $fresh = New-Object 'RiviuWin32+RECT'
    [void][RiviuWin32]::GetWindowRect($win.Handle, [ref]$fresh)
    $win.Left = $fresh.Left
    $win.Top = $fresh.Top
    $win.Width = $fresh.Right - $fresh.Left
    $win.Height = $fresh.Bottom - $fresh.Top
    if ($win.Width -lt 400 -or $win.Height -lt 300) {
        [void][RiviuWin32]::SetWindowPos($win.Handle, $HWND_NOTOPMOST, 0, 0, 0, 0, [RiviuWin32]::SWP_RAISE)
        throw ("app window is {0}x{1} at {2},{3} - it did not restore to a usable size" -f `
            $win.Width, $win.Height, $win.Left, $win.Top)
    }

    if ($Activate) { Enable-AppActive -Window $win }

    # A wrong screenshot is worse than no screenshot, so prove we are on top.
    $blocker = Get-Occluder -Window $win
    if ($blocker) {
        throw 'Owned window is occluded; refusing global input. Use scoped CDP for renderer checks.'
    }
    Assert-OwnedWindow $win

    try { & $Body $win }
    finally {
        [void][RiviuWin32]::SetWindowPos($win.Handle, $HWND_NOTOPMOST, 0, 0, 0, 0, [RiviuWin32]::SWP_RAISE)
    }
}

function Test-BitmapBlank {
    # PrintWindow on a GPU-composited webview can hand back a uniform frame. Sample a
    # grid; if every pixel is the same colour the capture is worthless.
    param([Parameter(Mandatory = $true)][System.Drawing.Bitmap]$Bitmap)
    $first = $null
    for ($x = 4; $x -lt $Bitmap.Width; $x += [math]::Max(8, [int]($Bitmap.Width / 24))) {
        for ($y = 4; $y -lt $Bitmap.Height; $y += [math]::Max(8, [int]($Bitmap.Height / 24))) {
            $argb = $Bitmap.GetPixel($x, $y).ToArgb()
            if ($null -eq $first) { $first = $argb }
            elseif ($argb -ne $first) { return $false }
        }
    }
    return $true
}

function Save-WindowPng {
    param([Parameter(Mandatory = $true)]$Window, [Parameter(Mandatory = $true)][string]$Path)
    $bitmap = New-Object System.Drawing.Bitmap $Window.Width, $Window.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        Assert-OwnedWindow $Window
        # Capture only this HWND; never fall back to desktop pixels.
        $hdc = $graphics.GetHdc()
        try { $ok = [RiviuWin32]::PrintWindow($Window.Handle, $hdc, 2) }
        finally { $graphics.ReleaseHdc($hdc) }
        if (-not $ok -or (Test-BitmapBlank -Bitmap $bitmap)) {
            throw 'Owned-window capture unavailable/blank; use scoped CDP, not a desktop fallback.'
        }
        Assert-OwnedWindow $Window
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally { $graphics.Dispose(); $bitmap.Dispose() }
}


function Test-PortListening([int]$Port) {
    return [bool](Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue)
}

# --------------------------------------------------------------------- commands

function Invoke-Launch {
    $mode = Get-RiviuLaunchMode @($Rest)
    # Never stop/attach an existing instance to make room for this helper.
    if (Get-Process -Name $ProcName -ErrorAction SilentlyContinue) {
        throw 'An app is already running. Leave it untouched; close it explicitly before launching a new instance.'
    }
    if (Test-PortListening 5173) { throw 'Port 5173 is in use; refusing an unrelated dev server.' }
    foreach ($name in @('RIVIU_UI_SMOKE', 'RIVIU_UI_SMOKE_DIR', 'RIVIU_MOCK_DEVICES',
        'RIVIU_MOCK_DATA_DIR', 'RIVIU_DEV_DATA_DIR', 'RIVIU_DEV_BACKGROUND',
        'RIVIU_DEV_CDP_PORT', 'RIVIU_DEV_MANUAL_ACCEPTANCE',
        'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS', 'WEBVIEW2_USER_DATA_FOLDER',
        'WEBVIEW2_BROWSER_EXECUTABLE_FOLDER', 'WEBVIEW2_RELEASE_CHANNEL_PREFERENCE')) {
        if ([Environment]::GetEnvironmentVariable($name)) { throw "Clear inherited $name before an explicit launch." }
    }
    $runId = [guid]::NewGuid().ToString('N')
    $runDir = Join-Path $OutDir $runId
    New-Item -ItemType Directory -Path $runDir | Out-Null
    $DevLog = Join-Path $runDir 'tauri-dev.log'
    $env:PATH = Get-DriverPath
    if ($mode -eq 'smoke') {
        # The backend, not this script, claims the as-yet nonexistent scratch directory.
        $env:RIVIU_UI_SMOKE = '1'
        $env:RIVIU_UI_SMOKE_DIR = Join-Path $runDir 'scratch'
        $env:RIVIU_MOCK_DEVICES = '1'
        $env:RIVIU_DEV_BACKGROUND = '1'
        $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
        $listener.Start(); $port = $listener.LocalEndpoint.Port; $listener.Stop()
        $env:RIVIU_DEV_CDP_PORT = [string]$port
    }
    Write-Step "launch mode=$mode (log: $DevLog)"
    try {
        $launcher = Start-Process -FilePath 'cmd.exe' `
            -ArgumentList '/c', "npm run tauri:dev > `"$DevLog`" 2>&1" `
            -WorkingDirectory $AppDir -WindowStyle Hidden -PassThru
    } finally {
        if ($mode -eq 'smoke') {
            foreach ($name in @('RIVIU_UI_SMOKE', 'RIVIU_UI_SMOKE_DIR', 'RIVIU_MOCK_DEVICES', 'RIVIU_DEV_BACKGROUND', 'RIVIU_DEV_CDP_PORT')) {
                [Environment]::SetEnvironmentVariable($name, $null, 'Process')
            }
        }
    }
    $identity = @(Get-ProcessInventory | Where-Object { $_.Id -eq $launcher.Id })
    if ($identity.Count -ne 1 -or -not $identity[0].ExecutablePath) { throw 'Cannot prove launcher identity; do not guess a process to stop.' }
    $target = if ($env:CARGO_TARGET_DIR) { [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR) } else { Join-Path $RepoRoot 'target' }
    $record = [pscustomobject]@{
        Schema = 1; RepoRoot = $RepoRoot; RunId = $runId; Mode = $mode
        Launcher = $identity[0]; App = $null; Log = $DevLog
        Scratch = if ($mode -eq 'smoke') { Join-Path $runDir 'scratch' } else { $null }
        CdpPort = if ($mode -eq 'smoke') { $port } else { $null }
        ExpectedPaths = @((Join-Path $target 'debug/riviu-managers-phone.exe'), (Join-Path $target 'x86_64-pc-windows-msvc/debug/riviu-managers-phone.exe'))
    }
    Save-Owner $record
    for ($i = 0; $i -lt 200; $i++) {
        Start-Sleep -Seconds 3
        if (Get-AppProcess) {
            $win = Get-AppWindow
    Assert-OwnedWindow $win
    $script:OperationWindow = $win
            Write-Step "owned window pid=$($win.Process.Id); mode=$mode; CDP port=$($record.CdpPort)"
            return
        }
        $launcher.Refresh()
        if ($launcher.HasExited) { throw "Launcher exited; inspect $DevLog. No automatic retry." }
    }
    throw "Startup deadline exceeded; inspect $DevLog. No process was killed or retried."
}

function Invoke-Wait {
    $seconds = 300
    if ($Rest -and $Rest[0] -match '^\d+$') { $seconds = [int]$Rest[0] }
    # The window can sit "(Not Responding)" for minutes: lib.rs runs
    # block_on(AppState::bootstrap(..)) inside .setup(), and the sidecar's
    # create_using_usbmux() has no timeout. Waiting it out is correct.
    # Require several consecutive Responding polls: the window answers briefly
    # BEFORE .setup() starts blocking, so a single true is not "ready".
    $streak = 0
    for ($i = 0; $i -lt [math]::Ceiling($seconds / 3); $i++) {
        $proc = Get-AppProcess
        if ($proc -and $proc.Responding) { $streak++ } else { $streak = 0 }
        if ($streak -ge 4) {
            Write-Step "responding for 4 consecutive polls at ~$($i * 3)s (cpu=$([math]::Round($proc.TotalProcessorTime.TotalSeconds,1))s)"
            return
        }
        Start-Sleep -Seconds 3
    }
    throw "window still not responding after ${seconds}s - check: driver.ps1 log"
}

function Invoke-Status {
    $proc = Get-AppProcessAny
    if ($proc) {
        $win = if ([RiviuWin32]::FindAppWindow([uint32]$proc.Id, $AppTitle) -ne [IntPtr]::Zero) { Get-AppWindow } else { $null }
        $shownTitle = if ($win) { $sb = New-Object System.Text.StringBuilder 512; [void][RiviuWin32]::GetWindowText($win.Handle, $sb, 512); $sb.ToString() } else { '<no app window>' }
        Write-Host "app          : pid=$($proc.Id) responding=$($proc.Responding) title='$shownTitle'"
        if ($win) {
            Write-Host ("window       : {0},{1} {2}x{3} visible={4} foreground={5}" -f `
                $win.Left, $win.Top, $win.Width, $win.Height,
                [RiviuWin32]::IsWindowVisible($win.Handle), (Test-AppForeground -Window $win))
        }
    }
    else { Write-Host 'app          : not running' }

    Write-Host "vite :5173   : $(if (Test-PortListening 5173) { 'listening' } else { 'down' })"
    Write-Host "usbmux :27015: $(if (Test-PortListening 27015) { 'listening' } else { 'down' })"

    $env:PATH = Get-DriverPath
    foreach ($tool in 'python3', 'python', 'cargo', 'tidevice') {
        $cmd = Get-Command $tool -ErrorAction SilentlyContinue
        if ($cmd) {
            $version = try { (& $cmd.Source --version 2>&1 | Out-String).Trim() } catch { '?' }
            Write-Host ("{0,-13}: {1}  [{2}]" -f $tool, $version, $cmd.Source)
        }
        else { Write-Host ("{0,-13}: MISSING" -f $tool) }
    }

    Get-Process -Name 'AppleMobileDeviceProcess' -ErrorAction SilentlyContinue |
        ForEach-Object { Write-Host "usbmux proc  : AppleMobileDeviceProcess pid=$($_.Id)" }
    Get-CimInstance Win32_Process -Filter "Name='python3.exe' OR Name='python.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandLine -like '*riviu_pmd.py*' } |
        ForEach-Object { Write-Host "sidecar      : pid=$($_.ProcessId) (arguments omitted)" }
}

function Invoke-Shot {
    if (-not $Rest -or -not $Rest[0]) { throw 'usage: driver.ps1 shot <name>' }
    $path = Join-Path $OutDir ("{0}.png" -f ($Rest[0] -replace '[^\w.-]', '_'))
    Use-RaisedWindow -AllowOccluded -Body {
        param($win)
        Save-WindowPng -Window $win -Path $path
    }
    $size = (Get-Item $path).Length
    Write-Step "saved $path ($size bytes)"
    if ($size -lt 20000) {
        Write-Warning 'tiny PNG - the webview may not have painted yet; wait and shoot again'
    }
}

function Invoke-Click {
    if ($Rest.Count -lt 2) { throw 'usage: driver.ps1 click <x> <y>   (window-relative)' }
    $x = [int]$Rest[0]; $y = [int]$Rest[1]
    $saved = New-Object 'RiviuWin32+POINT'
    [void][RiviuWin32]::GetCursorPos([ref]$saved)
    Use-RaisedWindow -SettleMs 900 -Activate -Body {
        param($win)
        $sx = $win.Left + $x; $sy = $win.Top + $y
        Write-Step "click window($x,$y) -> screen($sx,$sy)"
        Invoke-RawClick -ScreenX $sx -ScreenY $sy
        Start-Sleep -Milliseconds 1200
    }
    [void][RiviuWin32]::SetCursorPos($saved.X, $saved.Y)
}

function Invoke-RightClick {
    # Right-click and capture in ONE process, because the menu it opens closes on the next
    # `pointerdown` anywhere outside it — and a second driver invocation is a second process
    # whose activation click is exactly that. `click` + `shot` therefore always photographs
    # a closed menu, which looks identical to a menu that never opened.
    #
    # The cursor is deliberately NOT restored before the capture, for the same reason
    # `hovershot` does not: a menu row under the pointer is the state being photographed.
    if ($Rest.Count -lt 3) { throw 'usage: driver.ps1 rightclick <x> <y> <name>' }
    $x = [int]$Rest[0]; $y = [int]$Rest[1]
    $path = Join-Path $OutDir ("{0}.png" -f ($Rest[2] -replace '[^\w.-]', '_'))
    Use-RaisedWindow -SettleMs 900 -Activate -Body {
        param($win)
        $sx = $win.Left + $x; $sy = $win.Top + $y
        Write-Step "rightclick window($x,$y) -> screen($sx,$sy)"
        [void][RiviuWin32]::SetCursorPos($sx, $sy)
        Start-Sleep -Milliseconds 250
        Assert-OwnedWindow $script:OperationWindow
        Assert-CursorTarget $script:OperationWindow
        [RiviuWin32]::mouse_event([RiviuWin32]::MOUSEEVENTF_RIGHTDOWN, 0, 0, 0, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 90
        [RiviuWin32]::mouse_event([RiviuWin32]::MOUSEEVENTF_RIGHTUP, 0, 0, 0, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 900
        Save-WindowPng -Window $win -Path $path
        Write-Step "saved $path"
    }
}

function Invoke-MenuShot {
    # Right-click a tile, then left-click one row of the menu that opens, then capture —
    # all in ONE process, because the menu closes on any `pointerdown` outside itself and
    # every driver invocation begins with an activation click on the title bar. Split across
    # two calls, the second call's activation is what closes the menu, so the row is never
    # reached and the capture shows a closed menu: indistinguishable from a row that does
    # nothing. Pass row coordinates read off a `rightclick` capture.
    if ($Rest.Count -lt 5) {
        throw 'usage: driver.ps1 menushot <tileX> <tileY> <rowX> <rowY> <name> [rowX2 rowY2 ...]'
    }
    $tx = [int]$Rest[0]; $ty = [int]$Rest[1]
    $path = Join-Path $OutDir ("{0}.png" -f ($Rest[4] -replace '[^\w.-]', '_'))
    # The first row pair sits before the name (so the one-row call reads naturally); any
    # further pairs follow it, for a row that lives inside a submenu and therefore needs the
    # submenu opened first — in the same process, for the reason above.
    # The unary comma is load-bearing: `@(@(490,715))` flattens to two scalars in PS 5.1,
    # so the first click went to (490,0) and the second to (715,0) — the title bar, twice.
    $rows = @()
    $rows += , @([int]$Rest[2], [int]$Rest[3])
    for ($i = 5; $i + 1 -lt $Rest.Count; $i += 2) {
        $rows += , @([int]$Rest[$i], [int]$Rest[$i + 1])
    }
    Use-RaisedWindow -SettleMs 900 -Activate -Body {
        param($win)
        [void][RiviuWin32]::SetCursorPos($win.Left + $tx, $win.Top + $ty)
        Start-Sleep -Milliseconds 250
        Assert-OwnedWindow $script:OperationWindow
        Assert-CursorTarget $script:OperationWindow
        [RiviuWin32]::mouse_event([RiviuWin32]::MOUSEEVENTF_RIGHTDOWN, 0, 0, 0, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 90
        [RiviuWin32]::mouse_event([RiviuWin32]::MOUSEEVENTF_RIGHTUP, 0, 0, 0, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 800
        foreach ($row in $rows) {
            Write-Step "menushot tile($tx,$ty) row($($row[0]),$($row[1]))"
            Invoke-RawClick -ScreenX ($win.Left + $row[0]) -ScreenY ($win.Top + $row[1])
            Start-Sleep -Milliseconds 900
        }
        Start-Sleep -Milliseconds 900
        Save-WindowPng -Window $win -Path $path
        Write-Step "saved $path"
    }
}

function Invoke-MenuSearch {
    # Right-click a tile, type into the menu's search box, click the first result, capture —
    # one process, same reason as `menushot`. This is the *reliable* way to reach a row that
    # lives inside a submenu: the row's own coordinates depend on which submenus happen to be
    # expanded and on where the menu scrolled to, while a search result is always the first
    # row. The search box is autofocused when the menu opens, so no click is needed to type.
    if ($Rest.Count -lt 4) {
        throw 'usage: driver.ps1 menusearch <tileX> <tileY> <query> <name>'
    }
    $tx = [int]$Rest[0]; $ty = [int]$Rest[1]
    $query = [string]$Rest[2]
    $path = Join-Path $OutDir ("{0}.png" -f ($Rest[3] -replace '[^\w.-]', '_'))
    Use-RaisedWindow -SettleMs 900 -Activate -Body {
        param($win)
        [void][RiviuWin32]::SetCursorPos($win.Left + $tx, $win.Top + $ty)
        Start-Sleep -Milliseconds 250
        Assert-OwnedWindow $script:OperationWindow
        Assert-CursorTarget $script:OperationWindow
        [RiviuWin32]::mouse_event([RiviuWin32]::MOUSEEVENTF_RIGHTDOWN, 0, 0, 0, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 90
        [RiviuWin32]::mouse_event([RiviuWin32]::MOUSEEVENTF_RIGHTUP, 0, 0, 0, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 900
        Write-Step "menusearch tile($tx,$ty) query='$query'"
        Assert-OwnedWindow $script:OperationWindow
        if (-not (Test-AppForeground -Window $script:OperationWindow)) { throw "Foreground changed; refusing keys." }
        [System.Windows.Forms.SendKeys]::SendWait($query)
        Start-Sleep -Milliseconds 700
        # The first result sits 88 px below the right-click point: 22 for the device name,
        # 34 for the search box, then the row's own half-height. Measured on this build at
        # 100% scaling, and the reason a search is used rather than a row coordinate.
        Invoke-RawClick -ScreenX ($win.Left + $tx + 56) -ScreenY ($win.Top + $ty + 88)
        Start-Sleep -Milliseconds 2000
        Save-WindowPng -Window $win -Path $path
        Write-Step "saved $path"
    }
}

function Invoke-HoverShot {
    # Move the cursor over a window-relative point and capture *while hovering*, so a
    # hover-triggered UI (tooltip, menu) is in the frame. Unlike click/scroll it does NOT
    # restore the cursor before the shot — that restore is exactly what hides a hover state.
    if ($Rest.Count -lt 3) { throw 'usage: driver.ps1 hovershot <x> <y> <name>' }
    $x = [int]$Rest[0]; $y = [int]$Rest[1]
    $path = Join-Path $OutDir ("{0}.png" -f ($Rest[2] -replace '[^\w.-]', '_'))
    Use-RaisedWindow -SettleMs 800 -Body {
        param($win)
        $sx = $win.Left + $x; $sy = $win.Top + $y
        # SetCursorPos alone often does not update Chromium/WebView2 :hover — it wants a
        # genuine move *input*. Land 3px off, then nudge onto the target with a relative
        # MOUSEEVENTF_MOVE so the webview sees a real mousemove ending on the element.
        [void][RiviuWin32]::SetCursorPos($sx - 3, $sy)
        Start-Sleep -Milliseconds 120
        [RiviuWin32]::mouse_event([uint32]0x0001, 3, 0, 0, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 120
        [RiviuWin32]::mouse_event([uint32]0x0001, 0, 0, 0, [IntPtr]::Zero)
        # Let the webview render the hover UI before capturing.
        Start-Sleep -Milliseconds 900
        Save-WindowPng -Window $win -Path $path
        Write-Step "hovershot window($x,$y) -> screen($sx,$sy) -> $path"
    }
}

function Invoke-Scroll {
    # Wheel-scroll at a window-relative point, for scrollable popups the window cannot grow to
    # fit (e.g. the Interaction / Group Tools floating panels). Positive notches scroll down,
    # negative up. One notch = WHEEL_DELTA (120). Mirrors Invoke-Click's raise+restore.
    if ($Rest.Count -lt 2) { throw 'usage: driver.ps1 scroll <x> <y> [notches]  (window-relative; +down/-up)' }
    $x = [int]$Rest[0]; $y = [int]$Rest[1]
    $notches = if ($Rest.Count -ge 3) { [int]$Rest[2] } else { 3 }
    $saved = New-Object 'RiviuWin32+POINT'
    [void][RiviuWin32]::GetCursorPos([ref]$saved)
    Use-RaisedWindow -SettleMs 900 -Activate -Body {
        param($win)
        $sx = $win.Left + $x; $sy = $win.Top + $y
        [void][RiviuWin32]::SetCursorPos($sx, $sy)
        Start-Sleep -Milliseconds 200
        # Negative delta scrolls the content down (the wheel turns toward the user). Mask in
        # Int64 (the `L` literal) so a negative delta becomes its two's-complement uint32 —
        # `0xffffffff` alone is Int32 -1 in PS 5.1 and would leave the value negative, which
        # then fails the uint32 cast.
        $delta = -120 * $notches
        $data = [uint32](([int64]$delta) -band ([int64]4294967295))
        Write-Step "scroll window($x,$y) -> screen($sx,$sy) notches=$notches"
        Assert-OwnedWindow $script:OperationWindow
        Assert-CursorTarget $script:OperationWindow
        [RiviuWin32]::mouse_event([uint32]0x0800, 0, 0, $data, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 600
    }
    [void][RiviuWin32]::SetCursorPos($saved.X, $saved.Y)
}

function Invoke-CtrlScroll {
    throw 'Native modifier-held scroll is unavailable; use scoped CDP on an isolated UI smoke.'
}

function Invoke-DblClick {
    if ($Rest.Count -lt 2) { throw 'usage: driver.ps1 dblclick <x> <y>   (window-relative)' }
    $x = [int]$Rest[0]; $y = [int]$Rest[1]
    $saved = New-Object 'RiviuWin32+POINT'
    [void][RiviuWin32]::GetCursorPos([ref]$saved)
    # Both clicks in ONE process, for the same reason `fill` exists: two `click`
    # invocations are two processes and the gap between them is far wider than the
    # double-click interval, so they arrive as two single clicks. Device tiles open
    # their control overlay on double-click and merely select on a single click, so
    # the difference is not cosmetic.
    Use-RaisedWindow -SettleMs 900 -Activate -Body {
        param($win)
        $sx = $win.Left + $x; $sy = $win.Top + $y
        Write-Step "dblclick window($x,$y) -> screen($sx,$sy)"
        [void][RiviuWin32]::SetCursorPos($sx, $sy)
        Start-Sleep -Milliseconds 250
        # Measured, not assumed: at a 125 ms gap this arrived as two SINGLE clicks --
        # the tile toggled selection on and back off and its dblclick handler never
        # ran. `Start-Sleep` has ~15 ms granularity here, so the nominal gap is not
        # what the window sees. Both clicks now go out back-to-back with no sleep
        # between them, which is well inside GetDoubleClickTime on any setting.
        $dctime = [RiviuWin32]::GetDoubleClickTime()
        Write-Step "double-click interval is ${dctime}ms; sending both clicks with no gap"
        Assert-OwnedWindow $script:OperationWindow
        Assert-CursorTarget $script:OperationWindow
        [RiviuWin32]::mouse_event([RiviuWin32]::MOUSEEVENTF_LEFTDOWN, 0, 0, 0, [IntPtr]::Zero)
        [RiviuWin32]::mouse_event([RiviuWin32]::MOUSEEVENTF_LEFTUP, 0, 0, 0, [IntPtr]::Zero)
        Assert-OwnedWindow $script:OperationWindow
        Assert-CursorTarget $script:OperationWindow
        [RiviuWin32]::mouse_event([RiviuWin32]::MOUSEEVENTF_LEFTDOWN, 0, 0, 0, [IntPtr]::Zero)
        [RiviuWin32]::mouse_event([RiviuWin32]::MOUSEEVENTF_LEFTUP, 0, 0, 0, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 1200
    }
    [void][RiviuWin32]::SetCursorPos($saved.X, $saved.Y)
}

function Invoke-Drag {
    throw 'Native drag is unavailable; use scoped CDP on an isolated UI smoke.'
}

function Send-OwnedKeys {
    param([Parameter(Mandatory = $true)]$Window, [Parameter(Mandatory = $true)][string]$Sequence)
    Assert-OwnedWindow $Window
    if (-not (Test-AppForeground -Window $Window)) { throw 'Foreground changed; no keys sent.' }
    [System.Windows.Forms.SendKeys]::SendWait($Sequence)
    Write-Step 'Input dispatched once; content omitted. Value is not verified; observe before any retry.'
}

function Invoke-Fill {
    if ($Rest.Count -lt 3) { throw 'usage: driver.ps1 fill <x> <y> <text>' }
    $x = [int]$Rest[0]; $y = [int]$Rest[1]
    $text = ConvertTo-RiviuLiteralKeys ($Rest[2..($Rest.Count - 1)] -join ' ')
    Use-RaisedWindow -SettleMs 900 -Activate -Body {
        param($win)
        Invoke-RawClick -ScreenX ($win.Left + $x) -ScreenY ($win.Top + $y)
        Start-Sleep -Milliseconds 700
        Send-OwnedKeys -Window $win -Sequence $text
    }
}

function Send-AppKeys {
    param([Parameter(Mandatory = $true)][string]$Sequence)
    $win = Get-AppWindow
    Send-OwnedKeys -Window $win -Sequence $Sequence
}

function Invoke-Type {
    if (-not $Rest) { throw 'usage: driver.ps1 type <text>' }
    Send-AppKeys -Sequence (ConvertTo-RiviuLiteralKeys ($Rest -join ' '))
}

function Invoke-Key {
    if (-not $Rest) { throw 'usage: driver.ps1 key <sendkeys-sequence>' }
    Send-AppKeys -Sequence ($Rest -join ' ')
}

function Invoke-Log {
    if (Test-Path -LiteralPath $OwnerFile) { $DevLog = (Get-Content -LiteralPath $OwnerFile -Raw | ConvertFrom-Json).Log }
    $lines = 40
    if ($Rest -and $Rest[0] -match '^\d+$') { $lines = [int]$Rest[0] }
    if (-not (Test-Path $DevLog)) { Write-Host "no log at $DevLog"; return }
    Get-Content $DevLog -Tail $lines
}

function Invoke-Devices {
    $py = Resolve-Python312
    if (-not $py) { throw 'Python 3.12 not found' }
    $exe = Join-Path $py 'python.exe'
    # Staged copy is what the app actually runs; fall back to the source tree.
    $script = Join-Path $RepoRoot 'target\debug\sidecars\pymobiledevice3\riviu_pmd.py'
    if (-not (Test-Path $script)) { $script = Join-Path $RepoRoot 'sidecars\pymobiledevice3\riviu_pmd.py' }
    Write-Step "sidecar list via $script (60s cap)"
    $job = Start-Job -ScriptBlock { param($e, $s) & $e $s list 2>&1 } -ArgumentList $exe, $script
    try {
        if (Wait-Job $job -Timeout 60) { Receive-Job $job }
        else {
            Stop-Job $job
            Write-Warning 'sidecar list did not return in 60s - lockdown handshake is hanging (see Gotchas)'
        }
    }
    finally { Remove-Job $job -Force -ErrorAction SilentlyContinue }
}

function Invoke-Usbmux {
    if (Test-PortListening 27015) { Write-Step 'usbmux already listening on 27015'; return }
    $pkg = Get-AppxPackage AppleInc.AppleDevices -ErrorAction SilentlyContinue
    if (-not $pkg) { throw 'Apple Devices not installed: winget install --id 9NP83LWLPZ9K --source msstore' }
    # The exe inside C:\Program Files\WindowsApps is ACL-blocked; the AUMID works.
    $aumid = "$($pkg.PackageFamilyName)!AMPDevicesAgent"
    Write-Step "starting $aumid"
    Start-Process 'explorer.exe' -ArgumentList "shell:AppsFolder\$aumid"
    for ($i = 0; $i -lt 20; $i++) {
        Start-Sleep -Seconds 3
        if (Test-PortListening 27015) { Write-Step "usbmux up after ~$((($i + 1) * 3))s"; return }
    }
    throw 'usbmux did not come up on 27015'
}

function Invoke-Android {
    $adb = Resolve-Adb
    if (-not $adb) {
        throw ('adb not found. Unzip platform-tools and either put it on PATH or set ' +
               'RIVIU_ADB_PATH to the adb.exe itself: ' +
               'https://dl.google.com/android/repository/platform-tools-latest-windows.zip')
    }
    Write-Host "adb          : $adb"
    & $adb version | Select-Object -First 2 | ForEach-Object { Write-Host "               $_" }
    Write-Host 'devices      :'
    $lines = & $adb devices -l | Where-Object { $_ -and $_ -notmatch '^List of devices' }
    if (-not $lines) { Write-Host '               (none attached)'; return }
    foreach ($line in $lines) { Write-Host "               $line" }
    if ($lines -match 'unauthorized') {
        Write-Warning 'device is UNAUTHORIZED - accept the "Allow USB debugging" prompt on the phone (tick "always allow")'
    }
    if ($lines -match 'offline') {
        Write-Warning 'device is OFFLINE - inspect the exact serial, cable, authorization and owner; do not restart the shared ADB server.'
    }
    foreach ($line in ($lines | Where-Object { $_ -match '\sdevice(\s|$)' })) {
        $serial = ($line -split '\s+')[0]
        # `wm size` prints TWO lines; Override is the one that matters (AGENTS.md 9).
        $size = (& $adb -s $serial shell wm size) -join ' | '
        $release = (& $adb -s $serial shell getprop ro.build.version.release).Trim()
        $model = (& $adb -s $serial shell getprop ro.product.model).Trim()
        Write-Host "  $serial  model=$model android=$release"
        Write-Host "  $serial  wm size: $size"
    }
}

function Invoke-Occlusion {
    # Diagnostic for the guard in Use-RaisedWindow. With no args: is the app window
    # clear? With `--at <x> <y>`: which top-level window owns that screen pixel.
    if ($Rest -and $Rest[0] -eq '--at') {
        if ($Rest.Count -lt 3) { throw 'usage: driver.ps1 occlusion --at <screenX> <screenY>' }
        $point = New-Object 'RiviuWin32+POINT'
        $point.X = [int]$Rest[1]; $point.Y = [int]$Rest[2]
        $root = [RiviuWin32]::GetAncestor([RiviuWin32]::WindowFromPoint($point), [RiviuWin32]::GA_ROOT)
        $sb = New-Object System.Text.StringBuilder 512
        [void][RiviuWin32]::GetWindowText($root, $sb, 512)
        Write-Host "screen($($point.X),$($point.Y)) belongs to hwnd=$root '$($sb.ToString())'"
        return
    }
    $win = Get-AppWindow
    Assert-OwnedWindow $win
    $script:OperationWindow = $win
    $blocker = Get-Occluder -Window $win
    if ($blocker) { Write-Host "OCCLUDED by '$($blocker.Title)' (hwnd=$($blocker.Handle)) at $($blocker.Point)" }
    else { Write-Host "clear - app window owns all sampled points in $($win.Left),$($win.Top) $($win.Width)x$($win.Height)" }
}

function Invoke-Stop {
    $proc = Get-AppProcessAny
    if (-not $proc) { Write-Step 'Recorded app has exited; launcher tree is left untouched.'; return }
    $win = Get-AppWindow
    Assert-OwnedWindow $win
    $script:OperationWindow = $win
    Assert-OwnedWindow $win
    Write-Step "WM_CLOSE -> owned pid $($proc.Id)"
    [void][RiviuWin32]::PostMessage($win.Handle, [RiviuWin32]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Seconds 1
        if (-not (Get-AppProcessAny)) {
            Write-Step 'Owned app exited. Dev watcher may remain; no launcher or unrelated process was killed.'
            return
        }
    }
    throw 'Graceful shutdown is still pending. No force kill, launcher reap or device cleanup was performed.'
}

# ----------------------------------------------------------------------- switch

switch ($Command.ToLowerInvariant()) {
    'launch'  { Invoke-Launch }
    'wait'    { Invoke-Wait }
    'status'  { Invoke-Status }
    'shot'    { Invoke-Shot }
    'click'   { Invoke-Click }
    'scroll'  { Invoke-Scroll }
    'ctrlscroll' { Invoke-CtrlScroll }
    'hovershot' { Invoke-HoverShot }
    'rightclick' { Invoke-RightClick }
    'menushot'   { Invoke-MenuShot }
    'menusearch' { Invoke-MenuSearch }
    'dblclick' { Invoke-DblClick }
    'drag'     { Invoke-Drag }
    'fill'    { Invoke-Fill }
    'type'    { Invoke-Type }
    'key'     { Invoke-Key }
    'log'     { Invoke-Log }
    'devices' { Invoke-Devices }
    'usbmux'    { Invoke-Usbmux }
    'occlusion' { Invoke-Occlusion }
    'android'   { Invoke-Android }
    'stop'      { Invoke-Stop }
    default   { throw "unknown command '$Command' (launch|wait|status|shot|click|fill|type|key|log|devices|usbmux|android|occlusion|stop)" }
}
