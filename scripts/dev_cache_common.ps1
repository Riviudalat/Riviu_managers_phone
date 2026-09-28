# Shared by the build runner and cleanup planner. No installation or cleanup on import.
function Get-RiviuBuildCachePaths {
    $common = & git -C $PSScriptRoot rev-parse --path-format=absolute --git-common-dir
    if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve main checkout from Git metadata.' }
    $common = [IO.Path]::GetFullPath(($common | Select-Object -Last 1).Trim())
    if ((Split-Path -Leaf $common) -ne '.git') { throw 'Expected a non-bare main checkout.' }
    $main = Split-Path -Parent $common
    $target = [IO.Path]::GetFullPath((Join-Path $main 'target'))
    [pscustomobject]@{ Main=$main; Target=$target; CompilerCache=(Join-Path $target 'compiler-cache'); CacheLimit='8G'; Lock=(Join-Path $target '.riviu-build-cache.lock') }
}

function Assert-RiviuNoReparsePoint([string]$Path) {
    $cursor = [IO.Path]::GetFullPath($Path)
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -Force -LiteralPath $cursor
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Reparse path is not allowed: $cursor" }
        }
        $parent = Split-Path -Parent $cursor
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
}

function Enter-RiviuBuildCacheLock($Paths) {
    Assert-RiviuNoReparsePoint $Paths.Target
    [IO.Directory]::CreateDirectory($Paths.Target) | Out-Null
    try { return [IO.File]::Open($Paths.Lock, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) }
    catch { throw 'Canonical build cache is busy. Wait for its current owner; do not use another target or delete the lock.' }
}
