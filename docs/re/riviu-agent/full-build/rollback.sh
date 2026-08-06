#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../../.." && pwd)
ORACLE_IPA="$ROOT/target/riviu-agent/rollback/production-oracle/RiviuAgent.ipa"
ORACLE_MANIFEST="$ROOT/target/riviu-agent/rollback/production-oracle/agent-manifest.json"

test "$(shasum -a 256 "$ORACLE_IPA" | awk '{print $1}')" = \
  8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea
test "$(shasum -a 256 "$ORACLE_MANIFEST" | awk '{print $1}')" = \
  e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a

if [ "${1:-}" = "--verify-only" ]; then
  echo "Production oracle checksum verification PASS"
  exit 0
fi
if [ "$#" -ne 1 ]; then
  echo "usage: $0 <UDID>" >&2
  exit 2
fi

python3 "$ROOT/sidecars/pymobiledevice3/riviu_pmd.py" install \
  --udid "$1" --ipa "$ORACLE_IPA"
echo "Production oracle installed on $1"
