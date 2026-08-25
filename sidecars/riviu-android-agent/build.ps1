# Build the Riviu helper APK, or refuse with a named reason.
# Does not download a JDK or an Android SDK. Does not invent a digest.
#
# Two paths, and the second is not a shortcut: Gradle when it is on PATH, otherwise the
# build-tools pipeline it would have driven anyway (aapt2 -> javac -> d8 -> zipalign ->
# apksigner). This app is four hundred lines of Java with two resource files and no
# dependencies, so the second path produces the same APK from the same inputs — and it is the
# only path on a machine that has the Android SDK but not Gradle, which is this one. Neither
# path invents a signature: both sign with the standard debug keystore, and shipping requires
# pinning bytes + SHA-256 by hand (see README).

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $Root

function Fail([string]$Message) {
    Write-Error $Message
    exit 1
}

function Require-Tool([string]$Path, [string]$What) {
    if (-not (Test-Path $Path)) { Fail "$What not found at $Path" }
    return $Path
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

Write-Host "JAVA_HOME=$JavaHome"
Write-Host "ANDROID_SDK=$Sdk"

$Gradle = Get-Command gradle -ErrorAction SilentlyContinue
if ($Gradle) {
    $SdkEscaped = $Sdk.Replace("\", "\\")
    Set-Content -Path (Join-Path $Root "local.properties") -Value "sdk.dir=$SdkEscaped" -Encoding ascii
    & gradle ":app:assembleDebug" --no-daemon
    if ($LASTEXITCODE -ne 0) {
        Fail "gradle assembleDebug failed with exit $LASTEXITCODE"
    }
    $Apk = Join-Path $Root "app\build\outputs\apk\debug\app-debug.apk"
    if (-not (Test-Path $Apk)) {
        Fail "gradle reported success but $Apk is missing"
    }
}
else {
    Write-Host "gradle is not on PATH - building with the SDK build-tools directly"

    # Highest installed build-tools. Sorted as text, which is correct for the `34.0.0` shape
    # and would only mislead across a two-digit major that does not exist yet.
    $BuildToolsDir = Get-ChildItem (Join-Path $Sdk "build-tools") -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending | Select-Object -First 1
    if (-not $BuildToolsDir) { Fail "SDK at $Sdk has no build-tools - install build-tools;34.0.0" }
    Write-Host "BUILD_TOOLS=$($BuildToolsDir.Name)"

    $Aapt2     = Require-Tool (Join-Path $BuildToolsDir.FullName "aapt2.exe")     "aapt2"
    $D8        = Require-Tool (Join-Path $BuildToolsDir.FullName "d8.bat")        "d8"
    $ZipAlign  = Require-Tool (Join-Path $BuildToolsDir.FullName "zipalign.exe")  "zipalign"
    $ApkSigner = Require-Tool (Join-Path $BuildToolsDir.FullName "apksigner.bat") "apksigner"
    $Javac     = Require-Tool (Join-Path $JavaHome "bin\javac.exe")               "javac"
    $Jar       = Require-Tool (Join-Path $JavaHome "bin\jar.exe")                 "jar"

    $Keystore = Join-Path $env:USERPROFILE ".android\debug.keystore"
    if (-not (Test-Path $Keystore)) {
        Fail "no debug keystore at $Keystore. Create one: keytool -genkeypair -keystore `"$Keystore`" -alias androiddebugkey -storepass android -keypass android -dname `"CN=Android Debug,O=Android,C=US`" -validity 10000 -keyalg RSA -keysize 2048"
    }

    # Read the version from build.gradle rather than repeating it here: two places to change
    # is one place to forget, and the desktop compares what /status says against what it needs.
    $GradleText = Get-Content (Join-Path $Root "app\build.gradle") -Raw
    $VersionCode = ([regex]::Match($GradleText, 'versionCode\s+(\d+)')).Groups[1].Value
    $VersionName = ([regex]::Match($GradleText, 'versionName\s+"([^"]+)"')).Groups[1].Value
    $MinSdk = ([regex]::Match($GradleText, 'minSdk\s+(\d+)')).Groups[1].Value
    $TargetSdk = ([regex]::Match($GradleText, 'targetSdk\s+(\d+)')).Groups[1].Value
    if (-not $VersionCode -or -not $VersionName -or -not $MinSdk -or -not $TargetSdk) {
        Fail "could not read versionCode/versionName/minSdk/targetSdk out of app/build.gradle"
    }
    Write-Host "version $VersionName ($VersionCode), minSdk $MinSdk, targetSdk $TargetSdk"

    $Out = Join-Path $Root "build-tools-out"
    if (Test-Path $Out) { Remove-Item -Recurse -Force $Out }
    New-Item -ItemType Directory -Force -Path $Out | Out-Null
    $Gen = Join-Path $Out "gen"
    $Classes = Join-Path $Out "classes"
    $DexDir = Join-Path $Out "dex"
    New-Item -ItemType Directory -Force -Path $Gen, $Classes, $DexDir | Out-Null

    & $Aapt2 compile --dir (Join-Path $Root "app\src\main\res") -o (Join-Path $Out "res.zip")
    if ($LASTEXITCODE -ne 0) { Fail "aapt2 compile failed with exit $LASTEXITCODE" }

    # aapt2 wants `package` on the <manifest> tag; the Gradle build supplies it from
    # `namespace` in build.gradle, so the checked-in manifest deliberately does not carry it.
    # Injected into a copy rather than added to the source: a manifest with both a `package`
    # attribute and a `namespace` is what Android Gradle Plugin 8 refuses to build.
    $Namespace = ([regex]::Match($GradleText, 'namespace\s+"([^"]+)"')).Groups[1].Value
    if (-not $Namespace) { Fail "could not read `namespace` out of app/build.gradle" }
    $Manifest = Join-Path $Out "AndroidManifest.xml"
    $ManifestText = Get-Content (Join-Path $Root "app\src\main\AndroidManifest.xml") -Raw
    $Patched = $ManifestText -replace '(?s)<manifest\s', "<manifest package=`"$Namespace`" "
    if ($Patched -eq $ManifestText) { Fail "did not find a <manifest> tag to stamp the package onto" }
    Set-Content -Path $Manifest -Value $Patched -Encoding utf8

    & $Aapt2 link `
        -o (Join-Path $Out "base.apk") `
        -I $Platform `
        --manifest $Manifest `
        --min-sdk-version $MinSdk `
        --target-sdk-version $TargetSdk `
        --version-code $VersionCode `
        --version-name $VersionName `
        --java $Gen `
        (Join-Path $Out "res.zip")
    if ($LASTEXITCODE -ne 0) { Fail "aapt2 link failed with exit $LASTEXITCODE" }

    $Sources = @()
    $Sources += (Get-ChildItem -Recurse (Join-Path $Root "app\src\main\java") -Filter *.java).FullName
    $Sources += (Get-ChildItem -Recurse $Gen -Filter *.java -ErrorAction SilentlyContinue).FullName
    if (-not $Sources) { Fail "no .java sources found under app/src/main/java" }

    # `-source 8 -target 8` and not `--release 8`: android.jar is the platform's own class
    # library, and --release would pin the JDK's instead and reject every android.* import.
    # d8 desugars whatever javac leaves.
    & $Javac -source 8 -target 8 -nowarn -encoding UTF-8 -classpath $Platform -d $Classes @Sources
    if ($LASTEXITCODE -ne 0) { Fail "javac failed with exit $LASTEXITCODE" }

    $ClassFiles = (Get-ChildItem -Recurse $Classes -Filter *.class).FullName
    & $D8 --lib $Platform --min-api $MinSdk --output $DexDir @ClassFiles
    if ($LASTEXITCODE -ne 0) { Fail "d8 failed with exit $LASTEXITCODE" }

    $Unsigned = Join-Path $Out "unsigned.apk"
    Copy-Item (Join-Path $Out "base.apk") $Unsigned
    # An APK is a zip, so `jar uf` is the whole of "add the dex" — no extra tool needed.
    Push-Location $DexDir
    & $Jar uf $Unsigned "classes.dex"
    $JarExit = $LASTEXITCODE
    Pop-Location
    if ($JarExit -ne 0) { Fail "adding classes.dex failed with exit $JarExit" }

    $Aligned = Join-Path $Out "aligned.apk"
    & $ZipAlign -f 4 $Unsigned $Aligned
    if ($LASTEXITCODE -ne 0) { Fail "zipalign failed with exit $LASTEXITCODE" }

    $Apk = Join-Path $Out "riviu-agent.apk"
    # Align BEFORE signing: v2 signatures cover the whole file, so aligning afterwards would
    # invalidate them.
    & $ApkSigner sign --ks $Keystore --ks-pass pass:android --key-pass pass:android --out $Apk $Aligned
    if ($LASTEXITCODE -ne 0) { Fail "apksigner failed with exit $LASTEXITCODE" }
    & $ApkSigner verify $Apk
    if ($LASTEXITCODE -ne 0) { Fail "apksigner verify refused the APK it just signed" }
}

$Hash = (Get-FileHash -Algorithm SHA256 $Apk).Hash.ToLowerInvariant()
$Bytes = (Get-Item $Apk).Length
Write-Host "built $Apk"
Write-Host "bytes $Bytes"
Write-Host "sha256 $Hash"
Write-Host "Pin those numbers in sidecars/android/android-tools-manifest.json (role riviuAgentApk) only after you copy this file to sidecars/android/noarch/riviu-agent.apk."
