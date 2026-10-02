"""One approved alternate canary: reviewed setup reconcile then one discovery request."""
import json
import subprocess
import uuid
from pathlib import Path

root = Path.cwd()
serial = 'ce021712aaf9533405'
launched = subprocess.run([
    'python', 'tools/no_public_launch.py', '--exe', 'target/debug/riviu-managers-phone.exe',
    '--serial', serial, '--package', 'com.zhiliaoapp.musically', '--port', '9377',
    '--allow-warm-launch', '--allow-installed-runner',
], check=True, capture_output=True, text=True)
record = json.loads(launched.stdout)
activation = record['activationId']
print(json.dumps(record), flush=True)
base = ['node', 'tools/no_public_harness.mjs', '--port', '9377', '--activation', activation]
subprocess.run(base + [
    '--mode', 'reconcile-setup', '--udid', serial,
    '--prior-activation', '025b7131-e623-4d72-9dc0-1972f04a2352',
    '--prior-request', '93a38bdc-0327-4fc6-b78b-7c09743841e4',
    '--report', f'target/no-public-live/{activation}.reconcile',
], check=True)
request = str(uuid.uuid4())
subprocess.run(base + [
    '--mode', 'inspect', '--udid', serial, '--request', request, '--wait', 'true',
    '--report', f'target/no-public-live/{activation}.inspect',
], check=True)
print(json.dumps({'activationId': activation, 'requestId': request, 'state': 'discoveryReceiptAvailable', 'publicEffectsAllowed': False}), flush=True)
