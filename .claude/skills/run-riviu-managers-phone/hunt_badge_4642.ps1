param(
  [string]$Serial = "ce0517155ab38c390d",
  [int]$Port = 6899,
  [int]$Rounds = 22,
  [switch]$ForceStop
)
# ASCII source only: PowerShell 5.1 reads this file as the system ANSI codepage, so a literal
# em-dash or Vietnamese letter here is a parser error, not a display glitch.
$ErrorActionPreference = 'Continue'
$adb = "C:\Users\cattfan\AppData\Local\Android\platform-tools\adb.exe"
$sp  = "C:\Users\cattfan\AppData\Local\Temp\claude\C--Users-cattfan-Desktop-Riviu-managers-phone\f7dabcf4-c56b-483c-bd06-1f31b02b260a\scratchpad"
$out = Join-Path $sp "badge-4642"
New-Item -ItemType Directory -Force -Path $out | Out-Null

# "Anh" with the A-with-hook-above, built from its code point so this file stays ASCII.
$anhPattern = 'text="' + [char]0x1EA2 + 'nh"'

& $adb -s $Serial forward "tcp:$Port" tcp:6790 | Out-Null
"forward tcp:$Port -> 6790"

if ($ForceStop) {
  # The feed on this phone has been parked on one card. A force-stop is the only thing that
  # reliably gives TikTok a fresh For You page.
  & $adb -s $Serial shell am force-stop com.zhiliaoapp.musically
  Start-Sleep -Seconds 2
  & $adb -s $Serial shell monkey -p com.zhiliaoapp.musically -c android.intent.category.LAUNCHER 1 | Out-Null
  "relaunched; waiting for the feed"
  Start-Sleep -Seconds 20
}

$found = 0
for ($i = 1; $i -le $Rounds; $i++) {
  $n = "{0:d2}" -f $i
  try { & "$sp\dump_agent.ps1" -Port $Port -Out "$out\$n.xml" | Out-Null }
  catch { "  round $n dump failed: " + $_.Exception.Message; continue }
  $xml = Get-Content "$out\$n.xml" -Raw -Encoding UTF8
  $hasPhoto = $xml -match 'text="Photo"'
  $hasAnh   = $xml -match $anhPattern
  "round $n  Photo=$hasPhoto  Anh=$hasAnh  xml=$((Get-Item "$out\$n.xml").Length)"
  if ($hasPhoto -or $hasAnh) {
    $found++
    # Keep a frame of the card the badge was read on, so the claim is checkable by eye.
    & $adb -s $Serial shell "screencap -p /sdcard/riviu_badge.png"
    & $adb -s $Serial pull /sdcard/riviu_badge.png "$out\$n-hit.png" | Out-Null
    & $adb -s $Serial shell rm -f /sdcard/riviu_badge.png
    if ($found -ge 3) { "three sightings, stopping"; break }
  }
  if ($i -lt $Rounds) {
    & $adb -s $Serial shell input swipe 540 1600 540 500 220
    Start-Sleep -Milliseconds 2600
  }
}
"sightings: $found -> $out"
& $adb -s $Serial forward --remove "tcp:$Port" | Out-Null
