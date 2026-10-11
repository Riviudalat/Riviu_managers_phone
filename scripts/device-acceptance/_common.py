"""Paths and config shared by the Python scripts of the device-acceptance kit.

The scripts run with `py -3.12 -I`, which keeps the current directory and PYTHONPATH off
sys.path; each script adds only this tracked folder before importing this module.
"""
from __future__ import annotations

import datetime
import json
import os
import re
import subprocess
import sys
from pathlib import Path

# Vietnamese help and messages: a piped Windows console otherwise defaults to cp1252.
for _stream in (sys.stdout, sys.stderr):
    if hasattr(_stream, 'reconfigure'):
        _stream.reconfigure(encoding='utf-8')

KIT = Path(__file__).resolve().parent
REPO = KIT.parent.parent
LABEL_RE = re.compile(r'^[A-Za-z0-9][A-Za-z0-9-]{0,63}$')


def _main_checkout() -> Path:
    """Worktrees share the main checkout's target/ (scripts/dev_cache_common.ps1)."""
    try:
        out = subprocess.run(['git', '-C', str(REPO), 'rev-parse', '--path-format=absolute', '--git-common-dir'],
                             capture_output=True, text=True, check=True, timeout=30).stdout.strip().splitlines()[-1]
        common = Path(out)
        if common.name == '.git':
            return common.parent
    except (OSError, subprocess.SubprocessError, IndexError):
        pass
    return REPO


MAIN = _main_checkout()


def read_json(path: Path):
    try:
        return json.loads(path.read_text(encoding='utf-8-sig'))
    except FileNotFoundError:
        return None


CONFIG = read_json(KIT / 'local.config.json') or {}
OUT_ROOT = (MAIN / CONFIG['outRoot']).resolve() if CONFIG.get('outRoot') else MAIN / 'target' / 'device-acceptance'
DEFAULT_PORT = int(os.environ.get('RIVIU_CDP_PORT') or CONFIG.get('cdpPort') or 47182)


def default_db_path() -> Path:
    if CONFIG.get('dbPath'):
        return Path(os.path.expandvars(CONFIG['dbPath']))
    return Path(os.environ['APPDATA']) / 'riviu-managers-phone' / 'riviu.db'


def number_of(udid: str):
    return (CONFIG.get('numbers') or {}).get(udid, udid)


def check_label(label: str) -> str:
    if not LABEL_RE.match(label or ''):
        raise SystemExit(f'nhãn không hợp lệ (chỉ [A-Za-z0-9-]): {label!r}')
    return label


def check_port(port) -> int:
    n = int(port)
    if not 1024 <= n <= 65535:
        raise SystemExit(f'cổng CDP không hợp lệ: {port}')
    return n


def utc_now() -> str:
    return datetime.datetime.now(datetime.timezone.utc).isoformat()
