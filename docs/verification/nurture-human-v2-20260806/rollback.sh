#!/bin/sh
set -eu

APP="/Applications/Riviumanagersphone Full.app"
ROLLBACK="/Applications/Riviumanagersphone Full.app.rollback-20260806-human-v2"

test -d "$ROLLBACK"
ditto --rsrc --extattr "$ROLLBACK" "$APP"
codesign --verify --deep --strict "$APP"
shasum -a 256 "$APP/Contents/MacOS/riviu-managers-phone"
