param(
  [string]$Serial  = "ce0517155ab38c390d",
  [string]$Package = "com.zhiliaoapp.musically",
  [int]$Rounds     = 22,
  [string]$Out,
  [switch]$ForceStop
)
# ASCII source only: PowerShell 5.1 reads this file as the system ANSI codepage, so a literal
# em-dash or Vietnamese letter here is a parser error, not a display glitch.
#
# Cited from crates/core/src/tiktok_labels.rs and from AGENTS.md as the way to re-derive the
# 46.2.42 badge measurement, so it has to run on a fresh clone. Three things used to stop that:
# an absolute adb path from one machine, a hard-coded scratchpad GUID that had already expired,
# and a call to `dump_agent.ps1` that exists nowhere in the tree. All three are gone -- the dump
# is done here, and the only inputs are a serial and a package.
$ErrorActionPreference = 'Continue'

function Resolve-Adb {
  # Same precedence the app itself uses (crates/android-driver/src/adb.rs): the explicit env
  # var first, then PATH, then the default SDK location. A script that resolves adb differently
  # from the product can measure a different device than the product would drive.
  if ($env:RIVIU_ADB_PATH -and (Test-Path $env:RIVIU_ADB_PATH)) { return $env:RIVIU_ADB_PATH }
  $onPath = Get-Command adb -ErrorAction SilentlyContinue
  if ($onPath) { return $onPath.Source }
  foreach ($root in @($env:ANDROID_SDK_ROOT, $env:ANDROID_HOME,
                      "$env:LOCALAPPDATA\Android\platform-tools")) {
    if (-not $root) { continue }
    foreach ($candidate in @((Join-Path $root "platform-tools\adb.exe"), (Join-Path $root "adb.exe"))) {
      if (Test-Path $candidate) { return $candidate }
    }
  }
  throw "no adb found: set RIVIU_ADB_PATH, or put platform-tools on PATH"
}

$adb = Resolve-Adb
if (-not $Out) { $Out = Join-Path $env:TEMP "riviu-badge-4642" }
New-Item -ItemType Directory -Force -Path $Out | Out-Null
"adb   : $adb"
"out   : $Out"

# "Anh" with the A-with-hook-above, built from its code point so this file stays ASCII.
$anhPattern = 'text="' + [char]0x1EA2 + 'nh"'

function Get-Hierarchy([string]$Destination) {
  # `uiautomator dump` rather than the app's agent: this script drives the phone on its own, so
  # there is no agent to talk to, and the two cannot both hold the accessibility service. Do not
  # run this while the desktop app has the same phone in a live session -- see AGENTS.md on
  # dumping a tree with the app attached.
  & $adb -s $Serial shell uiautomator dump /sdcard/riviu_hunt.xml | Out-Null
  & $adb -s $Serial pull /sdcard/riviu_hunt.xml $Destination | Out-Null
  & $adb -s $Serial shell rm -f /sdcard/riviu_hunt.xml | Out-Null
  if (-not (Test-Path $Destination)) { throw "dump produced no file" }
}

if ($ForceStop) {
  # The feed on this phone has been parked on one card. A force-stop is the only thing that
  # reliably gives TikTok a fresh For You page. 20s is short of the 40s window AGENTS.md 9.19
  # measured for a cold start reaching the *post* page, which is fine: this only needs the feed.
  & $adb -s $Serial shell am force-stop $Package
  Start-Sleep -Seconds 2
  & $adb -s $Serial shell monkey -p $Package -c android.intent.category.LAUNCHER 1 | Out-Null
  "relaunched; waiting for the feed"
  Start-Sleep -Seconds 20
}

$found = 0
for ($i = 1; $i -le $Rounds; $i++) {
  $n = "{0:d2}" -f $i
  $xmlPath = Join-Path $Out "$n.xml"
  try { Get-Hierarchy $xmlPath }
  catch { "  round $n dump failed: " + $_.Exception.Message; continue }
  $xml = Get-Content $xmlPath -Raw -Encoding UTF8
  $hasPhoto = $xml -match 'text="Photo"'
  $hasAnh   = $xml -match $anhPattern
  "round $n  Photo=$hasPhoto  Anh=$hasAnh  xml=$((Get-Item $xmlPath).Length)"
  if ($hasPhoto -or $hasAnh) {
    $found++
    # Keep a frame of the card the badge was read on, so the claim is checkable by eye.
    & $adb -s $Serial shell "screencap -p /sdcard/riviu_badge.png"
    & $adb -s $Serial pull /sdcard/riviu_badge.png (Join-Path $Out "$n-hit.png") | Out-Null
    & $adb -s $Serial shell rm -f /sdcard/riviu_badge.png | Out-Null
    if ($found -ge 3) { "three sightings, stopping"; break }
  }
  if ($i -lt $Rounds) {
    & $adb -s $Serial shell input swipe 540 1600 540 500 220
    Start-Sleep -Milliseconds 2600
  }
}
"sightings: $found -> $Out"
