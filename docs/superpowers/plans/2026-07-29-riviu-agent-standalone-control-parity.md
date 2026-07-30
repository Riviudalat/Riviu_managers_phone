# Riviu Agent Standalone And Control Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tao Riviu-owned WDA 15.1.4 overlay, candidate build/probe tooling va cac behavior contract can de chay B0, Gate B va Gate C tren Mac/iPhone.

**Architecture:** Verify npm tarball da pin, extract vao ignored `target/riviu-agent/`, va apply mot patch series nho thay vi vendor WDA. Candidate protocol v2 tach khoi RT-MMO bang `X-Riviu-Token`, identity/health rieng va native sessionless gesture; Mac build/probe la authority cho standalone XCTest, signing va live parity.

**Tech Stack:** Python 3.9+, Pillow 11.3.0, Objective-C/XCTest, Appium WebDriverAgent 15.1.4, Xcode `xcodebuild`, pymobiledevice3 10.1.0, Rust workspace verification.

**Shared checkout note:** Repo dang o feature branch voi Gate A va runtime changes chua commit. Thuc thi tai cho, khong commit/stage/revert file ngoai scope va khong sua `docs/claude/**`.

---

## File map

**Create**

- `sidecars/wda/riviu-agent/README.md`: source ownership, build commands, gate state.
- `sidecars/wda/riviu-agent/baseline-lock.json`: baseline/tarball/source/patch digests.
- `sidecars/wda/riviu-agent/Config/RiviuAgent.xcconfig`: candidate identity/version/deployment target.
- `sidecars/wda/riviu-agent/AgentHost/README.md`: generated XCTest host boundary and B0 stop rule.
- `sidecars/wda/riviu-agent/AgentHost/patches/0004-signed-artifact-attestation.patch`: typed signed source/test/Xcode metadata.
- `sidecars/wda/riviu-agent/AgentRunner/patches/`: runner patch series if B0 evidence requires it.
- `sidecars/wda/riviu-agent/AgentServer/patches/0001-riviu-auth-health-status.patch`: auth and protocol identity.
- `sidecars/wda/riviu-agent/AgentInput/patches/0002-sessionless-native-gestures.patch`: native tap/swipe routes.
- `sidecars/wda/riviu-agent/AgentInput/patches/0005-clipboard-contract-hardening.patch`: exact clipboard runtime schemas.
- `sidecars/wda/riviu-agent/AgentStream/README.md`: baseline MJPEG contract.
- `sidecars/wda/riviu-agent/Contracts/control-v2.json`: route/body/auth/session contract.
- `sidecars/wda/riviu-agent/Contracts/native-input-v1.json`: direct event-record contract va evidence.
- `sidecars/wda/riviu-agent/Scripts/prepare.py`: deterministic source preparation.
- `sidecars/wda/riviu-agent/Scripts/build_candidate.py`: Mac build/sign/package/manifest.
- `sidecars/wda/riviu-agent/Scripts/probe_gate_bc.py`: B0/B/C live probe.
- `sidecars/wda/riviu-agent/requirements-mac.txt`: pinned live-probe dependencies.
- `sidecars/wda/riviu-agent/Tests/test_prepare.py`: preparation and integrity tests.
- `sidecars/wda/riviu-agent/Tests/test_contract.py`: contract invariant tests.
- `sidecars/wda/riviu-agent/Tests/test_build_candidate.py`: manifest/package tests.
- `sidecars/wda/riviu-agent/Tests/test_probe_gate_bc.py`: mock HTTP/MJPEG probe tests.
- `docs/re/riviu-agent/README.md`: generated evidence rules and current pending state.

**Modify**

- `.gitignore`: ignore generated source/build/artifacts under `target/riviu-agent/` only if not already covered.
- `AGENTS.md`: record Project 2 paths, protocol v2 boundary, B0 stop rule and Windows/Mac state.

**Never modify in this project**

- `sidecars/wda/RiviuAgent.ipa`
- `sidecars/wda/agent-manifest.json`
- `sidecars/wda/WebDriverAgent/**`
- `docs/claude/**`

---

### Task 1: Deterministic baseline preparation

**Files:**
- Create: `sidecars/wda/riviu-agent/baseline-lock.json`
- Create: `sidecars/wda/riviu-agent/Scripts/prepare.py`
- Create: `sidecars/wda/riviu-agent/Tests/test_prepare.py`

- [x] **Step 1: Write failing integrity and extraction tests**

Tests must cover valid synthetic tarball, SHA-512 mismatch, package version/git-head mismatch, absolute/traversal member, symlink member, patch hash mismatch, deterministic tree digest, and property-list changes affecting that digest.

```python
def test_rejects_parent_path_before_extracting(tmp_path):
    archive = make_tarball(tmp_path, {"package/../../outside": b"x"})
    with pytest_raises(PrepareError, "unsafe archive path"):
        prepare_source(archive, lock, output)
    assert not (tmp_path / "outside").exists()
```

