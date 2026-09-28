param(
    [ValidateSet('plan','configure','stats','run')][string]$Action = 'plan',
    [string[]]$CargoArgs = @(),
    [string]$ReceiptPath
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'dev_cache_common.ps1')
$paths = Get-RiviuBuildCachePaths
if ($Action -eq 'plan') { $paths | ConvertTo-Json; return }
$cache = Get-Command sccache -ErrorAction SilentlyContinue
if (-not $cache) {
    $localCache = Join-Path $paths.Target 'android-completion-20260919/toolchains/sccache-v0.17.0-x86_64-pc-windows-msvc/sccache.exe'
    if (Test-Path -LiteralPath $localCache) { $cache = Get-Command $localCache }
}
if (-not $cache) { throw 'sccache is not installed. No Cargo or runtime profile was changed.' }
$env:CARGO_TARGET_DIR = $paths.Target
$env:SCCACHE_DIR = $paths.CompilerCache
$env:SCCACHE_CACHE_SIZE = '8G'
$env:RUSTC_WRAPPER = $cache.Source
if ($Action -eq 'configure') {
    $paths | ConvertTo-Json
    return
}
if ($Action -eq 'stats') {
    & $cache.Source --show-stats
    if ($LASTEXITCODE -ne 0) { throw "sccache stats exited $LASTEXITCODE" }
    return
}
if (-not $CargoArgs.Count -or $CargoArgs[0] -notin @('check','build','test','clippy','run','fmt')) {
    throw 'Supply an approved Cargo check/build/test/clippy/run/fmt command. clean and benchmarks are not supported.'
}
if ($CargoArgs | Where-Object { $_ -match '^--(target-dir|config)(=|$)' }) {
    throw 'Target/config overrides are not allowed in the canonical cache runner.'
}
if (-not $ReceiptPath) { throw 'run requires an explicit receipt path outside build outputs.' }
$receipt = [IO.Path]::GetFullPath($ReceiptPath)
if (Test-Path -LiteralPath $receipt) { throw 'Receipt already exists; choose a new run identity.' }
$lock = Enter-RiviuBuildCacheLock $paths
try {
    $started = [DateTime]::UtcNow.ToString('o')
    & cargo @CargoArgs
    $code = $LASTEXITCODE
    $record = [ordered]@{ schemaVersion=1; target=$paths.Target; source=(Get-Location).Path; command=@('cargo') + $CargoArgs;
        startedAt=$started; finishedAt=[DateTime]::UtcNow.ToString('o'); exitCode=$code; successorVerified=($code -eq 0) }
    $parent = Split-Path -Parent $receipt
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    [IO.File]::WriteAllText($receipt, ($record | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    if ($code -ne 0) { throw "Cargo exited $code; see $receipt" }
} finally {
    $lock.Dispose()
}
