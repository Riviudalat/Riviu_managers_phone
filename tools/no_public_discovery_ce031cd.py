"""One alternate approved discovery only; never replay a failed request."""
import json
import subprocess
import uuid

serial = 'ce061716d811302c04'
record = json.loads(subprocess.run([
    'python', 'tools/no_public_launch.py', '--exe', 'target/debug/riviu-managers-phone.exe',
    '--serial', serial, '--package', 'com.zhiliaoapp.musically', '--port', '9377',
    '--allow-warm-launch', '--allow-installed-runner',
], check=True, capture_output=True, text=True).stdout)
request = str(uuid.uuid4())
print(json.dumps({**record, 'requestId': request}), flush=True)
subprocess.run([
    'node', 'tools/no_public_harness.mjs', '--port', '9377', '--activation', record['activationId'],
    '--mode', 'inspect', '--udid', serial, '--request', request, '--wait', 'true',
    '--report', f"target/no-public-live/{record['activationId']}.inspect",
], check=True)
