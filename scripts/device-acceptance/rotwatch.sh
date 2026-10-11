#!/bin/bash
# Poll accelerometer_rotation on every connected phone; log each change with UTC time. Read-only.
# Promoted from target/acceptance-20261010/kit/rotwatch.sh.
# usage: bash scripts/device-acceptance/rotwatch.sh [seconds=3600] [interval=30] [serial,...]
# Use the same adb the controller resolves (RIVIU_ADB_PATH, else PATH): a client of another
# version restarts the adb server under the controller.
if [ "$1" = "--help" ] || [ "$1" = "-h" ]; then
  sed -n '2,6p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
fi
ADB=${RIVIU_ADB_PATH:-adb}
secs=${1:-3600}
interval=${2:-30}
only=${3:-}
case "$secs$interval" in *[!0-9]*) echo "seconds/interval phải là số" >&2; exit 2 ;; esac
[ "$interval" -ge 5 ] || { echo "interval >= 5" >&2; exit 2; }
declare -A last
end=$(( $(date +%s) + secs ))
while [ "$(date +%s)" -lt "$end" ]; do
  for s in $("$ADB" devices | awk '/\tdevice$/{print $1}'); do
    if [ -n "$only" ] && [[ ",$only," != *",$s,"* ]]; then continue; fi
    r=$("$ADB" -s "$s" shell settings get system accelerometer_rotation 2>/dev/null | tr -d '\r')
    if [ "${last[$s]}" != "$r" ]; then echo "$(date -u +%FT%T) $s rotation=$r"; last[$s]=$r; fi
  done
  sleep "$interval"
done
