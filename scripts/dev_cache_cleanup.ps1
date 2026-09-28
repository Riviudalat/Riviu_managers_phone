param(
    [Parameter(Mandatory=$true)][string]$ManifestPath,
    [switch]$Apply,
    [string]$ApprovedManifestSha256
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'dev_cache_common.ps1')
$paths = Get-RiviuBuildCachePaths
$manifestFile = (Resolve-Path -LiteralPath $ManifestPath).Path
$manifestHash = (Get-FileHash -LiteralPath $manifestFile -Algorithm SHA256).Hash
$manifest = Get-Content -Raw -LiteralPath $manifestFile | ConvertFrom-Json
if ($manifest.schemaVersion -ne 1 -or [IO.Path]::GetFullPath($manifest.target) -ne $paths.Target) { throw 'Manifest must name the canonical target and schemaVersion 1.' }
if ($Apply -and (!$ApprovedManifestSha256 -or $ApprovedManifestSha256 -ne $manifestHash)) { throw 'Apply requires the approved exact manifest SHA-256.' }
$prefix = $paths.Target.TrimEnd('\','/') + [IO.Path]::DirectorySeparatorChar
$planned = @()
foreach ($entry in $manifest.entries) {
    $path = [IO.Path]::GetFullPath($entry.path)
    if (-not $path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { throw "Outside managed target: $path" }
    Assert-RiviuNoReparsePoint $path
    $relative = $path.Substring($prefix.Length).Replace('\','/')
    # Deliberately only individual Cargo outputs, never a target/profile/folder.
    # Operational runtime, evidence, rollback, fixtures and data are not candidates.
    if ($entry.category -ne 'obsoleteCargoOutput' -or $relative -notmatch '^(debug|release)/(deps|build|\.fingerprint|incremental)/' -or
        $relative -match '(?i)(runtime|evidence|rollback|backup|fixture|sidecar|\.db|\.sqlite)' -or
        [IO.Path]::GetExtension($path) -notin @('.rlib','.rmeta','.o','.obj','.pdb','.d','.lib','.exp')) {
        throw "Not an individually approved compiler output: $path"
    }
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Candidate is missing or is a directory: $path" }
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $entry.sha256) { throw "Candidate hash changed: $path" }
    $planned += [pscustomobject]@{path=$path; sha256=$entry.sha256; bytes=(Get-Item -LiteralPath $path).Length}
}
$plan = [ordered]@{ mode='dryRun'; manifest=$manifestFile; manifestSha256=$manifestHash; target=$paths.Target; entries=$planned; deleted=0 }
if (-not $Apply) { $plan | ConvertTo-Json -Depth 8; return }
$lock = Enter-RiviuBuildCacheLock $paths
try {
    # A build receipt alone does not prove an output exists. Check both the
    # successful successor command and its independently pinned artifact bytes.
    $receipt = Get-Content -Raw -LiteralPath $manifest.successor.receipt | ConvertFrom-Json
    if ($receipt.schemaVersion -ne 1 -or $receipt.exitCode -ne 0 -or !$receipt.successorVerified -or
        [IO.Path]::GetFullPath($receipt.target) -ne $paths.Target -or $receipt.command[0] -ne 'cargo' -or
        $receipt.command[1] -notin @('build','check','test','clippy')) { throw 'No successful successor build receipt.' }
    $successor = [IO.Path]::GetFullPath($manifest.successor.path)
    Assert-RiviuNoReparsePoint $successor
    if (-not $successor.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -or
        $planned.path -contains $successor -or !(Test-Path -LiteralPath $successor -PathType Leaf) -or
        (Get-FileHash -LiteralPath $successor -Algorithm SHA256).Hash -ne $manifest.successor.sha256) { throw 'Successor artifact is not verified or is also a cleanup candidate.' }
    # Revalidate every candidate after admission before deleting any file.
    foreach ($entry in $planned) {
        Assert-RiviuNoReparsePoint $entry.path
        if ((Get-FileHash -LiteralPath $entry.path -Algorithm SHA256).Hash -ne $entry.sha256) { throw 'Candidate changed after planning.' }
    }
    foreach ($entry in $planned) { Remove-Item -LiteralPath $entry.path -Force; $plan.deleted += 1 }
    $plan.mode = 'applied'
    $plan | ConvertTo-Json -Depth 8
} finally { $lock.Dispose() }
