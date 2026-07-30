# Riviu Flow V2 Acceptance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove Flow V2 release 1 in mock, browser, packaged desktop, and the exact Mac/iPhone live tuple without changing production Agent artifacts.

**Architecture:** Automated mock tests prove deterministic failure/recovery paths; Playwright proves layout/workflow; a dedicated Mac harness records device evidence and cleanup. Qualification is tuple-scoped and publishes only redacted, transactional reports.

**Tech Stack:** Cargo, Vitest, Playwright, Tauri 2, Rust live harness, Python redaction tooling, macOS/Xcode, iPhone USB.

---

### Task 1: Add End-To-End Mock Flow Coverage

**Files:**
- Create: `apps/desktop/src-tauri/src/bin/live_flow_test.rs`
- Create: `crates/core/tests/flow_release_one.rs`
- Create: `docs/fixtures/flow-release-one.json`
- Modify: `apps/desktop/src-tauri/Cargo.toml`

- [ ] **Step 1: Write the failing mock release test**

Build a fixed revision: Start -> Launch Settings -> Wait 10 ms -> Swipe ->
Screenshot -> Terminate Settings -> Home -> End. Run it on two mock UDIDs. Assert
independent device attempts, exact plan hash, stream budget <= configured limit,
screenshot hashes, verified `ProcessAbsent` evidence, no WDA screenshot call, and
both contexts released.

The integration test loads the checked-in fixture rather than rebuilding a graph
with random UUIDs:

```rust
#[tokio::test]
async fn release_one_fixture_runs_two_devices_without_shared_attempts() {
    let document: FlowDocumentV2 = serde_json::from_str(include_str!(
        "../../../docs/fixtures/flow-release-one.json"
    )).expect("fixture JSON");
    let snapshot = mock_snapshot();
    let profile_id = qualified_geometry_profile_id(&snapshot).expect("profile ID");
    let swipe = document.nodes.iter()
        .find(|node| node.kind == ActionKind::Swipe)
        .expect("fixture Swipe");
    let from: ImageCoordinateTarget = serde_json::from_value(
        swipe.config.get("from").cloned().expect("Swipe.from")
    ).expect("typed Swipe.from");
    let to: ImageCoordinateTarget = serde_json::from_value(
        swipe.config.get("to").cloned().expect("Swipe.to")
    ).expect("typed Swipe.to");
    assert_eq!(from.profile_id, profile_id);
    assert_eq!(to.profile_id, profile_id);

    let compiled = compile_flow(&document, &release_one_catalog()).expect("compile");
    let fixture = MockFlowRuntimeFixture::new(
        &["MOCK-IPHONE-01", "MOCK-IPHONE-02"],
        snapshot,
    );
    fixture.database.save_flow_revision(
        None,
        &document,
        &compiled.plan,
        &compiled.sha256,
    ).expect("persist immutable revision");
    let revision = fixture.database.get_flow_revision(document.id, Some(document.revision))
        .expect("reload revision")
        .expect("saved revision");
    assert_eq!(revision.plan_hash, compiled.sha256);
    let run = fixture.runtime.enqueue(
        revision,
        FlowTargetSelection::Selected {
            udids: vec!["MOCK-IPHONE-01".into(), "MOCK-IPHONE-02".into()],
        },
    ).await.expect("enqueue");
    let detail = fixture.wait_terminal(run.id).await.expect("terminal run");
    assert_eq!(detail.device_runs.len(), 2);
    assert!(detail.device_runs.iter().all(|device| device.state.is_success()));
    assert_eq!(detail.artifacts.len(), 2);
    let terminate_attempts: Vec<_> = detail.attempts.iter()
        .filter(|attempt| attempt.action_kind == ActionKind::TerminateApp)
        .filter(|attempt| attempt.state == FlowAttemptState::Succeeded)
        .collect();
    assert_eq!(terminate_attempts.len(), 2);
    assert!(terminate_attempts.iter().all(|attempt| {
        let evidence = attempt.evidence_result.as_ref().expect("Terminate evidence");
        evidence.get("kind").and_then(serde_json::Value::as_str) == Some("processAbsent")
            && evidence.get("matched").and_then(serde_json::Value::as_bool) == Some(true)
    }));
    assert_eq!(fixture.driver.wda_screenshot_calls(), 0);
    fixture.shutdown().await.expect("joined cleanup");
    assert_eq!(fixture.work.active_count(), 0);
}
```

Define this single snapshot fixture in the integration-test file and use it both
for the mock driver's exact-target inspection and the checked-in coordinate proof:

```rust
fn mock_snapshot() -> DeviceCapabilitySnapshot {
    DeviceCapabilitySnapshot {
        installed_agent: InstalledAgentIdentity {
            bundle_id: "com.mrph.svc".into(),
            version: "1.0".into(),
            build: "1".into(),
            executable_name: "fixture-agent".into(),
            signer_identity_sha256: "22".repeat(32),
        },
        selected_artifact_sha256: "33".repeat(32),
        agent_version: "1.0".into(),
        protocol_version: 1,
        driver_adapter_version: "fixture-driver-1".into(),
        transport: ActiveTransport::Mock,
        product_type: "iPhone10,1".into(),
        ios_version: "16.7.15".into(),
        target_app: InstalledTargetIdentity {
            bundle_id: "com.apple.Preferences".into(),
            version: "1".into(),
            build: "1".into(),
        },
        protected_auth_ready: true,
        geometry: Some(QualifiedGeometry {
            logical_width: 375.0,
            logical_height: 667.0,
            pixel_width: 375,
            pixel_height: 667,
            scale_x: 1.0,
            scale_y: 1.0,
            orientation: ScreenOrientation::Portrait,
        }),
    }
}
```