- [x] **Step 2: Run tests and confirm RED**

Run:

```powershell
python -m unittest discover -s sidecars\wda\riviu-agent\Tests -p test_prepare.py -v
```

Expected: import/file-not-found failure because `prepare.py` does not exist.

- [x] **Step 3: Implement bounded verifier/extractor**

`prepare.py` must use `hashlib`, `tarfile`, `pathlib` and `subprocess.run([...])` without a shell. Enforce total extracted bytes/member count, regular files/directories only, `package/` prefix, safe resolved paths, exact SRI/version/gitHead, normalized `0644`/`0755` file modes, POSIX mode restoration, and atomic output replacement.

CLI:

```powershell
python sidecars\wda\riviu-agent\Scripts\prepare.py `
  --archive target\rtmmo-re\baselines\appium-webdriveragent-15.1.4.tgz `
  --output target\riviu-agent\source
```

- [x] **Step 4: Run focused tests and confirm GREEN**

Expected: all `test_prepare.py` cases PASS with no network and no production file changes.

- [x] **Step 5: Run prepare against the real verified cache**

Expected: mode-aware full build-input baseline digest equals `f40eadb1e1d9872ad5a0574a5146cdbf5e0d04768ccb1f1701b289d50e4ee8f8` before patches and `2ca158cde4b2307957670680a6cd136b6c360d6f175303f1d012f7488e82c4cc` after patches. It covers every regular file, including Xcode project files and property lists, plus the canonical executable bit. The earlier Gate A code-only digest used a narrower suffix set and is not the signed candidate source digest.

---

### Task 2: Versioned Project 2 control contract

**Files:**
- Create: `sidecars/wda/riviu-agent/Contracts/control-v2.json`
- Create: `sidecars/wda/riviu-agent/Tests/test_contract.py`

- [x] **Step 1: Write failing contract invariant tests**

Assert exact feature list, only `GET /status` auth exemption, protected health identity fields, required body types, session semantics, unique method/path pairs, candidate ports differing from oracle, and absence of `text`, `pushMedia`, `FARM_KEY`, `X-RT-Token`.

- [x] **Step 2: Run focused tests and confirm RED**

Expected: missing contract file.

- [x] **Step 3: Add `control-v2.json`**

Contract must define:

- protocol `2`, agent `0.1.0`, features `stream/tap/swipe/clipboard`;
- candidate defaults `8916/9094`, logical `375x667`;
- `RIVIU_AGENT_TOKEN` and `X-Riviu-Token` by name, never a value;
- all routes in design section 7 including clipboard request/read-back schema;
- response status expectations for missing/wrong/correct auth.

- [x] **Step 4: Run focused tests and confirm GREEN**

- [x] **Step 5: Validate JSON and scan forbidden feature/vendor strings**

Run the test directly plus `tools/rtmmo-re verify-redaction` after the README/report files exist.

---

### Task 3: Riviu auth, health and status overlay

**Files:**
- Create: `sidecars/wda/riviu-agent/AgentServer/patches/0001-riviu-auth-health-status.patch`
- Modify generated source through patch only:
  - `WebDriverAgentLib/Routing/FBWebServer.h`
  - `WebDriverAgentLib/Routing/FBWebServer.m`
  - `WebDriverAgentLib/Commands/FBSessionCommands.m`
  - `WebDriverAgentTests/UnitTests/FBRouteTests.m`
- Extend: `sidecars/wda/riviu-agent/Tests/test_prepare.py`

- [x] **Step 1: Add failing source invariant tests**

After prepare, assert patch-required symbols and test methods exist, startup rejects blank token, auth exemption is exact method+path, compare does not call plain `isEqualToString:`, response never interpolates the token, and protocol features omit text.

- [x] **Step 2: Run focused tests and confirm RED**

Expected: patch list/hash or source invariant fails.

- [x] **Step 3: Create patch on a disposable baseline copy**

Implementation requirements:

- env `RIVIU_AGENT_TOKEN`, header `X-Riviu-Token`;
- constant-time UTF-8 byte comparison including equal-length check;
- only `GET /status` exempt;
- auth check nam trong `FBHTTPConnection` truoc route dispatch, bao gom server-key routes;
- missing/suppressed startup token aborts before bind;
- 401 W3C JSON without secret echo;
- protected `GET /riviu/health` route;
- `value.riviuAgent` added to WDA status;
- CORS allow-header includes `X-Riviu-Token`;
- Objective-C unit tests for auth truth table, exact exemption and identity payload.

- [x] **Step 4: Add patch SHA-256 to `baseline-lock.json` and prepare source**

Expected: patch applies with zero fuzz to byte-verified 15.1.4 and generated tree contains Xcode tests.

