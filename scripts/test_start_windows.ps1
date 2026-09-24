# No installs, downloads, elevation, or app/device startup. Run with Windows PowerShell.
$ErrorActionPreference = 'Stop'
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'start-windows.ps1'), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
# Load only production functions; never execute the bootstrap entry point.
foreach ($definition in $ast.FindAll({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    . ([scriptblock]::Create($definition.Extent.Text))
}
function Assert($Condition, $Message) { if (-not $Condition) { throw $Message } }
function Expect-Error([scriptblock]$Action, [string]$Pattern) {
    $message = $null
    try { & $Action } catch { $message = $_.Exception.Message }
    Assert ($message -and $message -match $Pattern) "Expected error matching '$Pattern', got '$message'"
}

# Native command mocks shadow programs, so nothing is installed or downloaded.
function winget.exe { $script:received = @($args); $global:LASTEXITCODE = $script:installerExit }
function Update-SessionPath {}
$script:installerExit = 0
Install-Package 'Microsoft.VisualStudio.2022.BuildTools' '--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
$index = [Array]::IndexOf($script:received, '--override')
Assert ($index -ge 0 -and $script:received[$index + 1] -match '--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended$') 'Installer override lost its workload or was split.'
$script:installerExit = 87
Expect-Error { Install-Package 'Example.Package' } 'exit 87'
$script:installerExit = 3010
Expect-Error { Install-Package 'Example.Package' } 'restart required'
$script:installerExit = -1978334967
Expect-Error { Install-Package 'Example.Package' } 'restart required'
$script:installerExit = -1978335189
Install-Package 'Example.Package'

function node.exe { $global:LASTEXITCODE = 0; return $script:nodeVersion }
function npm.cmd {}
foreach ($case in @(@('20.18.0', $false), @('20.19.0', $true), @('22.0.0', $false), @('22.12.0', $true), @('24.15.0', $true), @('23.0.0', $false))) {
    $script:nodeVersion = $case[0]
    Assert ((Test-Node) -eq $case[1]) "Node version guard failed for $($case[0])"
}
function py.exe { $global:LASTEXITCODE = 103 }
Assert (-not (Test-Python)) 'A launcher without Python 3.12 must not pass.'

function Get-VSPath { return $null }
Assert (-not (Import-CppEnvironment)) 'Missing C++ component must not pass.'

# Existing incomplete Build Tools must be modified, and rechecked after setup exits.
function Get-VSPath { return 'C:\Build Tools Test' }
function Import-CppEnvironment { return $script:cppReady }
function Start-Process {
    param($FilePath, $ArgumentList, $Verb, $WindowStyle, [switch]$Wait, [switch]$PassThru)
    $script:repairArguments = $ArgumentList
    Assert ($Verb -eq 'RunAs' -and $Wait) 'Repair must request elevation and wait.'
    return [pscustomobject]@{ ExitCode = $script:repairExit }
}
$script:repairExit = 0
$script:cppReady = $false
Expect-Error { Install-Cpp } 'MSVC/SDK still missing'
Assert ($script:repairArguments -match '--installPath "C:\\Build Tools Test"') 'Path with spaces must stay quoted.'
Assert ($script:repairArguments -notmatch '--wait') 'setup.exe does not support --wait.'
$script:cppReady = $true
Install-Cpp
$script:repairExit = 1602
Expect-Error { Install-Cpp } 'exit 1602'
$script:repairExit = 3010
Expect-Error { Install-Cpp } 'Restart Windows'

function failing.exe { $global:LASTEXITCODE = 42 }
Expect-Error { Invoke-Checked 'failing.exe' @() } 'exit 42'
Write-Host 'PASS: parsing, installer arguments/errors/reboot, Node/Python checks, C++ repair verification, command failure propagation.'