Define `MockFlowRuntimeFixture` in this integration-test file by composing the
F1 mock `DeviceDriver`, `DeviceControlPlane`, `StreamHub`, temporary `Database`,
and `FlowArtifactStore`. Its constructor publishes a baseline JPEG followed by a
different JPEG in generation 1 for each Swipe; `wait_terminal` polls the database
at 10 ms intervals with a 5-second deadline; `shutdown` calls Flow shutdown before
control cleanup. Add those three helpers as test-only methods, not production API.
The fixture driver returns `mock_snapshot()` for both UDIDs and verifies Terminate
by advertising `verifiedProcessControl`, removing the exact Settings process, and
returning its typed absence proof. Never maintain a second
hand-written snapshot or trust a profile constant without recomputing it through
`qualified_geometry_profile_id` and comparing both Swipe endpoints before compile.

- [ ] **Step 2: Run test red**

```powershell
cargo test -p riviu-core --test flow_release_one -- --nocapture
```

Expected: FAIL until the harness-facing runtime constructor and mock evidence frames are wired.

- [ ] **Step 3: Implement the headless harness**

`live_flow_test` accepts `--flow`, repeated `--udid`, `--jsonl`, and `--mock`. It
loads and compiles Flow JSON, saves the exact document/compiled plan/hash through
`save_flow_revision`, reloads that same `FlowRevisionRecord` through
`get_flow_revision`, and passes only the reloaded record to `enqueue`. It waits for
a terminal aggregate, writes one JSONL summary, invokes `flows.shutdown()`, then
`control.shutdown_cleanup()`. Exit 0 only for terminal success plus zero owned
contexts; exit 1 for arguments/config; exit 2 for
failed/partial/uncertain/cleanup failure.

Create `docs/fixtures/flow-release-one.json` with fixed UUIDs and the exact mock
sequence from Step 1. Its Swipe uses the profile produced by `mock_snapshot()` and
`FrameDigestChanged { minimumDistance: 8 }`; Screenshot uses
`ArtifactDecodedAndHashed`; Terminate uses
`ProcessAbsent { bundleId: "com.apple.Preferences" }`; Launch/Home use exact
active-app evidence.

Use this exact document:

```json
{
  "schemaVersion": 2,
  "id": "00000000-0000-0000-0000-000000000100",
  "name": "Release one acceptance",
  "revision": 1,
  "entryNodeId": "00000000-0000-0000-0000-000000000101",
  "nodes": [
    {
      "id": "00000000-0000-0000-0000-000000000101",
      "kind": "start",
      "position": { "x": 0.0, "y": 80.0 },
      "config": {}
    },
    {
      "id": "00000000-0000-0000-0000-000000000102",
      "kind": "launchApp",
      "position": { "x": 220.0, "y": 80.0 },
      "config": { "bundleId": "com.apple.Preferences" },
      "postcondition": {
        "kind": "activeAppEquals",
        "bundleId": "com.apple.Preferences"
      }
    },
    {
      "id": "00000000-0000-0000-0000-000000000103",
      "kind": "wait",
      "position": { "x": 440.0, "y": 80.0 },
      "config": { "durationMs": 10 }
    },
    {
      "id": "00000000-0000-0000-0000-000000000104",
      "kind": "swipe",
      "position": { "x": 660.0, "y": 80.0 },
      "config": {
        "from": {
          "x": 187.5, "y": 520.0,
          "imageWidth": 375, "imageHeight": 667,
          "orientation": "portrait",
          "profileId": "689551a9dbaa2e8ca25165f5b76ecaf43aa1f354551f957d3a75657105b9072b"
        },
        "to": {
          "x": 187.5, "y": 180.0,
          "imageWidth": 375, "imageHeight": 667,
          "orientation": "portrait",
          "profileId": "689551a9dbaa2e8ca25165f5b76ecaf43aa1f354551f957d3a75657105b9072b"
        },
        "durationMs": 450
      },
      "postcondition": { "kind": "frameDigestChanged", "minimumDistance": 8 }
    },
    {
      "id": "00000000-0000-0000-0000-000000000105",
      "kind": "screenshot",
      "position": { "x": 880.0, "y": 80.0 },
      "config": { "label": "release-one", "format": "jpeg" },
      "postcondition": { "kind": "artifactDecodedAndHashed" }
    },
    {
      "id": "00000000-0000-0000-0000-000000000106",
      "kind": "terminateApp",
      "position": { "x": 1100.0, "y": 80.0 },
      "config": { "bundleId": "com.apple.Preferences" },
      "postcondition": {
        "kind": "processAbsent",
        "bundleId": "com.apple.Preferences"
      }
    },
    {
      "id": "00000000-0000-0000-0000-000000000107",
      "kind": "home",
      "position": { "x": 1320.0, "y": 80.0 },
      "config": {},
      "postcondition": {
        "kind": "activeAppEquals",
        "bundleId": "com.apple.springboard"
      }
    },
    {
      "id": "00000000-0000-0000-0000-000000000108",
      "kind": "end",
      "position": { "x": 1540.0, "y": 80.0 },
      "config": {}
    }
  ],
  "edges": [
    { "id": "00000000-0000-0000-0000-000000000201", "sourceNodeId": "00000000-0000-0000-0000-000000000101", "sourcePort": "flow", "targetNodeId": "00000000-0000-0000-0000-000000000102", "targetPort": "flow" },
    { "id": "00000000-0000-0000-0000-000000000202", "sourceNodeId": "00000000-0000-0000-0000-000000000102", "sourcePort": "flow", "targetNodeId": "00000000-0000-0000-0000-000000000103", "targetPort": "flow" },
    { "id": "00000000-0000-0000-0000-000000000203", "sourceNodeId": "00000000-0000-0000-0000-000000000103", "sourcePort": "flow", "targetNodeId": "00000000-0000-0000-0000-000000000104", "targetPort": "flow" },
    { "id": "00000000-0000-0000-0000-000000000204", "sourceNodeId": "00000000-0000-0000-0000-000000000104", "sourcePort": "flow", "targetNodeId": "00000000-0000-0000-0000-000000000105", "targetPort": "flow" },
    { "id": "00000000-0000-0000-0000-000000000205", "sourceNodeId": "00000000-0000-0000-0000-000000000105", "sourcePort": "flow", "targetNodeId": "00000000-0000-0000-0000-000000000106", "targetPort": "flow" },
    { "id": "00000000-0000-0000-0000-000000000206", "sourceNodeId": "00000000-0000-0000-0000-000000000106", "sourcePort": "flow", "targetNodeId": "00000000-0000-0000-0000-000000000107", "targetPort": "flow" },
    { "id": "00000000-0000-0000-0000-000000000207", "sourceNodeId": "00000000-0000-0000-0000-000000000107", "sourcePort": "flow", "targetNodeId": "00000000-0000-0000-0000-000000000108", "targetPort": "flow" }
  ],
  "viewport": { "x": 0.0, "y": 0.0, "zoom": 0.8 }
}
```

