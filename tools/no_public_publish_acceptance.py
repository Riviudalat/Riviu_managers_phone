"""One approved native discovery, followed by exactly one pre-Post Publish run."""
import json
import subprocess
import uuid

serial = 'ce071827e0659c1605'
record = json.loads(subprocess.run([
    'python', 'tools/no_public_launch.py', '--exe', 'target/debug/riviu-managers-phone.exe',
    '--serial', serial, '--package', 'com.zhiliaoapp.musically', '--port', '9377',
    '--allow-warm-launch', '--allow-installed-runner',
], check=True, capture_output=True, text=True).stdout)
request = str(uuid.uuid4())
print(json.dumps({**record, 'discoveryRequestId': request}), flush=True)
subprocess.run([
    'node', 'tools/no_public_harness.mjs', '--port', '9377', '--activation', record['activationId'],
    '--mode', 'inspect', '--udid', serial, '--request', request, '--wait', 'true',
    '--report', f"target/no-public-live/{record['activationId']}.inspect",
], check=True)
# The continuation independently checks clean receipt, input hash, exact account
# and tuple, drains the old activation, and refuses a second desktop controller.
subprocess.run([
    'python', 'tools/no_public_publish_ce021.py', '--serial', serial,
    '--discovery-activation', record['activationId'], '--discovery-request', request,
], check=True)
