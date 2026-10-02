"""Record strict-rehearsal unsupported capabilities without touching a device."""
import json
from pathlib import Path

root = Path('target/no-public-live').resolve()
root.mkdir(parents=True, exist_ok=True)
record = {
    'publicEffectsAllowed': False,
    'liveActionsPerformed': False,
    'interaction': {
        'state': 'capabilityBlocked',
        'reason': 'Canonical Copy-link proof requires authenticated helper clipboard and temporary IME; strict diagnostic does not provision token or switch IME.',
        'draftTyped': False,
        'sendAttempted': False,
    },
    'publish': {
        'state': 'capabilityBlocked',
        'reason': 'Native media helper token handoff/setup is not enabled by strict discovery policy.',
        'mediaTransferred': False,
        'postAttempted': False,
    },
}
path = root / 'strict-capability-blockers.json'
with path.open('x', encoding='utf8') as file:
    json.dump(record, file, ensure_ascii=False, indent=2)
print(str(path))