- [ ] **Step 4: Run mock harness and commit**

```powershell
cargo test -p riviu-core --test flow_release_one -- --nocapture
cargo run -p riviu-managers-phone --bin live_flow_test -- --mock --flow docs/fixtures/flow-release-one.json --udid MOCK-IPHONE-01 --udid MOCK-IPHONE-02 --jsonl target/flow-release-one.jsonl
git add apps/desktop/src-tauri crates/core/tests docs/fixtures/flow-release-one.json
git commit -m "test(flow): cover release-one mock execution"
```

Expected: exit 0 and two independent device histories.

### Task 2: Add Playwright Workflow And Visual Checks

**Files:**
- Create: `apps/desktop/playwright.config.ts`
- Create: `apps/desktop/e2e/flow-workspace.spec.ts`
- Create: `apps/desktop/e2e/fixtures/tauriMock.ts`
- Modify: `apps/desktop/package.json`

- [ ] **Step 1: Add Playwright scripts and deterministic Tauri mock**

Add `test:e2e` as `playwright test`. The browser fixture installs `window.__TAURI_INTERNALS__` invoke/listener mocks before App loads and serves a stable catalog, devices, saved revisions, and run projection.

Use this Playwright configuration:

```ts
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  retries: 0,
  timeout: 30_000,
  expect: { timeout: 5_000 },
  use: {
    baseURL: "http://127.0.0.1:1421",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: "npm run dev -- --host 127.0.0.1 --port 1421",
    url: "http://127.0.0.1:1421",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
```

`installTauriMock(page)` must call `page.addInitScript` before navigation. Inside
that script keep callback IDs and listener IDs in separate maps. Implement this
exact ownership model; `plugin:event|listen` stores the numeric callback ID under a
new listener ID, while unlisten removes only the listener registration. Also
install `window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener` through the
same removal helper. Expose only this test hook for controlled invalidations:

