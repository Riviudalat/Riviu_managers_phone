param([string]$OutputDirectory = (Join-Path $env:USERPROFILE 'Downloads'))
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$originalLocation = Get-Location
$oldPath = $env:PATH
$oldMode = $env:RIVIU_DEFAULT_AGENT_MODE
$oldOAuth = $env:RIVIU_GOOGLE_OAUTH_CONFIG_JSON
function Run-Step([string]$Program, [string[]]$Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program failed (exit $LASTEXITCODE)" }
}
try {
    Set-Location $root
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
    $desktop = Join-Path $root 'apps/desktop'
    $version = (Get-Content (Join-Path $desktop 'package.json') -Raw | ConvertFrom-Json).version
    $cargoManifest = [IO.File]::ReadAllText((Join-Path $desktop 'src-tauri/Cargo.toml'))
    if ($cargoManifest -notmatch ('(?m)^version\s*=\s*"' + [regex]::Escape($version) + '"')) { throw 'Version mismatch: Cargo.toml' }
    foreach ($name in @('tauri.conf.json', 'tauri.full.conf.json')) {
        $config = Get-Content (Join-Path $desktop "src-tauri/$name") -Raw | ConvertFrom-Json
        if ($config.version -ne $version) { throw "Version mismatch: $name" }
    }
    $exe = Join-Path $root 'target/release/riviu-managers-phone.exe'
    $runningBuild = @(Get-CimInstance Win32_Process -Filter "name='cargo.exe'" | Where-Object { $_.CommandLine -match 'build|tauri' })
    if ($runningBuild.Count) { throw 'Another Cargo build is running. Wait for it to finish before exporting.' }
    if (-not $env:RIVIU_GOOGLE_OAUTH_CONFIG_JSON) {
        $candidate = $exe
        if (-not (Test-Path -LiteralPath $candidate)) {
            $candidate = Join-Path $env:LOCALAPPDATA 'Riviu Manager Full/riviu-managers-phone.exe'
        }
        if (-not (Test-Path -LiteralPath $candidate)) { throw 'Set RIVIU_GOOGLE_OAUTH_CONFIG_JSON before exporting.' }
        $bytes = [IO.File]::ReadAllBytes($candidate)
        $binaryText = [Text.Encoding]::ASCII.GetString($bytes)
        $clientIds = @([regex]::Matches($binaryText, '[A-Za-z0-9_-]+\.apps\.googleusercontent\.com') | ForEach-Object Value | Sort-Object -Unique)
        if ($clientIds.Count -ne 1) { throw 'Could not resolve the public Google OAuth client ID.' }
        $env:RIVIU_GOOGLE_OAUTH_CONFIG_JSON = @{clientId=$clientIds[0]} | ConvertTo-Json -Compress
        $binaryText = $null
        $bytes = $null
    }
    $env:RIVIU_DEFAULT_AGENT_MODE = 'full'
    $work = Join-Path $root ('target/export-thread/' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
    New-Item -ItemType Directory -Path $work -Force | Out-Null
    $bundleConfig = Join-Path $work 'bundle.json'
    '{"bundle":{"createUpdaterArtifacts":false}}' | Set-Content -LiteralPath $bundleConfig -Encoding UTF8
    Write-Host "Building Threads $version..."
    Set-Location $desktop
    Run-Step 'npm.cmd' @('run','tauri:build','--','--config','src-tauri/tauri.full.conf.json','--no-bundle')
    Set-Location $root
    Run-Step 'cargo.exe' @('build','--release','--locked','-p','riviu-deployment-checker')
    $checker = Join-Path $root 'target/release/riviu-deployment-check.exe'
    $stagedChecker = Join-Path $desktop 'src-tauri/binaries/riviu-deployment-check-x86_64-pc-windows-msvc.exe'
    Copy-Item -LiteralPath $checker -Destination $stagedChecker -Force
    if ((Get-FileHash $checker).Hash -ne (Get-FileHash $stagedChecker).Hash) { throw 'Checker hash mismatch' }
    $report = Join-Path $work 'smoke.json'
    $data = Join-Path $work 'smoke-data'
    Write-Host 'Checking startup with isolated test data...'
    $smoke = Start-Process -FilePath $exe -ArgumentList @('--deployment-smoke', ('"'+$report+'"'), '--data-dir', ('"'+$data+'"')) -WindowStyle Hidden -PassThru
    if (-not $smoke.WaitForExit(60000)) { throw 'Startup check timed out.' }
    if ($smoke.ExitCode -ne 0) { throw 'Startup check failed.' }
    $result = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
    if ($result.status -ne 'ready' -or -not $result.frontendReady -or -not $result.tauriReady) { throw 'Startup is not ready.' }
    Set-Location $desktop
    Run-Step 'npm.cmd' @('exec','tauri','bundle','--','--config','src-tauri/tauri.full.conf.json','--config','src-tauri/tauri.fast-bundle.conf.json','--config',$bundleConfig,'--bundles','nsis')
    $source = Join-Path $root "target/release/bundle/nsis/Riviu Manager Full_${version}_x64-setup.exe"
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
    $destination = Join-Path $OutputDirectory "Riviu-Thread_${version}_x64-setup.exe"
    if (Test-Path -LiteralPath $destination) {
        $destination = Join-Path $OutputDirectory ("Riviu-Thread_${version}_" + (Get-Date -Format 'yyyyMMdd-HHmmss') + '_x64-setup.exe')
    }
    Copy-Item -LiteralPath $source -Destination $destination
    $hash = (Get-FileHash -LiteralPath $source).Hash
    if ($hash -ne (Get-FileHash -LiteralPath $destination).Hash) { throw 'Export hash mismatch.' }
    "$hash  $([IO.Path]::GetFileName($destination))" | Set-Content -LiteralPath "$destination.sha256" -Encoding ASCII
    Write-Host "Exported: $destination"
} finally {
    Set-Location $originalLocation
    $env:PATH = $oldPath
    $env:RIVIU_DEFAULT_AGENT_MODE = $oldMode
    $env:RIVIU_GOOGLE_OAUTH_CONFIG_JSON = $oldOAuth
}
