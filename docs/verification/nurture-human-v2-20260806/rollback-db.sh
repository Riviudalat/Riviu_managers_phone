#!/bin/sh
set -eu

DB_PATH=${RIVIU_DB_PATH:-"$HOME/Library/Application Support/riviu-managers-phone/riviu.db"}
BACKUP_PATH=${RIVIU_NURTURE_DB_BACKUP:-"$HOME/Library/Application Support/riviu-managers-phone/riviu.db.rollback-human-v2-20260806"}

test -f "$DB_PATH"
test -f "$BACKUP_PATH"
test "$(sqlite3 "$BACKUP_PATH" 'PRAGMA integrity_check;')" = ok
sqlite3 "$DB_PATH" ".restore main '$BACKUP_PATH'"
test "$(sqlite3 "$DB_PATH" 'PRAGMA integrity_check;')" = ok
printf 'db_rollback=PASS\n'
