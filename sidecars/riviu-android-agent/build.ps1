# Build the Riviu helper APK, or refuse with a named reason.
# Does not download a JDK or an Android SDK. Does not invent a digest.

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $Root

function Fail([string]$Message) {
    Write-Error $Message
    exit 1
}

$JavaHome = $env:JAVA_HOME
if (-not $JavaHome -or -not (Test-Path (Join-Path $JavaHome "bin\java.exe"))) {
    Fail "JAVA_HOME must point at a JDK 17+ (java.exe missing). This script does not install one."
}

$Sdk = $env:ANDROID_HOME
if (-not $Sdk) { $Sdk = $env:ANDROID_SDK_ROOT }
if (-not $Sdk -or -not (Test-Path $Sdk)) {
    Fail "ANDROID_HOME or ANDROID_SDK_ROOT must point at an Android SDK with platforms;android-34. This script does not install one."
}

$Platform = Join-Path $Sdk "platforms\android-34\android.jar"
if (-not (Test-Path $Platform)) {
    Fail "SDK at $Sdk has no platforms\android-34\android.jar - install that platform, do not lower minSdk."
}

$Gradle = Get-Command gradle -ErrorAction SilentlyContinue
if (-not $Gradle) {
    Fail "gradle is not on PATH. Open this folder in Android Studio, or install Gradle 8.7+."
}

$SdkEscaped = $Sdk.Replace("\", "\\")
Set-Content -Path (Join-Path $Root "local.properties") -Value "sdk.dir=$SdkEscaped" -Encoding ascii

Write-Host "JAVA_HOME=$JavaHome"
Write-Host "ANDROID_SDK=$Sdk"
& gradle ":app:assembleDebug" --no-daemon
if ($LASTEXITCODE -ne 0) {
    Fail "gradle assembleDebug failed with exit $LASTEXITCODE"
}

$Apk = Join-Path $Root "app\build\outputs\apk\debug\app-debug.apk"
if (-not (Test-Path $Apk)) {
    Fail "gradle reported success but $Apk is missing"
}

$Hash = (Get-FileHash -Algorithm SHA256 $Apk).Hash.ToLowerInvariant()
$Bytes = (Get-Item $Apk).Length
Write-Host "built $Apk"
Write-Host "bytes $Bytes"
Write-Host "sha256 $Hash"
Write-Host "Pin those numbers in sidecars/android/android-tools-manifest.json (role riviuAgentApk) only after you copy this file to sidecars/android/noarch/riviu-agent.apk."