- [x] **Step 5: Run Windows source tests**

Expected: GREEN. Record Xcode unit tests as `PENDING_MAC_DEVICE`, not PASS.

---

### Task 4: Direct native-input contract and overlay

**Files:**
- Create: `sidecars/wda/riviu-agent/AgentInput/patches/0002-sessionless-native-gestures.patch`
- Create: `sidecars/wda/riviu-agent/Contracts/native-input-v1.json`
- Modify generated source through patch only:
  - `WebDriverAgentLib/Commands/FBElementCommands.m`
  - `WebDriverAgentTests/UnitTests/FBRouteTests.m`
- Extend: `sidecars/wda/riviu-agent/Tests/test_prepare.py`

- [x] **Step 1: Add failing native-input contract tests**

Assert `/wda/tap` and `/wda/swipe` have sessionless routes with distinct Riviu
handlers, finite number validation and swipe delay range `[0,5]`. Contract must
cite oracle selectors `handleHCTap:`, `handleHCSwipe:`,
`hcEmit:offsets:tag:` plus the baseline direct synthesizer APIs. It must forbid
W3C actions, `XCUICoordinate` gesture methods,
`pressForDuration:thenDragToCoordinate:` and `fb_waitUntilStable`.

- [x] **Step 2: Run focused tests and confirm RED**

- [x] **Step 3: Add `native-input-v1.json`, then implement minimal direct event-record handlers**

Tap accepts `{x,y}` and creates an `XCPointerEventPath` down/up at that raw screen
point. Swipe accepts `{fromX,fromY,toX,toY,delay}`, creates down/move/up offsets,
wraps the path in `XCSynthesizedEventRecord`, and calls
`FBXCTestDaemonsProxy synthesizeEventWithRecord:timeout:error:`. Device orientation
is mapped locally from `XCUIDevice`; handler does not query active applications or
resolve accessibility hierarchy, hit point, or quiescence. Synthesis has a
five-second deadline and validates callback error plus Boolean result. Non-object,
extra-key, Boolean, string, NaN, and Infinity inputs return invalid-argument.

- [x] **Step 4: Extend Xcode route unit tests**

Verify sessionless route paths and `requiresSession == NO`; live side effect remains Gate C.

Source invariant tests must reject any input patch containing `/actions`,
`XCUICoordinate`, `pressForDuration:thenDragToCoordinate:` or
`fb_waitUntilStable` in the Riviu handlers.

- [x] **Step 5: Hash patch, prepare again and confirm GREEN**

---

### Task 5: Candidate config, build, signing and manifest

**Files:**
- Create: `sidecars/wda/riviu-agent/Config/RiviuAgent.xcconfig`
- Create: `sidecars/wda/riviu-agent/Scripts/build_candidate.py`
- Create: `sidecars/wda/riviu-agent/Tests/test_build_candidate.py`

- [x] **Step 1: Write failing pure-function tests**

Cover xcode version parsing, bundle identity capture, source/xcconfig attestation,
candidate manifest generation, SHA-256, exact feature list, safe relative IPA
path, deterministic packaging order, Xcode 26 runtime closure/re-sign ordering,
and rejection of missing team/device/toolchain/dependency.

- [x] **Step 2: Run focused tests and confirm RED**

- [x] **Step 3: Implement build orchestration**

The script must:

1. call `prepare.py`;
2. require macOS, `xcodebuild`, `security`, `codesign`, `xcrun`;
3. run the Objective-C `UnitTests` target and stop on any failure;
4. run `xcodebuild build-for-testing` with list args, explicit team, destination, derived path and xcconfig;
5. locate generated `WebDriverAgentRunner-Runner.app`;
6. verify the locked xcconfig digest is embedded in the signed XCTest plist;
7. for Xcode >=26, enforce all four Testing runtimes, re-sign nested code and deep/strict verify again;
8. inspect actual plist/signature instead of assuming bundle/signer;
9. package to `target/riviu-agent/artifacts/<version>/RiviuAgent-candidate.ipa` atomically;
10. write `candidate-manifest.json` with `gateStatus: PENDING_MAC_DEVICE`, `objectiveCUnitTests: PASS`, source/xcconfig digests, and no secret.

- [x] **Step 4: Run pure tests and confirm GREEN on Windows**

- [x] **Step 5: Add Mac command without running it on Windows**

```bash
python3 sidecars/wda/riviu-agent/Scripts/build_candidate.py \
  --udid "$UDID" --team-id "$TEAM_ID"
```

Expected on this checkpoint: command documented; result remains pending until executed on Mac.

---

### Task 6: B0 and Gate B-C probe harness

**Files:**
- Create: `sidecars/wda/riviu-agent/Scripts/probe_gate_bc.py`
- Create: `sidecars/wda/riviu-agent/Tests/test_probe_gate_bc.py`
- Create: `docs/re/riviu-agent/README.md`

