"""Build the debug native controller for a device round and freeze its identity.

Promoted from target/acceptance-baidang-20261007/build-main-native.py. Run `npm run build` in
apps/desktop first: the binary embeds apps/desktop/dist.

usage: py -3.12 -I scripts/device-acceptance/build-native.py <label> [--out-dir DIR] [--dry-run]

Writes <out-dir>/native-<label>.{execution.json,stdout.txt,stderr.txt} and, only when cargo
exited 0 AND no source file changed during the build (`sourceStable`), a frozen copy
<out-dir>/riviu-<label>-acceptance.exe whose sha256 launch.py re-checks. Default out-dir:
<main checkout>/target/device-acceptance/builds. Never edit source while this runs.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _common import MAIN, OUT_ROOT, REPO, check_label, utc_now  # noqa: E402

COMMAND = ['cargo', 'build', '--offline', '--locked', '-p', 'riviu-managers-phone',
           '--features', 'tauri/custom-protocol', '--bin', 'riviu-managers-phone']


def source_files() -> list[Path]:
    rust = [p for folder in ('crates', 'apps/desktop/src-tauri/src') for p in (REPO / folder).rglob('*.rs')]
    manifests = [REPO / 'Cargo.lock', REPO / 'Cargo.toml', REPO / 'apps/desktop/src-tauri/Cargo.toml',
                 *(REPO / 'crates').glob('*/Cargo.toml')]
    dist = [p for p in (REPO / 'apps/desktop/dist').rglob('*') if p.is_file()]
    return sorted({p for p in rust + manifests + dist if p.is_file()})


def hashes(files: list[Path]) -> dict[str, str]:
    return {p.relative_to(REPO).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in files}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('label')
    parser.add_argument('--out-dir', type=Path, default=OUT_ROOT / 'builds')
    parser.add_argument('--dry-run', action='store_true', help='in kế hoạch, không build')
    args = parser.parse_args()
    label = check_label(args.label)
    out_dir: Path = args.out_dir.resolve()
    target_dir = Path(os.environ.get('CARGO_TARGET_DIR') or MAIN / 'target')
    record_path = out_dir / f'native-{label}.execution.json'
    if not (REPO / 'apps/desktop/dist/index.html').exists():
        raise SystemExit('thiếu apps/desktop/dist; chạy npm run build trong apps/desktop trước')
    if record_path.exists():
        raise SystemExit(f'{record_path} đã có; chọn nhãn mới')
    plan = {'command': COMMAND, 'cwd': str(REPO), 'targetDir': str(target_dir), 'record': str(record_path)}
    if args.dry_run:
        print(json.dumps({'dryRun': True, **plan}, ensure_ascii=False))
        return 0

    out_dir.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, 'CARGO_TARGET_DIR': str(target_dir), 'RIVIU_DEFAULT_AGENT_MODE': 'full'}
    oauth = REPO / 'apps/desktop/src-tauri/google-oauth.local.json'
    if oauth.exists():
        env['RIVIU_GOOGLE_OAUTH_CONFIG_FILE'] = str(oauth)  # path only; the file is never read here
    files = source_files()
    before = hashes(files)
    start = utc_now()
    stdout, stderr = out_dir / f'native-{label}.stdout.txt', out_dir / f'native-{label}.stderr.txt'
    with stdout.open('xb') as out, stderr.open('xb') as err:
        proc = subprocess.run(COMMAND, cwd=REPO, env=env, stdout=out, stderr=err)
    after = hashes(source_files())
    event = {**plan, 'startedAt': start, 'endedAt': utc_now(), 'exitCode': proc.returncode,
             'sourceStable': before == after, 'sourceHashes': after, 'stdout': str(stdout), 'stderr': str(stderr)}
    if proc.returncode == 0 and event['sourceStable']:
        exe = out_dir / f'riviu-{label}-acceptance.exe'
        exe.write_bytes((target_dir / 'debug' / 'riviu-managers-phone.exe').read_bytes())
        event['binary'] = {'path': str(exe), 'sha256': hashlib.sha256(exe.read_bytes()).hexdigest()}
    with record_path.open('x', encoding='utf-8') as f:
        json.dump(event, f, indent=2)
    print(json.dumps({k: v for k, v in event.items() if k != 'sourceHashes'}, ensure_ascii=False), flush=True)
    if proc.returncode == 0 and not event['sourceStable']:
        print('Source đổi trong lúc build: không đóng băng exe; build lại với nhãn mới.', file=sys.stderr)
        return 3
    return proc.returncode


if __name__ == '__main__':
    sys.exit(main())
