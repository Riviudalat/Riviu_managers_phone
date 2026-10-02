"""Continue only from a clean account-discovery receipt; one pre-Post rehearsal."""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path
import uuid

parser = argparse.ArgumentParser()
parser.add_argument('--discovery-activation', required=True)
parser.add_argument('--discovery-request', required=True)
parser.add_argument('--serial', choices=['ce021712aaf9533405', 'ce031713cd92d90701', 'ce041714a3159a0801', 'ce041714d44264230d', 'ce06171631ca38e00d', 'ce061716d811302c04', 'ce071827e0659c1605'], default='ce021712aaf9533405')
args = parser.parse_args()
activation = str(uuid.UUID(args.discovery_activation))
request = str(uuid.UUID(args.discovery_request))
root = Path.cwd()
live = root / 'target/no-public-live'
serial = args.serial
package = 'com.zhiliaoapp.musically'
status = json.loads((live / f'{activation}.run/run-{request}/status.json').read_text())
if status['activationId'] != activation or status['requestId'] != request or status['udid'] != serial or status['kind'] != 'inspect' or status['state'] != 'preparedAndCleaned' or status['publicEffectsAllowed'] is not False:
    raise SystemExit('Discovery was not clean; no preparation activation')
outcome = status['outcome']
if outcome['package'] != package or outcome['version'] != '45.7.3' or outcome['locale'] != 'en' or outcome['typed'] is not False or outcome['publicEffectDispatched'] is not False:
    raise SystemExit('Discovery tuple/boundary differs')
account = outcome['account']
if not isinstance(account, str) or not account.strip():
    raise SystemExit('Observed account missing')
discovery_process = json.loads((live / f'{activation}.process.json').read_text())
exe = root / 'target/debug/riviu-managers-phone.exe'
if hashlib.sha256(exe.read_bytes()).hexdigest() != discovery_process['exeSha256']:
    raise SystemExit('Executable changed since discovery; no preparation')
hashes = json.loads((root / 'target/no-public-fixture/input-hashes.json').read_text())
source = root / 'target/no-public-fixture/rehearsal-photo'
if any(hashlib.sha256((source / name).read_bytes()).hexdigest() != digest for name, digest in hashes.items()):
    raise SystemExit('Approved input hashes changed')
subprocess.run(['node', 'tools/no_public_harness.mjs', '--port', '9377', '--activation', activation, '--mode', 'shutdown', '--report', f'target/no-public-live/{activation}.shutdown'], check=True)
# No launch until the identity inventory confirms no desktop process.
import time
for _ in range(20):
    inventory = subprocess.run(['python', 'tools/no_public_process.py'], check=True, capture_output=True, text=True)
    if json.loads(inventory.stdout) == []:
        break
    time.sleep(0.5)
else:
    raise SystemExit('Prior desktop exit unproved; no second controller')
scope_path = live / f'{uuid.uuid4()}.approved-publish.json'
subprocess.run([str(root/'target/debug/examples/no_public_scope.exe'), str(source), serial, package, account, str(scope_path), '20261002'], check=True)
scope = json.loads(scope_path.read_text())
scope['deviceScopes'][0]['allowInstalledRunner'] = True
scope_path.write_text(json.dumps(scope, indent=2), encoding='utf8')
launched = subprocess.run(['python', 'tools/no_public_launch.py', '--exe', 'target/debug/riviu-managers-phone.exe', '--serial', serial, '--package', package, '--port', '9377', '--prepared-scope', str(scope_path)], check=True, capture_output=True, text=True)
record = json.loads(launched.stdout)
if record['exeSha256'] != discovery_process['exeSha256']:
    raise SystemExit('Launch executable provenance changed; no Publish command')
print(json.dumps(record), flush=True)
request = str(uuid.uuid4())
subprocess.run(['node', 'tools/no_public_harness.mjs', '--port', '9377', '--activation', record['activationId'], '--mode', 'publish', '--udid', serial, '--request', request, '--wait', 'true', '--report', f"target/no-public-live/{record['activationId']}.publish"], check=True)
final = json.loads((live / f"{record['activationId']}.run/run-{request}/status.json").read_text())
if final['state'] != 'preparedAndCleaned':
    raise SystemExit('Publish boundary not proved clean; retain scratch and fence')
subprocess.run(['node', 'tools/no_public_harness.mjs', '--port', '9377', '--activation', record['activationId'], '--mode', 'shutdown', '--report', f"target/no-public-live/{record['activationId']}.shutdown"], check=True)
