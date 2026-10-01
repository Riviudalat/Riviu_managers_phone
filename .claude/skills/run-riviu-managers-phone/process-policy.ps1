# Pure ownership decisions. No process, filesystem or Win32 operations on import.
function Test-RiviuIdentity($Expected, $Actual) {
    if (-not $Expected -or -not $Actual) { return $false }
    if (-not $Expected.ExecutablePath -or -not $Actual.ExecutablePath -or
        -not $Expected.Created -or -not $Actual.Created) { return $false }
    return ([int]$Expected.Id -eq [int]$Actual.Id -and
        [string]$Expected.Created -ceq [string]$Actual.Created -and
        [string]::Equals($Expected.ExecutablePath, $Actual.ExecutablePath, [StringComparison]::OrdinalIgnoreCase))
}

function Test-RiviuDescendant($Candidate, $Launcher, $Inventory) {
    $byId = @{}
    foreach ($item in $Inventory) { $byId[[int]$item.Id] = $item }
    if (-not (Test-RiviuIdentity $Launcher $byId[[int]$Launcher.Id])) { return $false }
    $seen = @{}
    $current = $Candidate
    while ($current -and -not $seen.ContainsKey([int]$current.Id)) {
        if (Test-RiviuIdentity $Launcher $current) { return $true }
        $seen[[int]$current.Id] = $true
        $parent = $byId[[int]$current.ParentId]
        if (-not $parent -or -not $current.Created -or -not $parent.Created -or
            [long]$parent.Created -gt [long]$current.Created) { return $false }
        $current = $parent
    }
    return $false
}

function Select-RiviuProcess($Record, $Inventory, [string]$RepoRoot) {
    if (-not $Record -or $Record.RepoRoot -ne $RepoRoot -or $Record.Schema -ne 1) {
        throw 'No valid launcher record for this checkout; refusing to select an application by name.'
    }
    if ($Record.App) {
        $samePid = @($Inventory | Where-Object { [int]$_.Id -eq [int]$Record.App.Id })
        if ($samePid.Count -eq 0) { return $null }
        if ($samePid.Count -ne 1 -or -not (Test-RiviuIdentity $Record.App $samePid[0])) {
            throw 'Recorded application identity changed or is unreadable; refusing PID reuse.'
        }
        return $samePid[0]
    }
    $launcher = @($Inventory | Where-Object { [int]$_.Id -eq [int]$Record.Launcher.Id })
    if ($launcher.Count -ne 1 -or -not (Test-RiviuIdentity $Record.Launcher $launcher[0])) {
        throw 'Recorded launcher identity changed or is unreadable.'
    }
    $matches = @($Inventory | Where-Object {
        $_.Name -eq 'riviu-managers-phone.exe' -and $_.ExecutablePath -and
        ($Record.ExpectedPaths -contains $_.ExecutablePath) -and
        (Test-RiviuDescendant $_ $Record.Launcher $Inventory)
    })
    if ($matches.Count -gt 1) { throw 'Multiple owned applications; select no process.' }
    if ($matches.Count -eq 1) { return $matches[0] }
    return $null
}

function ConvertTo-RiviuLiteralKeys([string]$Text) {
    $output = New-Object System.Text.StringBuilder
    foreach ($character in $Text.ToCharArray()) {
        if ([char]::IsControl($character)) { throw 'Literal input must not contain control characters or implicit Enter/Tab.' }
        if ($character -eq '{') { [void]$output.Append('{{}') }
        elseif ($character -eq '}') { [void]$output.Append('{}}') }
        elseif ('+^%~()[]'.Contains([string]$character)) { [void]$output.Append('{' + $character + '}') }
        else { [void]$output.Append($character) }
    }
    return $output.ToString()
}

function Get-RiviuLaunchMode([string[]]$Arguments) {
    if ($Arguments.Count -eq 0 -or
        ($Arguments.Count -eq 1 -and $Arguments[0] -in @('--smoke', '--mock'))) { return 'smoke' }
    if ($Arguments.Count -eq 1 -and $Arguments[0] -eq '--live-confirmed') { return 'live' }
    throw 'Usage: launch [--smoke|--mock|--live-confirmed]. Live mode requires prior operator authorization.'
}
