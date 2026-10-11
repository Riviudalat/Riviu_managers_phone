"""Launch the ONE debug acceptance controller frozen by build-native.py.

Promoted from target/acceptance-20261010/kit/launch-box3150.py (same guards; inputs and
outputs are now arguments).

usage: py -3.12 -I scripts/device-acceptance/launch.py <label> <device-scope.json>
           --build <build-label | path to native-*.execution.json> [--port 47182]
           [--out-dir DIR] [--db PATH] [--dry-run]
       RIVIU_BUILD_RECORD=native-<build>.execution.json works instead of --build.

Refuses when: the build record is not exitCode 0 + sourceStable; the frozen exe's sha256
differs from the record; any riviu*.exe is already running (one controller only); the CDP
port is in use; the scope file is malformed; a backup or record with this label exists.
Before the controller starts it takes a read-only online backup of riviu.db. Writes
<out-dir>/controller-<label>.{json,stdout.txt,stderr.txt} and prelaunch-<label>.sqlite.
Default out-dir: <main checkout>/target/device-acceptance/controllers.

Under RIVIU_DEV_MANUAL_ACCEPTANCE=1 schedules and automatic device workers stay frozen. The dev
scope gates publish dispatch, verification, Sheet and cleanup only; for publish its deviceIds
must equal the runner's --udids in order (docs/development/publish-acceptance.md).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import socket
import sqlite3
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _common import DEFAULT_PORT, OUT_ROOT, REPO, check_label, check_port, default_db_path, utc_now  # noqa: E402


def build_record(arg: str | None) -> Path:
    raw = arg or os.environ.get('RIVIU_BUILD_RECORD')
    if not raw:
        raise SystemExit('thiếu --build <nhãn build | file native-*.execution.json>')
    p = Path(raw)
    if p.suffix != '.json':
        p = Path(f'native-{check_label(raw)}.execution.json')
    if not p.is_absolute() and not p.exists():
        p = OUT_ROOT / 'builds' / p
    return p.resolve()


def load_scope(path: Path) -> dict:
    scope = json.loads(path.read_text(encoding='utf-8-sig'))
    # The controller parses the scope with deny_unknown_fields (dev_acceptance.rs) and would
    # silently run without one; refuse here instead.
    unknown = set(scope) - {'activationId', 'campaignIds', 'deviceIds', 'capabilities'}
    if unknown:
        raise SystemExit(f'scope có khóa lạ {sorted(unknown)} mà controller sẽ từ chối: {path}')
    ids = scope.get('deviceIds')
    ok = (isinstance(scope.get('activationId'), str) and scope['activationId']
          and isinstance(ids, list) and ids and all(isinstance(u, str) and u for u in ids) and len(set(ids)) == len(ids)
          and isinstance(scope.get('campaignIds'), list))
    if not ok:
        raise SystemExit(f'scope sai dạng (cần activationId, deviceIds khác rỗng không trùng, campaignIds): {path}')
    return scope


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('label')
    parser.add_argument('scope', type=Path)
    parser.add_argument('--build')
    parser.add_argument('--port', type=int, default=DEFAULT_PORT)
    parser.add_argument('--out-dir', type=Path, default=OUT_ROOT / 'controllers')
    parser.add_argument('--db', type=Path, default=None)
    parser.add_argument('--dry-run', action='store_true', help='chỉ kiểm điều kiện, không backup và không mở controller')
    args = parser.parse_args()
    label = check_label(args.label)
    port = check_port(args.port)
    out_dir: Path = args.out_dir.resolve()
    scope_path = args.scope.resolve()
    scope = load_scope(scope_path)

    record_path = build_record(args.build)
    if not record_path.exists():
        raise SystemExit(f'không có bản ghi build {record_path}; chạy build-native.py trước')
    event = json.loads(record_path.read_text(encoding='utf-8'))
    if not (event.get('exitCode') == 0 and event.get('sourceStable') and event.get('binary')):
        raise SystemExit(f'build chưa sạch/ổn định: {record_path}')
    identity = event['binary']
    exe = Path(identity['path'])
    if hashlib.sha256(exe.read_bytes()).hexdigest() != identity['sha256']:
        raise SystemExit('sha256 của exe khác bản ghi build')

    # One controller only (cargo test binaries under deps are ignored).
    running = subprocess.run(['powershell', '-NoProfile', '-Command',
                              "@(Get-CimInstance Win32_Process|Where-Object {$_.Name -like 'riviu*.exe' -and $_.ExecutablePath -notlike '*deps*'}).Count"],
                             capture_output=True, text=True, check=True, timeout=60).stdout.strip()
    if running != '0':
        raise SystemExit(f'đã có riviu*.exe đang chạy ({running}); đóng controller cũ bằng close.mjs trước')
    with socket.socket() as s:
        s.settimeout(1)
        if s.connect_ex(('127.0.0.1', port)) == 0:
            raise SystemExit(f'cổng CDP {port} đang bận')

    db = (args.db or default_db_path()).resolve()
    backup = out_dir / f'prelaunch-{label}.sqlite'
    record_out = out_dir / f'controller-{label}.json'
    for existing in (backup, record_out):
        if existing.exists():
            raise SystemExit(f'{existing} đã có; chọn nhãn mới')
    if args.dry_run:
        print(json.dumps({'dryRun': True, 'exe': str(exe), 'port': port, 'scope': str(scope_path),
                          'deviceIds': len(scope['deviceIds']), 'db': str(db), 'backup': str(backup)}, ensure_ascii=False))
        return 0

    out_dir.mkdir(parents=True, exist_ok=True)
    # Read-only online backup of the operational DB before the controller touches it.
    with sqlite3.connect(db.as_uri() + '?mode=ro', uri=True) as source, sqlite3.connect(backup) as destination:
        source.backup(destination)

    env = {**os.environ, 'RIVIU_AGENT_MODE': 'full', 'RIVIU_DEV_BACKGROUND': '1', 'RIVIU_DEV_CDP_PORT': str(port),
           'RIVIU_SIDECAR_ROOT': str(REPO / 'sidecars'), 'RIVIU_DEV_MANUAL_ACCEPTANCE': '1',
           'RIVIU_DEV_ACCEPTANCE_SCOPE': str(scope_path), 'RIVIU_DEV_ACCEPTANCE_ACTIVATION': scope['activationId']}
    for name in ['RIVIU_MOCK_DEVICES', 'RIVIU_MOCK_DATA_DIR', 'RIVIU_UI_SMOKE', 'RIVIU_UI_SMOKE_DIR', 'RIVIU_DEV_DATA_DIR']:
        env.pop(name, None)
    with (out_dir / f'controller-{label}.stdout.txt').open('xb') as out, \
            (out_dir / f'controller-{label}.stderr.txt').open('xb') as err:
        child = subprocess.Popen([str(exe)], cwd=REPO, env=env, stdout=out, stderr=err,
                                 creationflags=subprocess.CREATE_NO_WINDOW)
    record = {'pid': child.pid, **identity, 'port': port, 'startedAt': utc_now(), 'buildRecord': str(record_path),
              'scope': str(scope_path), 'deviceIds': scope['deviceIds'], 'campaignIds': scope['campaignIds'],
              'dbBackup': str(backup)}
    with record_out.open('x', encoding='utf-8') as f:
        json.dump(record, f, indent=2)
    print(json.dumps(record, ensure_ascii=False))
    return 0


if __name__ == '__main__':
    sys.exit(main())