- [x] **Step 1: Write failing mock transport tests**

Use local HTTP/TCP fixtures. Cover control and MJPEG auth 401/401/200, new
session ID, ordering `launch -> health -> foreground -> session -> MJPEG`, real
JPEG decode, causal visual evidence, clipboard Unicode byte equality, exact
Unicode read-back, no token/path/UDID in reports, cleanup, and failure preventing
PASS.

- [x] **Step 2: Run focused tests and confirm RED**

- [x] **Step 3: Implement transport-independent probe state machine**

Keep HTTP, MJPEG parser, evidence recorder and device adapter separate. Requests use per-request deadlines and `Connection: close`; token is injected only as an HTTP/DVT environment value and custom `repr`/report output omits it. Preflight scans the manifest, decompressed IPA, prepared source, separately locked xcconfig, and argv; the xcconfig digest must still match the manifest.

- [x] **Step 4: Implement Mac device adapter**

Use pymobiledevice3 10.1.0 DVT `ProcessControl.launch(environment=...)`, exact
bundle terminate, bounded usbmux relays on candidate ports, Settings foreground,
and strict cleanup. Load only a manifest-bound candidate: verify source/xcconfig/IPA hashes,
fresh-install, and compare installed identity before launch. Never put token in
child argv. Before clipboard set/get, foreground the running candidate with
`kill_existing=false`, require the same PID before/after, then verify the same
bundle/PID through `/wda/activeAppInfo`. Every cold launch must prove the prior
process is absent, both ports are closed, and DVT reports a new stable PID. Do not launch
desktop/harness/oracle concurrently.

Recheck the candidate PID after protected health, fresh session, and the first
JPEG before counting a cold-launch witness. The next cycle or final cleanup must
terminate that same PID. Run the Rust evidence verifier with `RIVIU_AGENT_TOKEN`
removed from its child environment.

- [x] **Step 5: Implement atomic evidence output**

Outputs:

- `docs/re/riviu-agent/candidate-probes.json`
- `docs/re/riviu-agent/gate-b.md`
- `docs/re/riviu-agent/gate-c.md`

The versioned Windows checkpoint says `PENDING_MAC_DEVICE`; fixture-generated
reports say `FIXTURE_ONLY`. Live generation permits PASS only for
`LIVE_MAC_DEVICE` with fixed thresholds from design section 10; caller-supplied
lower values are rejected. All three staged evidence files must pass the Rust
redaction verifier before publication.
Publication is transactional: if any destination replace fails, JSON and both
Markdown documents are restored as one set.

- [x] **Step 6: Run mock tests and redaction verifier**

---

### Task 7: Documentation and successor invariants

**Files:**
- Create: `sidecars/wda/riviu-agent/README.md`
- Create: `sidecars/wda/riviu-agent/AgentHost/README.md`
- Create: `sidecars/wda/riviu-agent/AgentStream/README.md`
- Modify: `AGENTS.md`

- [x] **Step 1: Document reproducible Windows prepare/test commands**

- [x] **Step 2: Document Mac build/B0/Gate B-C commands and exclusive-device rule**

- [x] **Step 3: Update `AGENTS.md` in the same change**

Record:

- Project 2 source path and pinned overlay architecture;
- protocol v2 env/header/ports/features;
- B0 stop rule: HTTP alone is not automation readiness;
- current `PENDING_MAC_DEVICE` state;
- production RT-MMO artifact/manifest remain unchanged;
- do-not-repeat: no selector-only porting, no `text` advertisement, no product switch.

- [x] **Step 4: Run placeholder/contradiction/secret scan**

No unresolved placeholder marker, secret literal, device ID, user-home path or
false PASS in versioned Project 2 artifacts.

---

### Task 8: Full verification and review

**Files:** all Project 2 files plus unchanged production hashes.

- [x] **Step 1: Run Python suite**

```powershell
python -m unittest discover -s sidecars\wda\riviu-agent\Tests -p "test_*.py" -v
```

- [x] **Step 2: Prepare twice and compare output digests**

Both generated source tree digests must match. Production IPA and manifest hashes must remain byte-identical to pre-Project-2 values.

- [x] **Step 3: Run existing verification**

```powershell
cargo test --workspace -- --test-threads=1
cargo clippy -p rtmmo-re --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

- [x] **Step 4: Run report redaction and forbidden-string scans**

- [x] **Step 5: Request spec-compliance review, then code-quality review**

Resolve every finding before closing the Windows checkpoint.

- [x] **Step 6: State the exact gate result**

Allowed result on Windows: source/tooling tests PASS, B0/Gate B/Gate C
`PENDING_MAC_DEVICE`. Do not state that the candidate IPA is buildable, signed,
plain-launching or control-parity until the corresponding Mac/device evidence
exists.