```ts
const callbacks = new Map<number, (event: unknown) => void>();
const listeners = new Map<number, number>();
const commandHandlers = new Map<
  string,
  (args: Record<string, unknown>) => unknown | Promise<unknown>
>();
let nextCallbackId = 1;
let nextListenerId = 1;

function transformCallback(
  callback: (event: unknown) => void,
  once = false,
): number {
  const callbackId = nextCallbackId++;
  callbacks.set(callbackId, (event) => {
    if (once) callbacks.delete(callbackId);
    callback(event);
  });
  return callbackId;
}

function unregisterCallback(callbackId: number): void {
  callbacks.delete(callbackId);
}

function unregisterListener(listenerId: number): void {
  const callbackId = listeners.get(listenerId);
  listeners.delete(listenerId);
  if (callbackId !== undefined) callbacks.delete(callbackId);
}

async function invoke(command: string, args: Record<string, unknown> = {}) {
  if (command === "plugin:event|listen") {
    const handlerId = args.handler;
    if (typeof handlerId !== "number" || !callbacks.has(handlerId)) {
      throw new Error("Invalid mock event handler");
    }
    const listenerId = nextListenerId++;
    listeners.set(listenerId, handlerId);
    return listenerId;
  }
  if (command === "plugin:event|unlisten") {
    const listenerId = args.eventId;
    if (typeof listenerId !== "number") throw new Error("Invalid mock listener ID");
    unregisterListener(listenerId);
    return null;
  }
  const handler = commandHandlers.get(command);
  if (!handler) throw new Error(`Unknown mock command: ${command}`);
  return handler(args);
}

window.__TAURI_INTERNALS__ = {
  ...window.__TAURI_INTERNALS__,
  transformCallback,
  unregisterCallback,
  invoke,
};
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
  unregisterListener: (_event: string, listenerId: number) => {
    unregisterListener(listenerId);
  },
};

window.__RIVIU_TEST__ = {
  emit(payload: unknown) {
    for (const handlerId of listeners.values()) {
      callbacks.get(handlerId)?.({ id: 1, event: "riviu://event", payload });
    }
  },
};
```

Populate `commandHandlers` in the same init script. Its handlers return: two ready
mock devices for `list_devices`; `[]`
for `list_jobs`; `{fps:24,tileSize:"medium",gridQuality:"medium",focusQuality:"high"}`
for `get_stream_settings`; `{showAuthUi:false,bypassed:true,user:null}` for
`auth_session`; the final F1 release-one catalog including typed Terminate for
`flow_action_catalog`; an in-memory revision
array for Flow list/get/save/export; compiler fixture output for validate; and an
in-memory run/detail for run/cancel/retry/list/get. Unknown commands throw
`Unknown mock command: <name>` so new startup dependencies cannot pass silently.

- [ ] **Step 2: Write workflow checks**

Test create flow, drag Wait/Tap, connect, edit properties/evidence, validate, save, choose Selected devices, run, observe independent attempts, cancel another run, import supported legacy JSON, and show unsupported diagnostics without mutating the draft.

Drive only accessible controls, with this test shape:

```ts
test("authors, saves, and runs a selected-device flow", async ({ page }) => {
  await installTauriMock(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Flow" }).click();
  await page.getByRole("button", { name: "New flow" }).click();
  await page.getByLabel("Flow name").fill("E2E flow");
  await page.locator(".react-flow__edge-path").first().click();
  await page.getByRole("button", { name: "Wait" }).dragTo(
    page.getByTestId("flow-canvas"),
    { targetPosition: { x: 360, y: 220 } },
  );
  await page.getByLabel("Duration (ms)").fill("250");
  await page.getByRole("button", { name: "Validate flow" }).click();
  await expect(page.getByRole("dialog", { name: "Compile preview" })).toContainText("Valid");
  await page.getByRole("button", { name: "Save revision" }).click();
  await page.getByRole("button", { name: "Run flow" }).click();
  await page.getByRole("radio", { name: "Selected" }).check();
  await page.getByRole("button", { name: "Run on devices" }).click();
  await expect(page.getByRole("row", { name: /MOCK-IPHONE-01.*Succeeded/ })).toBeVisible();
  await expect(page.getByRole("row", { name: /MOCK-IPHONE-02.*Succeeded/ })).toBeVisible();
});
```

Add a second test whose mock returns `Uncertain` for Tap and assert no Retry button,
then starts a Wait run, clicks Cancel, emits `flowRunUpdated`, and observes
`Cancelled` after the matching-detail refetch. Add a third test importing one
supported legacy JSON and one Wait 60,001 ms JSON; the latter must show
`WaitOutOfRange` while the canvas node count remains unchanged.

Add this compatibility check under the exact title used by the rollback command:

```ts
test("legacy scripts and jobs remain reachable", async ({ page }) => {
  await installTauriMock(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Flow" }).click();
  await page.getByRole("tab", { name: "Legacy" }).click();
  await expect(page.getByRole("heading", { name: "Scripts" })).toBeVisible();
  await page.getByRole("button", { name: "Use in Jobs" }).first().click();
  await expect(page.getByText("Jobs", { selector: ".topbar-title" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Run script" })).toBeVisible();
});
```

- [ ] **Step 3: Write visual checks**

Capture 1440x900 and 900x700 screenshots. Assert the canvas bounding box is nonzero, at least one node is visible, palette/toolbar/inspector/monitor boxes do not overlap incoherently, longest labels fit, and no horizontal document overflow exists.

Use `page.setViewportSize`, `locator.boundingBox()`, and this overlap predicate;
commit Playwright's generated snapshot baselines beside the spec:

```ts
interface Box { x: number; y: number; width: number; height: number }

function overlaps(a: Box, b: Box): boolean {
  return a.x < b.x + b.width && a.x + a.width > b.x
    && a.y < b.y + b.height && a.y + a.height > b.y;
}

const canvas = await page.getByTestId("flow-canvas").boundingBox();
expect(canvas).not.toBeNull();
expect(canvas!.width).toBeGreaterThanOrEqual(420);
expect(await page.locator(".flow-node").count()).toBeGreaterThan(0);
const regionBoxes = (await Promise.all([
  page.getByTestId("flow-toolbar").boundingBox(),
  page.getByTestId("flow-palette").boundingBox(),
  page.getByTestId("flow-canvas").boundingBox(),
  page.getByTestId("flow-inspector").boundingBox(),
  page.getByTestId("flow-monitor").boundingBox(),
])).filter((box): box is Box => box !== null);
for (let left = 0; left < regionBoxes.length; left += 1) {
  for (let right = left + 1; right < regionBoxes.length; right += 1) {
    expect(overlaps(regionBoxes[left], regionBoxes[right])).toBe(false);
  }
}
expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
expect(await page.locator("button, .flow-node-title").evaluateAll((elements) =>
  elements.every((element) => element.scrollWidth <= element.clientWidth)
)).toBe(true);
await expect(page).toHaveScreenshot(
  `flow-${viewport.width}x${viewport.height}.png`,
  {
    fullPage: true,
    animations: "disabled",
    maxDiffPixelRatio: 0.002,
  },
);
```

- [ ] **Step 4: Run and commit**

```powershell
npm --prefix apps/desktop exec -- playwright install chromium
npm --prefix apps/desktop run test:e2e
git add apps/desktop/package.json apps/desktop/package-lock.json apps/desktop/playwright.config.ts apps/desktop/e2e
git commit -m "test(flow): verify visual workflow with Playwright"
```

Expected: all workflow and both viewport tests pass.

### Task 3: Run The Mac/iPhone Live Gate

**Files:**
- Create: `docs/re/flow-v2/gate-f3.json`
- Create: `docs/re/flow-v2/gate-f3.md`
- Modify: `apps/desktop/src-tauri/src/bin/live_flow_test.rs`

- [ ] **Step 1: Prepare without changing production artifacts**

On Mac, confirm 3uTools runner is absent, the exact device is unlocked/trusted, production IPA/manifest hashes match `AGENTS.md`, and no desktop/harness process owns the UDID. Build `live_flow_test --release` and record app/Agent/adapter versions.

Run these preconditions from the repository root; the token is already present in
the shell environment and is never echoed:

```bash
export PATH="$HOME/Library/Python/3.9/bin:$PATH"
export RIVIU_FLOW_UDID="a99f4bd9f877b2a0e3682ee24fd1c68f75ba6982"
test -n "$RIVIU_RTMMO_TOKEN"
test "$(shasum -a 256 sidecars/wda/RiviuAgent.ipa | awk '{print $1}')" = "8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea"
test "$(shasum -a 256 sidecars/wda/agent-manifest.json | awk '{print $1}')" = "e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a"
tidevice -u "$RIVIU_FLOW_UDID" kill notes.3u || true
tidevice -u "$RIVIU_FLOW_UDID" launch com.ss.iphone.ugc.Ame
cargo build -p riviu-managers-phone --bin live_flow_test --release
```

- [ ] **Step 2: Execute the fixed live sequence**

Use Settings with Launch, Wait, verified Tap, verified Swipe, Type Text with exact SearchField read-back, stream Screenshot, and Home. Require pre/post generation-qualified evidence for each side effect. Inject one fixture ACK without postcondition and assert it terminates `FailedVerified`, not `Succeeded`.

Add `--settings-gate` to the harness. Before compiling, it briefly acquires
`ManualControl`, calls `inspect_flow_device(..., "com.apple.Preferences")`, computes
the shared profile ID, and releases. It constructs fixed-ID nodes Launch -> Wait
250 ms -> coordinate Tap at `(187.5, 85.0)` with keyboard-region frame evidence ->
Type Text `Riviu Unicode được 🔥` with
`{strategy:"className",value:"XCUIElementTypeSearchField"}` exact read-back ->
Swipe `(187.5,520)` to `(187.5,180)` over 450 ms -> Screenshot JPEG -> Home.
Both coordinate actions store the inspected dimensions/orientation/profile; the
runtime independently inspects again and fails before dispatch if the tuple changed.

Add mutually exclusive harness modes `--fault-ack-without-effect tap`,
`--cancel-during-wait`, and `--recover-only`, plus `--state-dir`. The fault adapter
returns a successful Tap driver result without delivering it; unchanged keyboard
region must produce `FailedVerified`. Cancellation starts a 10-second Wait, requests
cancel after 250 ms, and requires `Cancelled` plus release proof. A test-only
`--crash-after-dispatch swipe` hook exits with code 86 after persisting dispatch and
before verification; the next `--recover-only` run must classify the old attempt
`Uncertain`, never dispatch it again, then clean both ports.

Execute each mode against its own persisted state directory:

