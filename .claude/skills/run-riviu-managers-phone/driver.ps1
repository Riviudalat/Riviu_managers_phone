#Requires -Version 5.1
<#
    Driver for the Riviumanagersphone Tauri 2 desktop app (Windows).

    The app is a WebView2 window created from Rust (tauri.conf.json sets
    "create": false, lib.rs builds it), so there is no CDP endpoint and no
    Playwright _electron handle. The only reliable handle is Win32: raise the
    window by z-order, capture the screen rectangle, and inject real mouse /
    keyboard input. SetForegroundWindow is refused for a non-foreground caller,
    which is why every visual command goes through SetWindowPos(HWND_TOPMOST).

    Usage:
      powershell -NoProfile -ExecutionPolicy Bypass `
        -File .claude/skills/run-riviu-managers-phone/driver.ps1 <command> [args]

    Commands:
      launch [--mock]      start `npm run tauri:dev` detached; returns when the window exists
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
      stop                 WM_CLOSE the window, then reap the tauri-dev / cmd launcher

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
$LauncherPidFile = Join-Path $OutDir 'launcher.pid'
$ProcName = 'riviu-managers-phone'
$AppTitle = 'Riviumanagersphone'

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
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll", CharSet = CharSet.Auto)] public static extern IntPtr PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
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

function Get-AppProcess {
    Get-Process -Name $ProcName -ErrorAction SilentlyContinue |
        Where-Object { [RiviuWin32]::FindAppWindow([uint32]$_.Id, $AppTitle) -ne [IntPtr]::Zero } |
        Select-Object -First 1
}

function Get-AppProcessAny {
    Get-Process -Name $ProcName -ErrorAction SilentlyContinue | Select-Object -First 1
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
    [void][RiviuWin32]::SetCursorPos($ScreenX, $ScreenY)
    Start-Sleep -Milliseconds 250
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
        Write-Step "occluded by '$($blocker.Title)' at $($blocker.Point) - clicking title bar to raise"
        Invoke-RawClick -ScreenX ($win.Left + [int]($win.Width / 2)) -ScreenY ($win.Top + 15)
        Start-Sleep -Milliseconds 900
        [void][RiviuWin32]::SetWindowPos($win.Handle, $HWND_TOPMOST, 0, 0, 0, 0, [RiviuWin32]::SWP_RAISE)
        Start-Sleep -Milliseconds 700
        $blocker = Get-Occluder -Window $win
        if ($blocker) {
            if (-not $AllowOccluded) {
                [void][RiviuWin32]::SetWindowPos($win.Handle, $HWND_NOTOPMOST, 0, 0, 0, 0, [RiviuWin32]::SWP_RAISE)
                throw ("app window is covered by '{0}' at {1}; refusing to send input to it. " +
                       'Minimise or move that window and retry.') -f $blocker.Title, $blocker.Point
            }
            Write-Step "still covered by '$($blocker.Title)' - capturing with PrintWindow instead of the screen"
            $win | Add-Member -NotePropertyName Occluded -NotePropertyValue $true -Force
        }
    }
    if (-not ($win.PSObject.Properties.Name -contains 'Occluded')) {
        $win | Add-Member -NotePropertyName Occluded -NotePropertyValue $false -Force
    }

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
        if ($Window.Occluded) {
            # PW_RENDERFULLCONTENT (2) - asks the window to render itself, so z-order
            # and whatever is on top of it stop mattering.
            $hdc = $graphics.GetHdc()
            try { $ok = [RiviuWin32]::PrintWindow($Window.Handle, $hdc, 2) }
            finally { $graphics.ReleaseHdc($hdc) }
            if (-not $ok) { throw 'PrintWindow failed and the window is occluded - move the covering window and retry' }
            if (Test-BitmapBlank -Bitmap $bitmap) {
                throw 'PrintWindow returned a blank frame (GPU-composited webview) - move the covering window and retry'
            }
        }
        else {
            $graphics.CopyFromScreen($Window.Left, $Window.Top, 0, 0,
                (New-Object System.Drawing.Size($Window.Width, $Window.Height)))
        }
        $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally { $graphics.Dispose(); $bitmap.Dispose() }
}

function Get-RegionSignature {
    # SHA-256 of a small screen patch, used to prove an input actually changed.
    # Never point this at the device tile: a live MJPEG stream changes every frame.
    param([Parameter(Mandatory = $true)]$Window, [int]$X, [int]$Y, [int]$W = 280, [int]$H = 32)
    $left = [math]::Max($Window.Left, $Window.Left + $X - [int]($W / 2))
    $top  = [math]::Max($Window.Top, $Window.Top + $Y - [int]($H / 2))
    $w = [math]::Min($W, $Window.Left + $Window.Width - $left)
    $h = [math]::Min($H, $Window.Top + $Window.Height - $top)
    if ($w -le 0 -or $h -le 0) { return $null }
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $graphics = [System.Drawing.Graphics]::FromImage($bmp)
    try {
        $graphics.CopyFromScreen($left, $top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
        $stream = New-Object System.IO.MemoryStream
        $bmp.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
        $sha = [System.Security.Cryptography.SHA256]::Create()
        return [BitConverter]::ToString($sha.ComputeHash($stream.ToArray()))
    }
    finally { $graphics.Dispose(); $bmp.Dispose() }
}

function Test-PortListening([int]$Port) {
    return [bool](Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue)
}

# --------------------------------------------------------------------- commands

function Invoke-Launch {
    if (Get-AppProcessAny) { Write-Step 'app already running - use stop first'; return }

    $useMock = $Rest -contains '--mock'
    if (Test-Path $DevLog) { Remove-Item $DevLog -Force }

    $py = Resolve-Python312
    if (-not $py) { Write-Warning 'Python 3.12 not found; the pymobiledevice3 sidecar will not work' }
    $env:PATH = Get-DriverPath
    if ($useMock) { $env:RIVIU_MOCK_DEVICES = '1'; Write-Step 'RIVIU_MOCK_DEVICES=1 (fake driver)' }

    Write-Step "python3 -> $((Get-Command python3 -ErrorAction SilentlyContinue).Source)"
    Write-Step "cargo   -> $((Get-Command cargo -ErrorAction SilentlyContinue).Source)"
    Write-Step "launching npm run tauri:dev (log: $DevLog)"

    # cmd.exe wrapper keeps the app alive after this PowerShell exits and gives
    # us a single file with both npm and cargo output.
    $launcher = Start-Process -FilePath 'cmd.exe' `
        -ArgumentList '/c', "npm run tauri:dev > `"$DevLog`" 2>&1" `
        -WorkingDirectory $AppDir -WindowStyle Hidden -PassThru
    Write-Step "launcher pid=$($launcher.Id)"
    Set-Content -Path $LauncherPidFile -Value $launcher.Id -Encoding ascii

    # First run compiles ~460 crates; subsequent runs only relink.
    for ($i = 0; $i -lt 200; $i++) {
        Start-Sleep -Seconds 3
        if (Get-AppProcess) {
            $win = Get-AppWindow
            Write-Step ("window up after ~{0}s: pid={1} rect={2},{3} {4}x{5}" -f `
                (($i + 1) * 3), $win.Process.Id, $win.Left, $win.Top, $win.Width, $win.Height)
            return
        }
        if ($launcher.HasExited) {
            Write-Step "launcher exited early (code $($launcher.ExitCode)); last log lines:"
            Invoke-Log
            throw 'tauri dev failed to start'
        }
    }
    throw 'window did not appear within 600s'
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
        ForEach-Object { Write-Host "sidecar      : pid=$($_.ProcessId) $($_.CommandLine)" }
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

function Invoke-Fill {
    if ($Rest.Count -lt 3) { throw 'usage: driver.ps1 fill <x> <y> <text>' }
    $x = [int]$Rest[0]; $y = [int]$Rest[1]
    $text = ($Rest[2..($Rest.Count - 1)] -join ' ')
    $saved = New-Object 'RiviuWin32+POINT'
    [void][RiviuWin32]::GetCursorPos([ref]$saved)
    # click + type must happen in ONE process: between two driver invocations
    # another app can take focus back and swallow the keystrokes.
    Use-RaisedWindow -SettleMs 900 -Activate -Body {
        param($win)
        # The click that focuses the field loses races on a busy desktop, and
        # SendKeys then goes nowhere while everything still "succeeds". Prove the
        # field changed; retry once if it did not.
        $before = Get-RegionSignature -Window $win -X $x -Y $y
        for ($attempt = 1; $attempt -le 2; $attempt++) {
            Invoke-RawClick -ScreenX ($win.Left + $x) -ScreenY ($win.Top + $y)
            Start-Sleep -Milliseconds 700
            if (-not (Test-AppForeground -Window $win)) {
                throw "refusing to type: foreground is '$(Get-ForegroundTitle)', not the app"
            }
            Write-Step "fill window($x,$y) <- $text (attempt $attempt)"
            [System.Windows.Forms.SendKeys]::SendWait($text)
            Start-Sleep -Milliseconds 1200
            if ((Get-RegionSignature -Window $win -X $x -Y $y) -ne $before) {
                Write-Step 'field region changed - text landed'
                return
            }
            Write-Warning "nothing changed around ($x,$y) - click/focus race"
        }
        Write-Warning "fill changed nothing after 2 attempts - verify the coordinates with 'shot'"
    }
    [void][RiviuWin32]::SetCursorPos($saved.X, $saved.Y)
}

function Send-AppKeys {
    param([Parameter(Mandatory = $true)][string]$Sequence)
    $win = Get-AppWindow
    if (-not (Test-AppForeground -Window $win)) {
        throw ("refusing to send keys: foreground is '{0}', not the app. " +
               'Use `fill <x> <y> <text>` so the click and the typing share one process.') -f (Get-ForegroundTitle)
    }
    Write-Step "keys -> $Sequence"
    [System.Windows.Forms.SendKeys]::SendWait($Sequence)
    Start-Sleep -Milliseconds 1200
}

function Invoke-Type {
    if (-not $Rest) { throw 'usage: driver.ps1 type <text>' }
    Send-AppKeys -Sequence ($Rest -join ' ')
}

function Invoke-Key {
    if (-not $Rest) { throw 'usage: driver.ps1 key <sendkeys-sequence>' }
    Send-AppKeys -Sequence ($Rest -join ' ')
}

function Invoke-Log {
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
        Write-Warning 'device is OFFLINE - replug it, or run: adb kill-server; adb start-server'
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
    $blocker = Get-Occluder -Window $win
    if ($blocker) { Write-Host "OCCLUDED by '$($blocker.Title)' (hwnd=$($blocker.Handle)) at $($blocker.Point)" }
    else { Write-Host "clear - app window owns all sampled points in $($win.Left),$($win.Top) $($win.Width)x$($win.Height)" }
}

function Invoke-Stop {
    $proc = Get-AppProcessAny
    if ($proc) {
        $handle = [RiviuWin32]::FindAppWindow([uint32]$proc.Id, $AppTitle)
        if ($handle -ne [IntPtr]::Zero) {
            Write-Step "WM_CLOSE -> pid $($proc.Id) (lets the app run its own exit ordering)"
            [void][RiviuWin32]::PostMessage($handle, [RiviuWin32]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
        }
        for ($i = 0; $i -lt 30; $i++) {
            if (-not (Get-AppProcessAny)) { Write-Step "app exited after ~$($i)s"; break }
            Start-Sleep -Seconds 1
        }
        $still = Get-AppProcessAny
        if ($still) { Write-Step "app still up; terminating pid $($still.Id)"; Stop-Process -Id $still.Id -Force }
    }
    else { Write-Step 'app not running' }

    # Reap the npm/tauri/vite chain. AGENTS.md 2.8 forbids broad kills, so every
    # candidate must satisfy BOTH a command-line fingerprint AND belong to this
    # repo - a bare '*vite*' match would take out an unrelated project's dev
    # server. Preferred source of truth is the launcher's own process tree.
    $all = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue)
    $byId = @{}
    foreach ($proc in $all) { $byId[[int]$proc.ProcessId] = $proc }

    # Two different ownership proofs:
    #  - inside the launcher's process tree, parentage IS the proof, so a name
    #    allowlist is enough (it keeps conhost/powershell children out).
    #  - in the fallback scan there is no tree, so the command line must name THIS
    #    repo; a bare '*vite*' match would kill another project's dev server.
    # cargo.exe must be in the tree allowlist: `tauri dev` runs the app via
    # `cargo run --no-default-features --color always --`, whose command line
    # contains no repo path at all, so the scan can never identify it safely.
    $treeNames = @('node.exe', 'cmd.exe', 'cargo.exe')

    function Test-TreeCandidate($proc) {
        if (-not $proc) { return $false }
        return ($treeNames -contains $proc.Name)
    }

    function Test-ReapCandidate($proc) {
        if (-not $proc) { return $false }
        if ($proc.Name -ne 'node.exe' -and $proc.Name -ne 'cmd.exe') { return $false }
        if (-not $proc.CommandLine) { return $false }
        if ($proc.CommandLine -notlike "*$RepoRoot*") { return $false }
        return ($proc.CommandLine -like '*tauri*' -or $proc.CommandLine -like '*vite*')
    }

    $targets = New-Object 'System.Collections.Generic.HashSet[int]'

    if (Test-Path $LauncherPidFile) {
        $rootPid = [int](Get-Content $LauncherPidFile -Raw).Trim()
        $rootProc = $byId[$rootPid]
        # PID reuse: only trust the recorded pid if it still looks like our launcher.
        if (Test-ReapCandidate $rootProc) {
            [void]$targets.Add($rootPid)
            $queue = New-Object System.Collections.Queue
            $queue.Enqueue($rootPid)
            while ($queue.Count -gt 0) {
                $current = $queue.Dequeue()
                foreach ($proc in $all) {
                    if ([int]$proc.ParentProcessId -ne $current) { continue }
                    if (-not (Test-TreeCandidate $proc)) { continue }
                    if ($targets.Add([int]$proc.ProcessId)) { $queue.Enqueue([int]$proc.ProcessId) }
                }
            }
        }
        else { Write-Step "recorded launcher pid $rootPid no longer matches - falling back to scan" }
    }

    foreach ($proc in $all) { if (Test-ReapCandidate $proc) { [void]$targets.Add([int]$proc.ProcessId) } }

    foreach ($id in $targets) {
        $proc = $byId[$id]
        Write-Step "reaping $($proc.Name) pid=$id"
        Stop-Process -Id $id -Force -ErrorAction SilentlyContinue
    }
    if (Test-Path $LauncherPidFile) { Remove-Item $LauncherPidFile -Force -ErrorAction SilentlyContinue }

    Start-Sleep -Seconds 2
    # Filter on the python process name too: a shell whose own command line
    # contains the literal "riviu_pmd.py" (e.g. a grep for it) matches otherwise.
    $orphans = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like 'python*' -and $_.CommandLine -like '*riviu_pmd.py*' })
    if ($orphans.Count -gt 0) {
        Write-Warning "$($orphans.Count) riviu_pmd.py sidecar(s) still alive: $(($orphans.ProcessId) -join ', ')"
        Write-Warning 'these are usbmux/relay holders - re-run stop, or kill them by pid after checking the command line'
    }
    else { Write-Step 'no riviu_pmd.py sidecars left' }

    # `cargo run` wrappers orphaned by an earlier stop cannot be attributed to this
    # repo from their command line, so report instead of killing blind (AGENTS.md 2.8).
    $strays = @(Get-CimInstance Win32_Process -Filter "Name='cargo.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandLine -like '*run*--no-default-features*' -and -not $targets.Contains([int]$_.ProcessId) })
    if ($strays.Count -gt 0) {
        Write-Warning ("$($strays.Count) stray 'cargo run' wrapper(s) from a previous tauri dev: " +
            (($strays.ProcessId) -join ', ') + ' - verify they are this repo, then Stop-Process them')
    }

    Write-Step "vite :5173 $(if (Test-PortListening 5173) { 'STILL LISTENING' } else { 'closed' })"
}

# ----------------------------------------------------------------------- switch

switch ($Command.ToLowerInvariant()) {
    'launch'  { Invoke-Launch }
    'wait'    { Invoke-Wait }
    'status'  { Invoke-Status }
    'shot'    { Invoke-Shot }
    'click'   { Invoke-Click }
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
