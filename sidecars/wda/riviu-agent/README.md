# Riviu Agent Candidate

This directory owns the reproducible Project 2 overlay for the standalone Riviu
Agent candidate. It does not vendor a generated WDA tree. `Scripts/prepare.py`
verifies the pinned Appium WebDriverAgent 15.1.4 tarball and applies the ordered,
hashed patch series into ignored `target/riviu-agent/source/`.

Gate results are historical and artifact-bound, not a current readiness flag.
`docs/re/riviu-agent/candidate-probes.json` records the earlier device2 four-capability
PASS. Media candidate reports and the current manifest have separate FAIL/PENDING
states. Compare exact artifact/source/device/date tuples before interpreting a gate;
none certifies a newly signed IPA or another phone. The current lock/manifest are
machine-readable owners, not the digest prose of an old checkpoint.

## Layout

- `baseline-lock.json`: package, archive, source, and patch digests.
- `AgentServer/patches/`: connection-boundary auth and protocol identity.
- `AgentInput/patches/`: direct sessionless event-record tap/swipe.
- `AgentHost/patches/`: typed signed metadata in the XCTest runner plist.
- `Contracts/`: protocol v2 and native input contracts.
- `Contracts/media-v1.json`: native pushMedia prepare/readback contract. It is
  opt-in in `build_candidate.py --media-capable` and remains hidden from the
  default artifact until the TikTok post gates pass.
- `Config/RiviuAgent.xcconfig`: candidate-only build identity.
- `Scripts/`: deterministic prepare, Mac build, Gate B/C probe, and real comment probe.
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
The locked post-patch digest and ordered patch set are in `baseline-lock.json`;
`Scripts/prepare.py` verifies them. Do not copy a historical digest into the lock.
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
  --manifest target/riviu-agent/artifacts/0.1.0/candidate-manifest.json \
  --wait-for-trust
```

`--wait-for-trust` pauses after the fresh install so the Apple Development
profile can be approved on the iPhone before the first DVT launch. The flag is
optional and does not change live thresholds; omit it only for an intentionally
non-interactive run against an already trusted fresh install.

For repeated functional checks when the candidate is already trusted, use
`--reuse-trusted-install`. It performs an installation-proxy `Upgrade` without
uninstalling the bundle, preserving the device approval. Its report is marked
`SUPPLEMENTAL_MAC_DEVICE`/`SUPPLEMENTAL_ONLY`; the official Gate B/C path still
requires the default fresh-install command above.

```bash
RIVIU_AGENT_TOKEN="$(openssl rand -hex 32)" \
python3 sidecars/wda/riviu-agent/Scripts/probe_gate_bc.py \
  --udid "$UDID" \
  --manifest target/riviu-agent/artifacts/0.1.0/candidate-manifest.json \
  --reuse-trusted-install
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
The generated XCTest host is branded immediately before the final signing pass:
its outer `Info.plist` contains `CFBundleDisplayName=Riviu Agent`,
`CFBundleName=Riviu Agent`, and `CFBundleIconName=AppIcon`. Packaging fails if the
compiled `Assets.car` or rendered `AppIcon*.png` resources are missing. The
locked `apps/desktop/public/logo.jpg` and WDA `AppIcon.appiconset` is compiled into the package,
so the Home Screen name and orange-R logo are part of the verified candidate IPA.
Apple development profiles are UDID-bound: build and install one candidate IPA
per connected device (for example, use a distinct `--artifact-version` for the
second device); reusing one signed IPA on another UDID is rejected by installd.

## Real TikTok Comment Probe

This is an isolated-device diagnostic with a **real public Send**, not a UI smoke
or a production-controller substitute. Read WDA safety and obtain exact device,
content and effect authorization first. No sample sentence or real serial is supplied.
Do not run alongside a controller owning the same phone.

Execution requires `--udid`, `--comment-text`, `--candidate-manifest`, fresh absolute
`--output` and `--frames-dir`, `--isolated-device-confirmed`, and `--allow-send`.
An exclusive intent is saved before device work. Existing/partial intent refuses
execution. A timeout/crash is not permission to change directories and Send again.

After execution inspect the saved frames. Exit2 / `PENDING_OPERATOR` is not a live
PASS. Confirm only the existing evidence, offline:

```bash
python3 sidecars/wda/riviu-agent/Scripts/probe_tiktok_comment.py   --confirm-evidence "$EVIDENCE_ABSOLUTE_PATH" --run-id "$RECORDED_RUN_ID"   --operator-confirmed-comment-visible
```

Confirmation checks run/intent/frame hashes and calls no driver, sidecar or network.
It returns `OPERATOR_CONFIRMED`, **not artifact qualification**. The legacy probe
cannot attest that the relay's running agent/device matches the local IPA supplied
by the caller. New promotion through `promote_text_candidate.py` therefore refuses
this standalone evidence until runtime artifact/device attestation is implemented
and separately reviewed. Historical evidence is read-only, not upgraded implicitly.
Do not modify IPA/features/hash to bypass this missing gate.

## Boundaries

- Candidate control/MJPEG device ports are `8916` and `9094`.
- Only exact `GET /status` is auth-exempt. Every other route requires
  `X-Riviu-Token`; the loopback-only MJPEG socket requires the same header.
- The default Project 2 candidate advertises only `stream`, `tap`, `swipe`, and
  `clipboard`; the promoted Full artifact adds `text` only after the live probe.
- Desktop media staging is implemented through HouseArrest/AFC with a manifest
  and size+SHA-256 readback. The media-capable candidate now exposes protected
  `POST/GET /riviu/media/v1/prepare` for native manifest/file verification; the
  default artifact still hides `pushMedia`, and preparation alone never counts
  as a TikTok post.
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
