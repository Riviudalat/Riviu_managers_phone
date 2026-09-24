# Windows PowerShell 5.1. Installers may request UAC; the app is not elevated.
[CmdletBinding()]
param([switch]$CheckOnly, [switch]$SetupOnly)
$ErrorActionPreference = 'Stop'

function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    & $Program @Arguments | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "$Program failed (exit $LASTEXITCODE). Fix the error above and run START-RIVIU.cmd again." }
}

function Update-SessionPath {
    $paths = @(
        [Environment]::GetEnvironmentVariable('Path', 'Machine'),
        [Environment]::GetEnvironmentVariable('Path', 'User'),
        "$env:USERPROFILE\.cargo\bin", $env:Path
    )
    $env:Path = $paths -join ';'
}

function Install-Package {
    param([string]$Id, [string]$Override)
    if (-not (Get-Command winget.exe -ErrorAction SilentlyContinue)) {
        throw 'WinGet missing. Install/update App Installer from Microsoft Store, then reopen START-RIVIU.cmd.'
    }
    Write-Host "Installing $Id. Accept the Windows UAC prompt if shown."
    $arguments = @('install', '--id', $Id, '--exact', '--source', 'winget', '--accept-source-agreements', '--accept-package-agreements')
    if ($Override) { $arguments += @('--override', $Override) }
    & winget.exe @arguments | Out-Host
    if ($LASTEXITCODE -in @(3010, 1641, -1978334967, -1978334966, -1978334965)) {
        throw 'Windows restart required. Restart the PC and run START-RIVIU.cmd again.'
    }
    # Already installed/no upgrade: the caller must still verify the actual tool.
    if ($LASTEXITCODE -notin @(0, -1978335189)) { throw "Installation of $Id failed (exit $LASTEXITCODE). See installer output above." }
    Update-SessionPath
}

function Test-Node {
    if (-not (Get-Command node.exe -ErrorAction SilentlyContinue)) { return $false }
    $version = & node.exe -p 'process.versions.node' 2>$null
    if ($LASTEXITCODE -ne 0) { return $false }
    # Vite 8 requires Node 20.19+ or 22.12+; do not accept EOL odd majors.
    try { $v = [version]$version } catch { return $false }
    return (($v.Major -eq 20 -and $v.Minor -ge 19) -or ($v.Major -eq 22 -and $v.Minor -ge 12) -or ($v.Major -ge 24 -and $v.Major % 2 -eq 0)) -and [bool](Get-Command npm.cmd -ErrorAction SilentlyContinue)
}

function Test-Python {
    if (-not (Get-Command py.exe -ErrorAction SilentlyContinue)) { return $false }
    try {
        & py.exe -3.12 -c 'import sys; sys.exit(0 if sys.version_info[:2] == (3, 12) else 1)' 2>$null | Out-Null
        return $LASTEXITCODE -eq 0
    } catch { return $false }
}

function Get-VSPath {
    param([switch]$RequireCpp, [switch]$BuildToolsOnly)
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path -LiteralPath $vswhere)) { return $null }
    $arguments = @('-latest', '-products', '*', '-version', '[17.0,18.0)', '-property', 'installationPath')
    if ($BuildToolsOnly) { $arguments[2] = 'Microsoft.VisualStudio.Product.BuildTools' }
    if ($RequireCpp) { $arguments += @('-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64') }
    $result = & $vswhere @arguments
    if ($LASTEXITCODE -eq 0 -and $result) { return ($result | Select-Object -First 1).Trim() }
    return $null
}

function Import-CppEnvironment {
    $vsPath = Get-VSPath -RequireCpp
    if (-not $vsPath) { return $false }
    $devCmd = Join-Path $vsPath 'Common7\Tools\VsDevCmd.bat'
    if (-not (Test-Path -LiteralPath $devCmd)) { return $false }
    # Use an environment variable, not interpolated shell code, for paths with spaces.
    $env:RIVIU_VS_DEV_CMD = $devCmd
    try {
        $lines = & $env:ComSpec /d /s /c '"call "%RIVIU_VS_DEV_CMD%" -no_logo -arch=x64 -host_arch=x64 >nul && set"'
        if ($LASTEXITCODE -ne 0) { return $false }
        foreach ($line in $lines) {
            if ($line -match '^([^=]+)=(.*)$') { [Environment]::SetEnvironmentVariable($matches[1], $matches[2], 'Process') }
        }
    } finally { Remove-Item Env:RIVIU_VS_DEV_CMD -ErrorAction SilentlyContinue }
    if (-not $env:VCToolsInstallDir -or -not $env:WindowsSdkDir -or -not $env:WindowsSDKVersion) { return $false }
    $required = @(
        (Join-Path $env:VCToolsInstallDir 'bin\Hostx64\x64\cl.exe'),
        (Join-Path $env:VCToolsInstallDir 'bin\Hostx64\x64\link.exe'),
        (Join-Path $env:WindowsSdkDir "Lib\$($env:WindowsSDKVersion.TrimEnd('\'))\um\x64\kernel32.lib"),
        (Join-Path $env:WindowsSdkDir "Lib\$($env:WindowsSDKVersion.TrimEnd('\'))\ucrt\x64\ucrt.lib")
    )
    foreach ($path in $required) { if (-not (Test-Path -LiteralPath $path)) { return $false } }
    return $true
}

