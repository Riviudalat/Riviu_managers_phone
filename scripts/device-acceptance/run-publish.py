"""Publish (Đăng bài) through scripts/publish_acceptance.mjs, protocol start.

Promoted from target/acceptance-20261010/kit/run-publish.py.
Stages: preflight -> submit --confirm <sha from preflight.json> -> observe.
Never-replay guards (on top of the runner's own):
  * <report-dir>/start-intent.json exists => preflight and submit are skipped; observe only.
  * the runner writes start-intent.json (exclusive) BEFORE publish_start; an exception after
    that is reported as ackUnknownNeverReplay and this wrapper never submits again.
  * the confirmation passed to submit is read from preflight.json and re-hashed by the runner.
  * the mapping in preflight.json must equal the template (udids, bundle ids, order).
  * the controller must have been launched on the same dev scope the runner activates.

usage: py -3.12 -I scripts/device-acceptance/run-publish.py <label> --template <publish-template.json>
           [--wait 5400] [--observe-only] [--port 47182] [--controllers-dir DIR]
Each invocation needs a new label: logs are opened exclusively. Run inspect-source.mjs first.
Never re-submit and never retry `uncertain`: observe it (docs/development/publish-acceptance.md).
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _common import DEFAULT_PORT, OUT_ROOT, REPO, check_label, check_port, utc_now  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('label')
    parser.add_argument('--template', type=Path, required=True)
    parser.add_argument('--wait', type=int, default=5400, help='giây observe tối đa')
    parser.add_argument('--observe-only', action='store_true')
    parser.add_argument('--port', type=int, default=DEFAULT_PORT)
    parser.add_argument('--controllers-dir', type=Path, default=OUT_ROOT / 'controllers')
    args = parser.parse_args()
    label = check_label(args.label)
    port = check_port(args.port)
    if not 60 <= args.wait <= 6 * 3600:
        raise SystemExit('--wait 60..21600')

    template = json.loads(args.template.read_text(encoding='utf-8-sig'))
    common = list(template['common'])
    flag = lambda name: common[common.index(name) + 1]  # noqa: E731
    runner = Path(template.get('runner') or REPO / 'scripts' / 'publish_acceptance.mjs')
    if runner.resolve() != (REPO / 'scripts' / 'publish_acceptance.mjs').resolve():
        raise SystemExit(f'runner phải là scripts/publish_acceptance.mjs của checkout này: {runner}')
    D = Path(flag('--report-dir'))
    RUNS = Path(f'{D}-runs')
    udids = [m['udid'] for m in template['mapping']]
    bundle_ids = [m['bundleId'] for m in template['mapping']]
    assert flag('--udids') == ','.join(udids), '--udids khác mapping'
    assert flag('--bundle-ids') == ','.join(bundle_ids), '--bundle-ids khác mapping'
    assert flag('--cdp') == f'http://127.0.0.1:{port}', f'--cdp phải là http://127.0.0.1:{port}'
    if template.get('contentSnapshot'):
        common = [*common, '--content-snapshot', template['contentSnapshot']]
    RUNS.mkdir(parents=True, exist_ok=True)

    # The controller must have been launched on the same dev scope the runner activates.
    records = [json.loads(p.read_text(encoding='utf-8')) for p in
               sorted(args.controllers_dir.glob('controller-*.json'), key=lambda p: p.stat().st_mtime)]
    records = [r for r in records if r.get('port') == port]
    assert records, f'không có controller-*.json cho cổng {port} trong {args.controllers_dir}; chạy launch.py trước'
    controller = records[-1]
    assert Path(controller['scope']) == Path(flag('--dev-scope')), \
        f"scope controller {controller['scope']} != --dev-scope {flag('--dev-scope')}"
    assert controller['deviceIds'] == udids, 'deviceIds của scope controller phải bằng --udids, đúng thứ tự'

    # Bundle ids must have been confirmed by publish_scan_folder (inspect-source.mjs).
    summary_path = Path(f'{D}-inspect') / 'summary.json'
    inspect_script = REPO / 'scripts' / 'device-acceptance' / 'inspect-source.mjs'
    summary = json.loads(summary_path.read_text(encoding='utf-8')) if summary_path.exists() else {}
    source_path = Path(flag('--source'))
    summary_stale = summary_path.exists() and source_path.exists() and summary_path.stat().st_mtime < source_path.stat().st_mtime
    if summary.get('bundleIdsMatchTemplate') is not True or summary_stale:
        print(json.dumps({'inspect-source': 'auto-running', 'reason': 'stale' if summary_stale else 'missing'}), flush=True)
        inspect_cmd = ['node', str(inspect_script), '--template', str(args.template.resolve()), '--port', str(port)]
        inspect_proc = subprocess.run(inspect_cmd, cwd=REPO)
        if inspect_proc.returncode:
            raise SystemExit(f'inspect-source.mjs thất bại (exit {inspect_proc.returncode})')
        summary = json.loads(summary_path.read_text(encoding='utf-8')) if summary_path.exists() else {}
    assert summary.get('bundleIdsMatchTemplate') is True, 'bundle id quét được khác template'

    def run(stage: str, extra: list[str]) -> int:
        cmd = ['node', str(runner), '--mode', stage, *common, *extra]
        start = utc_now()
        out_p, err_p = RUNS / f'{stage}-{label}.stdout.txt', RUNS / f'{stage}-{label}.stderr.txt'
        with out_p.open('xb') as out, err_p.open('xb') as err:
            proc = subprocess.run(cmd, cwd=REPO, stdout=out, stderr=err)
        event = {'stage': stage, 'command': cmd, 'cwd': str(REPO), 'startedAt': start, 'endedAt': utc_now(),
                 'exitCode': proc.returncode, 'stdout': str(out_p), 'stderr': str(err_p)}
        (RUNS / f'{stage}-{label}.execution.json').write_text(json.dumps(event, indent=2), encoding='utf-8')
        print(json.dumps({k: event[k] for k in ('stage', 'exitCode', 'stdout')}, ensure_ascii=False), flush=True)
        if proc.returncode:
            print(err_p.read_text(encoding='utf-8', errors='replace')[-3000:], flush=True)
        return proc.returncode

    def report() -> dict:
        r = json.loads((D / 'report.json').read_text(encoding='utf-8')) if (D / 'report.json').exists() else {}
        return {k: r.get(k) for k in ('mode', 'acceptance', 'start', 'error', 'exitCode')}

    intent = D / 'start-intent.json'
    if not args.observe_only and not intent.exists():
        code = run('preflight', [])
        print(json.dumps(report(), ensure_ascii=False), flush=True)
        if code:
            return code
        prepared = json.loads((D / 'preflight.json').read_text(encoding='utf-8'))
        request = prepared['approval']['request']
        assert request['udids'] == udids and request['bundleIds'] == bundle_ids, 'mapping preflight bị lệch'
        assert request['sheetEnabled'] is True and request['deleteAfterPublish'] is True
        assert prepared['approval']['scope']['sheetId'] == flag('--sheet-id')
        confirmation = prepared['confirmation']
        assert len(confirmation) == 64
        # accounts is udid -> TikTok handle, which the runner validates as [a-z0-9_.]{1,24}.
        (RUNS / f'submit-{label}-command.json').write_text(json.dumps({
            'confirmation': confirmation, 'requestId': prepared['approval']['requestId'],
            'accounts': prepared['approval']['accounts'], 'at': utc_now()}, indent=2), encoding='utf-8')
        # Re-checked right before the effect: nothing may have started this request meanwhile.
        assert not intent.exists(), 'start intent đã xuất hiện; chỉ observe'
        code = run('submit', ['--confirm', confirmation])
        print(json.dumps(report(), ensure_ascii=False), flush=True)
        if not intent.exists():
            return code or 1  # refused before Start; nothing was sent
        # Any outcome after the intent (ack, ackUnknownNeverReplay, error) continues to observe only.
    else:
        print(json.dumps({'start': 'intentExistsObserveOnly' if intent.exists() else 'observeOnlyRequested'}), flush=True)
        assert intent.exists(), 'không có start intent: không có gì để observe'

    code = run('observe', ['--wait-seconds', str(args.wait), '--poll-seconds', '30'])
    print(json.dumps(report(), ensure_ascii=False), flush=True)
    return code


if __name__ == '__main__':
    sys.exit(main())