```bash
SUCCESS_DIR="$(mktemp -d /tmp/riviu-flow-f3-success.XXXXXX)"
FAULT_DIR="$(mktemp -d /tmp/riviu-flow-f3-fault.XXXXXX)"
CANCEL_DIR="$(mktemp -d /tmp/riviu-flow-f3-cancel.XXXXXX)"
RECOVERY_DIR="$(mktemp -d /tmp/riviu-flow-f3-recovery.XXXXXX)"
./target/release/live_flow_test --settings-gate --udid "$RIVIU_FLOW_UDID" --state-dir "$SUCCESS_DIR" --jsonl "$SUCCESS_DIR/summary.jsonl"
./target/release/live_flow_test --settings-gate --fault-ack-without-effect tap --udid "$RIVIU_FLOW_UDID" --state-dir "$FAULT_DIR" --jsonl "$FAULT_DIR/summary.jsonl"
./target/release/live_flow_test --settings-gate --cancel-during-wait --udid "$RIVIU_FLOW_UDID" --state-dir "$CANCEL_DIR" --jsonl "$CANCEL_DIR/summary.jsonl"
./target/release/live_flow_test --settings-gate --crash-after-dispatch swipe --udid "$RIVIU_FLOW_UDID" --state-dir "$RECOVERY_DIR" --jsonl "$RECOVERY_DIR/crash.jsonl" || test "$?" = 86
./target/release/live_flow_test --recover-only --udid "$RIVIU_FLOW_UDID" --state-dir "$RECOVERY_DIR" --jsonl "$RECOVERY_DIR/recovered.jsonl"
```

Run a second sequence cancelled during Wait and a controlled desktop termination with an in-flight request. Restart and assert recovery class, no automatic duplicate effect, zero owned context, and both control/MJPEG device ports closed after cleanup.

- [ ] **Step 3: Publish redacted transactional evidence**

The JSON contains exact tuple, plan/revision/hash, per-node state/timing/evidence hash, artifact hashes, cancellation/recovery result, release proof, and production artifact hashes. Markdown summarizes pass/fail without raw token, full UDID, user home path, or proxy password. Write both to temporary files, run repository redaction verification, then atomically replace both reports; retain old reports if either replace fails.