function Install-Cpp {
    $vsPath = Get-VSPath -BuildToolsOnly
    if ($vsPath) {
        $setup = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\setup.exe"
        Write-Host 'Adding MSVC and Windows SDK to existing Visual Studio. Accept the UAC prompt.'
        $arguments = 'modify --installPath "{0}" --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --passive --norestart' -f $vsPath
        # --wait is only supported by the bootstrapper, not Installer/setup.exe.
        $process = Start-Process -FilePath $setup -ArgumentList $arguments -Verb RunAs -WindowStyle Hidden -Wait -PassThru
        if ($process.ExitCode -in @(3010, 1641)) { throw 'Restart Windows, then run START-RIVIU.cmd again.' }
        if ($process.ExitCode -ne 0) { throw "Visual Studio installer failed (exit $($process.ExitCode)). Close other installers and retry; check Visual Studio Installer for details." }
    } else {
        Install-Package 'Microsoft.VisualStudio.2022.BuildTools' '--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
    }
    if (-not (Import-CppEnvironment)) { throw 'MSVC/SDK still missing. Open Visual Studio Installer > Modify and verify MSVC v143 x64/x86 and Windows SDK finished installing.' }
}

function Test-WebView {
    $client = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    foreach ($key in @("HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\$client", "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$client", "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\$client")) {
        $entry = Get-ItemProperty -LiteralPath $key -Name pv -ErrorAction SilentlyContinue
        if ($entry -and $entry.pv -and $entry.pv -ne '0.0.0.0') { return $true }
    }
    return $false
}

try {
    if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { throw 'This launcher supports Windows x64.' }
    $repo = Split-Path -Parent $PSScriptRoot
    Set-Location -LiteralPath $repo
    Update-SessionPath
    $checks = [ordered]@{
        'Node/npm' = (Test-Node)
        'Rustup' = [bool](Get-Command rustup.exe -ErrorAction SilentlyContinue)
        'Python 3.12' = (Test-Python)
        'MSVC + Windows SDK' = (Import-CppEnvironment)
        'WebView2' = (Test-WebView)
    }
    foreach ($item in $checks.GetEnumerator()) { Write-Host ("{0}: {1}" -f $item.Key, $(if ($item.Value) { 'OK' } else { 'MISSING' })) }
    if ($CheckOnly) {
        if ($checks.Values -contains $false) { exit 1 }
        Write-Host 'Prerequisites detected. Dependencies/build have not been tested.'
        exit 0
    }
    if (-not $checks['Node/npm']) { Install-Package 'OpenJS.NodeJS.LTS' }
    if (-not (Test-Node)) { throw 'Compatible Node/npm still missing. Reopen the terminal or check the Node installation.' }
    if (-not $checks['Rustup']) { Install-Package 'Rustlang.Rustup' }
    if (-not $checks['Python 3.12']) { Install-Package 'Python.Python.3.12' }
    if (-not (Test-Python)) { throw 'Python 3.12 launcher unavailable. Reopen START-RIVIU.cmd after installation.' }
    if (-not $checks['WebView2']) { Install-Package 'Microsoft.EdgeWebView2Runtime' }
    if (-not (Test-WebView)) { throw 'WebView2 runtime was not detected after installation.' }
    # Refreshing PATH after installers can displace MSVC: import it last.
    if (-not (Import-CppEnvironment)) { Install-Cpp }
    $toolchainText = Get-Content -LiteralPath (Join-Path $repo 'rust-toolchain.toml') -Raw
    if ($toolchainText -notmatch '(?m)^channel\s*=\s*"([^"]+)"') { throw 'Cannot read pinned Rust toolchain.' }
    $toolchain = $matches[1]
    Invoke-Checked 'rustup.exe' @('toolchain', 'install', $toolchain, '--profile', 'minimal')
    Invoke-Checked 'cargo.exe' @('--version')
    Invoke-Checked 'py.exe' @('-3.12', '-m', 'pip', 'install', '-r', (Join-Path $repo 'sidecars\pymobiledevice3\requirements.txt'))
    Set-Location -LiteralPath (Join-Path $repo 'apps\desktop')
    Invoke-Checked 'npm.cmd' @('ci')
    if ($SetupOnly) { Write-Host 'Setup complete. Run START-RIVIU.cmd to open the app.'; exit 0 }
    Write-Host 'Starting Riviu. First Rust build can take a while. Close other Riviu instances first.'
    Invoke-Checked 'npm.cmd' @('run', 'tauri:dev')
    exit 0
} catch {
    Write-Host "ERROR: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
