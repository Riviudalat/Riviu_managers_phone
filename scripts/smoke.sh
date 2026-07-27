#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# shellcheck disable=SC1091
source "$HOME/.cargo/env" 2>/dev/null || true

echo "== script-engine tests =="
cargo test -p riviu-script-engine --quiet

echo "== rust check =="
cargo check -p riviu-managers-phone --quiet

echo "== frontend build =="
(cd apps/desktop && npm run build)

echo "== sidecar ping =="
python3 sidecars/pymobiledevice3/riviu_pmd.py ping || true
python3 sidecars/signer/riviu_signer.py sign-install-wda \
  --udid SMOKE --apple-id smoke@example.com --password x --wda /tmp/wda.ipa >/dev/null

echo "== bundle resources present =="
test -f sidecars/pymobiledevice3/riviu_pmd.py
test -f sidecars/signer/riviu_signer.py
test -f apps/desktop/src-tauri/tauri.conf.json

echo "Smoke OK"