Serialize the report from these structs; `device_fingerprint_sha256` is SHA-256 of
the full UDID and is the only device identifier written:

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GateF3Report {
    status: String,
    generated_at: String,
    device_fingerprint_sha256: String,
    product_type: String,
    ios_version: String,
    agent_artifact_sha256: String,
    agent_manifest_sha256: String,
    agent_version: String,
    protocol_version: u32,
    driver_adapter_version: String,
    successful_run: GateRunEvidence,
    ack_without_effect: GateRunEvidence,
    cancellation: GateRunEvidence,
    recovery: GateRunEvidence,
    cleanup: GateCleanupEvidence,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GateRunEvidence {
    flow_id: String,
    revision: u64,
    plan_sha256: String,
    terminal_state: String,
    attempts: Vec<GateAttemptEvidence>,
    artifact_sha256: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GateAttemptEvidence {
    node_id: String,
    action: String,
    state: String,
    started_at: String,
    finished_at: String,
    evidence_sha256: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GateCleanupEvidence {
    owned_contexts: usize,
    control_port_open: bool,
    mjpeg_port_open: bool,
    workers_joined: bool,
}
```

Require success run `Succeeded`; fault Tap `FailedVerified`; cancellation
`Cancelled`; recovered Swipe `Uncertain`; zero contexts, closed device ports, and
joined workers. Produce `.tmp` JSON/Markdown, then run:

```bash
cargo run -q -p rtmmo-re -- verify-redaction --input docs/re/flow-v2/gate-f3.json.tmp --input docs/re/flow-v2/gate-f3.md.tmp
```

Use the same backup/replace/rollback transaction pattern as
`probe_gate_bc.py`; never leave one new report beside one old report.

- [ ] **Step 4: Commit live evidence only after PASS**

```bash
git add apps/desktop/src-tauri/src/bin/live_flow_test.rs docs/re/flow-v2/gate-f3.json docs/re/flow-v2/gate-f3.md
git commit -m "test(flow): record Mac device gate F3"
```

Expected: F3 is `PASS` only when every node proof and cleanup assertion passes. Windows/mock output is labeled `FIXTURE_ONLY`.

### Task 4: Package, Roll Back, And Close Release 1

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/superpowers/plans/2026-07-30-riviu-flow-v2-acceptance.md`
- Create: `docs/fixtures/rollback-legacy-probe.rs`
- Create: `docs/re/flow-v2/release-1.md`

- [ ] **Step 1: Run final regression**

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
npm --prefix apps/desktop test
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
npm --prefix apps/desktop run test:e2e
python -m unittest sidecars.pymobiledevice3.test_app_control -v
git diff --check
```

On Mac additionally run the release Tauri build and launch the packaged app with mock devices before the live tuple.

- [ ] **Step 2: Prove rollback**

Create `docs/fixtures/rollback-legacy-probe.rs`. The proof copies this test into
`crates/script-engine/tests` in the detached pre-F0 worktree, so it is compiled
against the old `riviu-core` plus legacy parser and exercises both against the
database already migrated by the release binary:

```rust
use riviu_core::db::Database;
use riviu_script_engine::parse_script;

#[test]
fn pre_f0_core_reads_legacy_rows_from_release_migrated_database() {
    let path = std::env::var_os("RIVIU_ROLLBACK_PROOF_DB")
        .expect("RIVIU_ROLLBACK_PROOF_DB");
    let database = Database::open(path).expect("pre-F0 opens migrated database");
    let scripts = database.list_scripts().expect("pre-F0 list_scripts");
    let (_, body) = scripts.iter().find(|(name, _)| name == "fixture")
        .expect("fixture script");
    let script = parse_script(body).expect("pre-F0 parses the unchanged v1 script");
    assert_eq!(script.name, "fixture");
    assert_eq!(script.steps.len(), 1);
    let jobs = database.list_jobs(100).expect("pre-F0 list_jobs");
    assert!(jobs.iter().any(|job|
        job.id.to_string() == "00000000-0000-0000-0000-000000000901"
            && job.script_name == "fixture"
    ));
}
```

```powershell
if ([string]::IsNullOrWhiteSpace($env:RIVIU_PRE_F0_COMMIT)) {
  throw "RIVIU_PRE_F0_COMMIT must be the commit recorded before Task F0.1"
}

$repo = (Resolve-Path ".").Path
$rollbackRoot = Join-Path (Split-Path $repo -Parent) ("riviu-flow-rollback-" + [guid]::NewGuid())
$proofRoot = Join-Path $env:TEMP ("riviu-flow-rollback-" + [guid]::NewGuid())
$fixturePath = Join-Path $proofRoot "pre-flow-v1.db"
$rollbackData = Join-Path $proofRoot "rollback-appdata"
$rollbackDbDir = Join-Path $rollbackData "riviu-managers-phone"
$rollbackDb = Join-Path $rollbackDbDir "riviu.db"
$cleanDb = Join-Path $proofRoot "release-migrated-clean.db"
$probeTarget = Join-Path $rollbackRoot "crates/script-engine/tests/rollback_legacy_probe.rs"
New-Item -ItemType Directory -Path $proofRoot,$rollbackDbDir | Out-Null

$env:RIVIU_LEGACY_FIXTURE_PATH = $fixturePath
cargo test -p riviu-core db::migrations::tests::write_populated_legacy_fixture -- --ignored --exact --nocapture
python -c 'import os,sqlite3; p=os.environ["RIVIU_LEGACY_FIXTURE_PATH"]; c=sqlite3.connect(p); c.execute("update jobs set id=? where id=?",("00000000-0000-0000-0000-000000000901","job-1")); c.commit(); c.close()'
Copy-Item -LiteralPath $fixturePath -Destination $rollbackDb

$savedAppData = $env:APPDATA
$savedMock = $env:RIVIU_MOCK_DEVICES
$oldApp = $null
$releaseApp = $null
try {
  $env:APPDATA = $rollbackData
  $env:RIVIU_MOCK_DEVICES = "1"

  cargo build -p riviu-managers-phone
  $releaseApp = Start-Process -FilePath (Join-Path $repo "target/debug/riviu-managers-phone.exe") -WindowStyle Hidden -PassThru
  Start-Sleep -Seconds 5
  if ($releaseApp.HasExited) { throw "Release desktop failed to migrate the copied v1 database" }
  if (-not $releaseApp.CloseMainWindow()) { throw "Release desktop did not accept a clean close" }
  if (-not $releaseApp.WaitForExit(10000)) { throw "Release desktop did not finish clean shutdown" }
  $releaseApp = $null

  $env:RIVIU_ROLLBACK_PROOF_DB = $rollbackDb
  $env:RIVIU_ROLLBACK_CLEAN_DB = $cleanDb
  python -c 'import json,os,sqlite3; c=sqlite3.connect(os.environ["RIVIU_ROLLBACK_PROOF_DB"]); body=c.execute("select body_json from scripts where id=?",("script-1",)).fetchone()[0]; assert json.loads(body)=={"version":1,"name":"fixture","steps":[{"action":"wait","milliseconds":1}]}; assert c.execute("select id from jobs where id=?",("00000000-0000-0000-0000-000000000901",)).fetchone()==("00000000-0000-0000-0000-000000000901",); assert c.execute("select version from schema_migrations order by version").fetchall()==[(1,),(2,)]; c.close()'
  python -c 'import os,sqlite3; src=os.environ["RIVIU_ROLLBACK_PROOF_DB"]; dst=os.environ["RIVIU_ROLLBACK_CLEAN_DB"]; a=sqlite3.connect(src); b=sqlite3.connect(dst); a.backup(b); b.close(); a.close(); c=sqlite3.connect(dst); assert c.execute("pragma integrity_check").fetchone()==("ok",); c.close(); os.replace(dst,src)'

  git worktree add --detach $rollbackRoot $env:RIVIU_PRE_F0_COMMIT
  New-Item -ItemType Directory -Path (Split-Path $probeTarget -Parent) -Force | Out-Null
  Copy-Item -LiteralPath (Join-Path $repo "docs/fixtures/rollback-legacy-probe.rs") -Destination $probeTarget
  Push-Location $rollbackRoot
  npm --prefix apps/desktop ci
  npm --prefix apps/desktop run build
  cargo test -p riviu-script-engine --test rollback_legacy_probe -- --nocapture
  cargo build -p riviu-managers-phone
  $oldApp = Start-Process -FilePath (Join-Path $rollbackRoot "target/debug/riviu-managers-phone.exe") -WindowStyle Hidden -PassThru
  Start-Sleep -Seconds 5
  if ($oldApp.HasExited) { throw "Pre-F0 desktop failed to boot the release-migrated database" }
  if (-not $oldApp.CloseMainWindow()) { throw "Pre-F0 desktop did not accept a clean close" }
  if (-not $oldApp.WaitForExit(10000)) { throw "Pre-F0 desktop did not finish clean shutdown" }
  $oldApp = $null
  Pop-Location
} finally {
  if ($null -ne $oldApp -and -not $oldApp.HasExited) {
    Stop-Process -Id $oldApp.Id
    $oldApp.WaitForExit()
  }
  if ($null -ne $releaseApp -and -not $releaseApp.HasExited) {
    Stop-Process -Id $releaseApp.Id
    $releaseApp.WaitForExit()
  }
  if ((Get-Location).Path -eq $rollbackRoot) { Pop-Location }
  if (Test-Path -LiteralPath $probeTarget) {
    Remove-Item -LiteralPath $probeTarget
  }
  $env:APPDATA = $savedAppData
  $env:RIVIU_MOCK_DEVICES = $savedMock
  Remove-Item Env:RIVIU_LEGACY_FIXTURE_PATH -ErrorAction SilentlyContinue
  Remove-Item Env:RIVIU_ROLLBACK_PROOF_DB -ErrorAction SilentlyContinue
  Remove-Item Env:RIVIU_ROLLBACK_CLEAN_DB -ErrorAction SilentlyContinue
}

git worktree remove $rollbackRoot
```

The release binary must migrate the one copied database and complete its normal exit
path first. Only after SQLite confirms versions 1 and 2 plus unchanged legacy rows
does the script use SQLite's backup API to replace it with a clean, integrity-checked
copy of those same migrated bytes. The pre-F0 worktree then builds. Its probe and
desktop binary both use that same `rollbackDb`; the probe exercises the old
`list_scripts`, v1 `parse_script`, and `list_jobs` paths, while desktop boot exercises
legacy startup and must remain alive for five seconds. Run worktree removal only
after those assertions; leave `proofRoot`
in place until `release-1.md` records its hashes. No command points `APPDATA` at the
operator's real profile. Rollback disables Flow UI/commands and reads the additive
database in place; it never downgrades or rewrites it.

- [ ] **Step 3: Record release scope**

```markdown
# Riviu Flow V2 Release 1

## Gates

- F0 foundation: PASS, with commit and command counts recorded in AGENTS.md.
- F1 runtime: PASS, with mock recovery, shutdown, exact DVT termination, and
  `ProcessAbsent` evidence recorded in AGENTS.md.
- F2 desktop: PASS, with Vitest, build, lint, and Playwright counts recorded in AGENTS.md.
- F3 live: PASS only when docs/re/flow-v2/gate-f3.json reports PASS.

## Enabled Nodes

Start, End, Launch App, Terminate App, Wait, Tap, Swipe, Type Text, Screenshot,
Home, and Assert Visible are enabled only when their action definition has no
disabled reason and runtime preflight qualifies the required device capabilities.

Terminate App is enabled after F1's bounded exact-PID DVT termination, Rust sidecar
contract test, and runtime evidence verifier pass. A successful attempt must persist
`ProcessAbsent { bundleId }` evidence derived from `ok=true`, `running=false`, and
the exact requested bundle, with `oldPid` matched to the persisted pre-effect
process baseline; transport acknowledgement alone remains insufficient.
This is the approved F1 enablement contract and does not introduce a separate live
tuple gate.

## Qualified Profiles

The only live-qualified tuple is the tuple and profile hash recorded in
docs/re/flow-v2/gate-f3.json. Mock profiles are FIXTURE_ONLY. No model, iOS,
Agent, target-app, adapter, transport, orientation, or geometry wildcard is
implied.

## Deferred Scope

TikTok action nodes, conditions, loops, cross-device bindings, account switching,
MDM/supervision, backup/restore, proxy application, push media, and syslog remain
outside release 1.

## Production Agent

- RiviuAgent.ipa SHA-256: 8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea
- agent-manifest.json SHA-256: e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a

## Rollback

The pre-F0 commit is recorded in AGENTS.md. Rollback disables Flow UI and commands,
keeps the additive database intact, and retains the Legacy tab plus Jobs path for
one release. The isolated-worktree pre-F0 binary and legacy repository probe both
opened the same copied database after Release 1 migrated it; no production database
was downgraded or rewritten.
```

Document enabled release-1 nodes and exact qualified profiles. Record Terminate App
as enabled by F1 only when its attempt contains verified `ProcessAbsent` evidence;
do not add a new live tuple gate beyond the approved DVT/Rust/runtime contract.
State that TikTok nodes, conditions, loops, cross-device bindings, MDM,
backup/restore, proxy application, and syslog remain outside release 1. Record
production Agent hashes unchanged.

- [ ] **Step 4: Close F3 and commit**

Mark checkboxes complete, update `AGENTS.md` with F3 commit/test counts/live tuple/rollback, and commit:

```powershell
git add AGENTS.md docs/fixtures/rollback-legacy-probe.rs docs/superpowers/plans/2026-07-30-riviu-flow-v2-acceptance.md docs/re/flow-v2/release-1.md
git commit -m "docs(flow): close release-one gate F3"
```
