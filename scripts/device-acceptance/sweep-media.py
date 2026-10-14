"""Sweep orphaned publish media off the round's phones before a device round.

usage: py -3.12 -I scripts/device-acceptance/sweep-media.py <label> <device-scope.json> [--apply]

Why: the controller's own sweep (`publish_commands/media_sweep.rs`) is off while a
dev-acceptance scope is active, so acceptance rounds never cleared what failed or cancelled
rows left behind. Measured 14/10/2026 (r112, 20 SM-G955F/N): 16-31 `riviu-*` albums per
phone, and TikTok's picker spent up to 12 s on one album-row lookup in that list until the
step's window closed (`selecting_album`, phones 35/41/42).

Same rule as the controller: an album `Pictures/riviu-<campaign>-<n>-<hash>` or stage
`Pictures/.riviu-publish/<campaign>-<n>` is kept while its assignment on that phone is live
(`Database::live_publish_media_scopes`): any state other than succeeded / failed before
dispatch / cancelled / missed, or succeeded without a cleanup proof. The rest go in the order
`android-driver/src/publish.rs::cleanup` uses: MediaStore rows of exactly that directory, then
`rm -rf`, then a readback. Names that are not Riviu's are never touched.

Read-only by default (lists what would go). `--apply` deletes and refuses while any riviu*.exe
runs: the sweep must not race a controller for the phone. Writes
<out-root>/sweep-media-<label>.tsv.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sqlite3
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _common import OUT_ROOT, CONFIG, check_label, default_db_path, number_of  # noqa: E402

UUID = r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}'
ALBUM = re.compile(rf'^riviu-({UUID}-\d+)-[0-9a-f]{{12}}$')
STAGE = re.compile(rf'^({UUID}-\d+)$')
# Phones swept side by side. One at a time took ~5 min a phone (r113 prep, 14/10/2026).
# Chosen, not measured: well under the controller's 12 adb slots.
PHONES_AT_ONCE = 8
LIVE_SCOPES = """
SELECT a.campaign_id || '-' || a.ordinal FROM publish_assignments a
WHERE a.udid=?1 AND (
  a.state NOT IN ('succeeded','failed_before_dispatch','cancelled','missed')
  OR (a.state='succeeded'
      AND COALESCE(json_extract(a.evidence_json,'$.cleanup.state'),'')<>'cleaned'
      AND COALESCE(json_extract(a.evidence_json,'$.cleanup.value.state'),'')<>'cleaned'))
"""


def adb_path() -> str:
    # The controller's adb: a client of another version restarts the server under it.
    return os.environ.get('RIVIU_ADB_PATH') or CONFIG.get('adb') or 'adb'


def shell(serial: str, command: str) -> str:
    done = subprocess.run([adb_path(), '-s', serial, 'shell', command],
                          capture_output=True, text=True, timeout=120)
    return done.stdout


def controller_running() -> bool:
    count = subprocess.run(['powershell', '-NoProfile', '-Command',
                            "@(Get-CimInstance Win32_Process|Where-Object {$_.Name -like 'riviu*.exe' -and $_.ExecutablePath -notlike '*deps*'}).Count"],
                           capture_output=True, text=True, check=True, timeout=60).stdout.strip()
    return count != '0'


def album_where(album: str) -> str:
    # `album` matched ALBUM, so it holds only [0-9a-z-]: no LIKE wildcard and no quote.
    return f"\"_data LIKE '/storage/emulated/0/Pictures/{album}/%'\""


def media_row_count(serial: str, album: str) -> int:
    listed = ''.join(shell(serial, f'content query --uri content://media/external/{collection}/media '
                                   f'--projection _id --where {album_where(album)}')
                     for collection in ('images', 'video'))
    return len(re.findall(r'_id=\d+', listed))


def sweep_phone(serial: str, live: set[str], apply: bool) -> tuple[list[str], str, int]:
    """One phone: report lines, a summary sentence and the number of failed deletions."""
    lines: list[str] = []
    failures = 0
    albums = [(m.group(0), m.group(1)) for m in map(ALBUM.match, shell(serial, 'ls /sdcard/Pictures').split()) if m]
    stages = [n for n in shell(serial, 'ls /sdcard/Pictures/.riviu-publish 2>/dev/null').split() if STAGE.match(n)]
    gone = 0
    for name, scope in albums:
        if scope in live:
            lines.append(f'{serial}\talbum\t{name}\tkeptLive')
            continue
        result = 'wouldDelete'
        if apply:
            for collection in ('images', 'video'):
                shell(serial, f'content delete --uri content://media/external/{collection}/media --where {album_where(name)}')
            shell(serial, f'rm -rf /sdcard/Pictures/{name}')
            left = media_row_count(serial, name) or name in shell(serial, 'ls /sdcard/Pictures').split()
            result = 'failed' if left else 'deleted'
            failures += bool(left)
        gone += result != 'failed'
        lines.append(f'{serial}\talbum\t{name}\t{result}')
    for name in stages:
        if name in live:
            continue
        result = 'wouldDelete'
        if apply:
            shell(serial, f'rm -rf /sdcard/Pictures/.riviu-publish/{name}')
            result = 'deleted'
        lines.append(f'{serial}\tstage\t{name}\t{result}')
    summary = (f'{number_of(serial)}: {len(albums)} album, {"xóa" if apply else "sẽ xóa"} {gone}, '
               f'giữ {len(albums) - gone} (còn việc), stage {len(stages)}')
    return lines, summary, failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('label')
    parser.add_argument('scope', type=Path)
    parser.add_argument('--apply', action='store_true', help='xóa thật (mặc định chỉ liệt kê)')
    args = parser.parse_args()
    label = check_label(args.label)
    serials = json.loads(args.scope.read_text(encoding='utf-8-sig'))['deviceIds']
    if args.apply and controller_running():
        raise SystemExit('đang có riviu*.exe chạy; đóng controller bằng close.mjs trước khi xóa media')
    db = sqlite3.connect(f'file:{default_db_path()}?mode=ro', uri=True)
    live = {serial: {row[0] for row in db.execute(LIVE_SCOPES, (serial,))} for serial in serials}
    report = OUT_ROOT / f'sweep-media-{label}.tsv'
    report.parent.mkdir(parents=True, exist_ok=True)
    failures = 0
    with report.open('x', encoding='utf-8') as out, ThreadPoolExecutor(max_workers=PHONES_AT_ONCE) as pool:
        out.write('serial\tkind\tname\tresult\n')
        for lines, summary, failed in pool.map(lambda serial: sweep_phone(serial, live[serial], args.apply), serials):
            out.write(''.join(line + '\n' for line in lines))
            out.flush()
            print(summary, flush=True)
            failures += failed
    print(f'Báo cáo: {report}')
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main())
