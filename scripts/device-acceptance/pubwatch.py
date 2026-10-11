"""Poll publish_get for one campaign and summarise each assignment. Observe only, no effects.

Promoted from target/acceptance-20261010/kit/pubwatch.py.

usage: py -3.12 -I scripts/device-acceptance/pubwatch.py <campaignId> <maxSeconds> --out DIR
           [--once] [--interval 60] [--port 47182]
Writes <out>/watch-NNN.json snapshots and prints one line per change:
[{n: Riviu number (from local.config.json numbers) or serial, state, err, url}, ...].
A failed read is printed and the watch goes on until the deadline.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _common import DEFAULT_PORT, KIT, check_port, number_of  # noqa: E402

TERMINAL = {'verified', 'failedBeforeDispatch', 'failed', 'cancelled', 'uncertain', 'completed', 'sheetDelivered', 'done'}


def get(cid: str, port: int):
    out = subprocess.run(['node', str(KIT / 'ipc.mjs'), '--port', str(port), 'publish_get', json.dumps({'campaignId': cid})],
                         capture_output=True, text=True, encoding='utf-8', timeout=120)
    d = json.loads(out.stdout or '{}')
    if not d.get('ok'):
        raise RuntimeError(json.dumps(d.get('error') or {'stderr': out.stderr[-300:]}, ensure_ascii=False))
    return d['value']


def summary(a: dict) -> dict:
    try:
        ev = json.loads(a.get('evidenceJson') or '{}')
    except ValueError:
        ev = {}
    url = None
    s = json.dumps(ev, ensure_ascii=False)
    for key in ('canonicalUrl', 'postUrl', 'url'):
        i = s.find(f'"{key}": "')
        if i >= 0:
            url = s[i + len(key) + 5:].split('"')[0]
            break
    return {'n': number_of(a['udid']), 'state': a['state'], 'err': a.get('errorCode'), 'url': url}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('campaign_id')
    parser.add_argument('max_seconds', type=int)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--once', action='store_true')
    parser.add_argument('--interval', type=int, default=60)
    parser.add_argument('--port', type=int, default=DEFAULT_PORT)
    args = parser.parse_args()
    port = check_port(args.port)
    if args.interval < 10:
        raise SystemExit('--interval >= 10')
    args.out.mkdir(parents=True, exist_ok=True)
    start, n, last = time.time(), 0, None
    while True:
        try:
            v = get(args.campaign_id, port)
            n += 1
            rows = sorted((summary(a) for a in v['assignments']), key=lambda r: str(r['n']))
            (args.out / f'watch-{n:03d}.json').write_text(json.dumps(v, ensure_ascii=False), encoding='utf-8')
            line = json.dumps(rows, ensure_ascii=False)
            if line != last:
                print(time.strftime('%H:%M:%S'), line, flush=True)
                last = line
            done = all(r['state'] in TERMINAL for r in rows)
        except (RuntimeError, ValueError, subprocess.SubprocessError) as error:
            print(time.strftime('%H:%M:%S'), 'READ_ERROR', str(error)[:300], flush=True)
            done = False
        if args.once or done or time.time() - start > args.max_seconds:
            return 0
        time.sleep(args.interval)


if __name__ == '__main__':
    sys.exit(main())
