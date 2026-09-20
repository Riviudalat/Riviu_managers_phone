param([ValidateSet('configure','stats')][string]$Action = 'stats')
$ErrorActionPreference = 'Stop'
$cache = Get-Command sccache -ErrorAction SilentlyContinue
if (-not $cache) {
    $localCache = Join-Path $PSScriptRoot '../target/android-completion-20260919/toolchains/sccache-v0.17.0-x86_64-pc-windows-msvc/sccache.exe'
    if (Test-Path -LiteralPath $localCache) { $cache = Get-Command $localCache }
}
if (-not $cache) { throw 'sccache is not installed. No Cargo or runtime profile was changed.' }
$env:SCCACHE_DIR = Join-Path $env:LOCALAPPDATA 'Riviu/compile-cache'
$env:SCCACHE_CACHE_SIZE = '8G'
if ($Action -eq 'configure') {
    $env:RUSTC_WRAPPER = $cache.Source
    Write-Output 'Compiler cache limited to 8G for this shell; release optimization unchanged.'
}
& $cache.Source --show-stats
exit $LASTEXITCODE
