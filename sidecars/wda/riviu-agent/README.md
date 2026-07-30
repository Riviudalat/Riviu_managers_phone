# Riviu Agent Candidate

This directory owns the reproducible Project 2 overlay for the standalone Riviu
Agent candidate. It does not vendor a generated WDA tree. `Scripts/prepare.py`
verifies the pinned Appium WebDriverAgent 15.1.4 tarball and applies the ordered,
hashed patch series into ignored `target/riviu-agent/source/`.

Current gate state: `PENDING_MAC_DEVICE`. The Windows source, contract, packaging,
and fixture-probe suites pass, but no candidate IPA is accepted until a Mac/iPhone
proves plain-launch XCTest automation, protected health, fresh session before
MJPEG, direct gesture side effects, clipboard read-back, and stream stability.

## Layout

- `baseline-lock.json`: package, archive, source, and patch digests.
- `AgentServer/patches/`: connection-boundary auth and protocol identity.
- `AgentInput/patches/`: direct sessionless event-record tap/swipe.
- `AgentHost/patches/`: typed signed metadata in the XCTest runner plist.
- `Contracts/`: protocol v2 and native input contracts.
- `Config/RiviuAgent.xcconfig`: candidate-only build identity.
- `Scripts/`: deterministic prepare, Mac build, and Gate B/C probe.
- `requirements-mac.txt`: exact live-probe Python dependencies.
- `Tests/`: Windows-safe integrity, contract, packaging, and fixture tests.

## Windows Verification

```powershell
python sidecars\wda\riviu-agent\Scripts\prepare.py `
  --archive target\rtmmo-re\baselines\appium-webdriveragent-15.1.4.tgz `
  --output target\riviu-agent\source

python -m unittest discover `
  -s sidecars\wda\riviu-agent\Tests `
  -p "test_*.py" -v
```

The prepared baseline digest must be
`f40eadb1e1d9872ad5a0574a5146cdbf5e0d04768ccb1f1701b289d50e4ee8f8`.
The locked post-patch digest is
`2ca158cde4b2307957670680a6cd136b6c360d6f175303f1d012f7488e82c4cc`.
Preparation rejects any other result before replacing the generated source tree.
Both digests cover every regular file, including Xcode project/build inputs and
property lists, plus each file's canonical `0644`/`0755` mode, rather than only
Objective-C source suffixes. POSIX extraction restores the executable bit needed
by the runner scheme. The candidate xcconfig is separately locked at SHA-256
`2bed5a711927df27a86b2e2f7237bad99406b3cbbf5fccb09f8ce03fc58f53ae`.

## Mac Build And Live Probe

Run with the desktop app, nurture harness, 3uTools, and oracle agent stopped.

```bash
python3 -m pip install -r sidecars/wda/riviu-agent/requirements-mac.txt

python3 sidecars/wda/riviu-agent/Scripts/build_candidate.py \
  --udid "$UDID" --team-id "$TEAM_ID"

RIVIU_AGENT_TOKEN="$(openssl rand -hex 32)" \
python3 sidecars/wda/riviu-agent/Scripts/probe_gate_bc.py \
  --udid "$UDID" \
  --manifest target/riviu-agent/artifacts/0.1.0/candidate-manifest.json
```

The build script runs the Objective-C `UnitTests` target before the runner build,
then rehashes the complete source tree after unit tests and after the runner build.
It records that result, actual signature identity, Xcode version, source and
xcconfig digests, and IPA digest in the manifest. The probe verifies that chain,
uninstalls the exact
candidate bundle, installs the attested IPA, and compares installed metadata before
any launch. It reads the runtime token only from the environment and passes it to
`ProcessControl.launch(environment=...)`; it is not placed in child argv or evidence.
Patch `0004` declares six attestation keys directly in the embedded
`WebDriverAgentRunner.xctest/Info.plist`; protocol v2 is a plist integer and the five
string values expand from explicit build settings. Custom user-defined
`INFOPLIST_KEY_RiviuAgent*` settings are not part of this chain.
Patch `0005` makes the two clipboard handlers reject missing, mistyped, or extra
request fields so runtime behavior matches `control-v2.json`.
For Xcode 26 and newer, packaging requires `Testing.framework`,
`_Testing_Foundation.framework`, `lib_TestingInterop.dylib`, and
`libXCTestSwiftSupport.dylib`; the build re-signs nested code and deep-verifies the
outer app after completing that runtime set.

## Boundaries

- Candidate control/MJPEG device ports are `8916` and `9094`.
- Only exact `GET /status` is auth-exempt. Every other route requires
  `X-Riviu-Token`; the loopback-only MJPEG socket requires the same header.
- Project 2 advertises only `stream`, `tap`, `swipe`, and `clipboard`.
- Native handlers use direct XCTest event records. They do not use W3C actions,
  coordinate gesture helpers, accessibility queries, or quiescence waits. Device
  orientation is mapped locally and event synthesis has a five-second deadline
  that checks both the callback error and Boolean result.
- The control relay opens before health/session. The MJPEG relay and reader open
  only after the fresh session exists.
- Live acceptance thresholds are fixed at five cold launches, 50 causal taps,
  20 causal swipes, and 300 seconds of decoded-JPEG stream observation. Fixture
  runs are always labeled `FIXTURE_ONLY`.
- Every cold launch records `old PID gone -> ports closed -> new stable PID`. The
  PID is looked up again after protected health, fresh session, and the first JPEG;
  the next cycle or final cleanup must terminate that same PID. Witnesses must be
  distinct across all five cycles. Stream control checks also
  enforce a 5-second cycle deadline, 5.5-second completion gap, and 0.5-second
  maximum schedule lateness instead of accepting a catch-up count.
- Project 2 treats any control/session fault as Gate C failure. Desktop soft/hard
  recovery budgets are tested only when the candidate is integrated in Project 4;
  this probe permits at most one MJPEG reader reconnect.
- Clipboard byte equality is measured only after the candidate is moved to the
  foreground with `kill_existing=false`, its PID is proven unchanged, and
  `/wda/activeAppInfo` returns the candidate bundle with that same PID. An HTTP
  success returned while another app is foreground is not clipboard evidence.
- The token preflight scans the raw manifest, every decompressed IPA entry, the
  reconstructed locked source, the separately locked xcconfig, and argv. It
  rehashes the xcconfig against the manifest before recording
  `xcconfigTokenScanClean`. Gate finalization separately checks guarded process
  output and the serialized report; the Rust verifier subprocess receives an
  environment with `RIVIU_AGENT_TOKEN` removed. Evidence JSON and both gate
  documents publish as one rollback transaction.
- `sidecars/wda/RiviuAgent.ipa`, `sidecars/wda/agent-manifest.json`, and
  `sidecars/wda/WebDriverAgent/` are outside this candidate build and stay intact.
