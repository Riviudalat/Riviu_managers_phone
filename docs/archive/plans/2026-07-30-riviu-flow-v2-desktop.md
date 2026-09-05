# Riviu Flow V2 Desktop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose Flow V2 through typed Tauri commands and replace the JSON-first Automation page with a usable visual editor and per-device run monitor.

**Architecture:** Tauri compiles and persists authoritative revisions, then delegates immutable plans to `FlowRuntime`. React uses controlled `@xyflow/react` nodes/edges, keeps layout/draft/undo locally, and refetches projections after ID/revision invalidation events.

**Tech Stack:** Tauri 2, Rust 2021, React 19, TypeScript 6, `@xyflow/react`, `lucide-react`, Vitest, Testing Library, jsdom, Playwright.

---

### Task 1: Compose Flow Runtime And Typed Tauri Commands

**Files:**
- Create: `apps/desktop/src-tauri/src/flow_commands.rs`
- Modify: `apps/desktop/src-tauri/src/command_error.rs`
- Modify: `apps/desktop/src-tauri/src/state.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/src/commands.rs`
- Modify: `apps/desktop/src-tauri/src/agent_commands.rs`
- Modify: `apps/desktop/src-tauri/src/farm_commands.rs`
- Modify: `apps/desktop/src-tauri/src/nurture_commands.rs`
- Test: `apps/desktop/src-tauri/src/flow_commands.rs`
- Test: `apps/desktop/src-tauri/src/lib.rs`

- [x] **Step 1: Write failing command and exit-order tests**

Test action catalog redaction, save revision 1, stale revision conflict with `nodeId/field` omitted, validation with node-scoped errors, legacy diagnostics, run selection serialization, retry-not-allowed, and exit order `reject new work -> drain admitted commands -> flows.stop_all -> sampler shutdown -> flows.shutdown -> jobs.shutdown -> control.shutdown_cleanup` with cancellation during acquire, Wait, and an in-flight UI call. Add deterministic barriers for a mutating command admitted immediately before shutdown and a contender arriving immediately after rejection.

```rust
#[test]
fn catalog_command_exposes_only_release_one_typed_actions() {
    let json = serde_json::to_value(flow_action_catalog()).expect("catalog json");
    assert!(json.as_array().expect("catalog array").iter().all(|action| {
        action.as_object().expect("action object").contains_key("disabledReason")
    }));
    let encoded = json.to_string();
    assert!(encoded.contains("terminateApp"));
    assert!(encoded.contains("processAbsent"));
    assert!(!encoded.contains("rawHttp"));
    assert!(!encoded.contains("rawWda"));
    assert!(!encoded.contains("shell"));
    assert!(!encoded.to_ascii_lowercase().contains("token"));
}

#[tokio::test]
async fn shutdown_joins_flow_before_control_cleanup() {
    let fixture = shutdown_fixture_with_blocked_flow();
    fixture.shutdown().await.expect("shutdown");
    assert_eq!(fixture.operations(), vec![
        "rejectNewWork", "mutationsDrained", "nurtureStop", "flowsStop", "jobsStop",
        "samplerJoined", "flowsJoined", "jobsJoined", "controlCleanup",
    ]);
}

#[tokio::test]
async fn shutdown_drains_an_admitted_mutation_and_rejects_the_racing_contender() {
    let fixture = command_shutdown_race_fixture();
    let admitted = fixture.spawn_blocked_mutating_command();
    fixture.wait_until_command_is_admitted().await;
    let shutdown = fixture.spawn_shutdown();
    fixture.wait_until_new_work_is_rejected().await;

    let error = fixture.run_mutating_command().await.expect_err("contender rejected");
    assert_eq!(error.code, "ApplicationShuttingDown");
    assert_eq!(fixture.repository_mutations(), 0);

    fixture.release_admitted_command();
    admitted.await.expect("admitted command joined");
    shutdown.await.expect("shutdown joined");
    assert!(fixture.no_mutation_after("mutationsDrained"));
}
```

Define `shutdown_fixture_with_blocked_flow` and `command_shutdown_race_fixture` in
the test module with fake components that append to one
`Arc<Mutex<Vec<&'static str>>>`. The Flow fake releases its blocked task only when
`flows.shutdown()` is awaited. The command fixture blocks after obtaining its
admission permit but before its repository mutation, so the test proves shutdown
waits for already admitted work while a later command never reaches the repository.

- [x] **Step 2: Run tests red**

```powershell
cargo test -p riviu-managers-phone flow_commands -- --nocapture
cargo test -p riviu-managers-phone exit_order -- --nocapture
```

Expected: FAIL because Flow state and commands are absent.

- [x] **Step 3: Add structured Flow command errors**

Extend `CommandError` with optional `node_id`, `field`, and `attempt_id`; reuse its
existing optional `udid` for device scope. Add constructors that preserve stable
codes from compiler, repository, runtime, and retry errors. Existing DeviceBusy
serialization must remain byte-compatible apart from omitted new fields. Add the
stable document-level code `ApplicationShuttingDown` for command admission rejection;
it has no node, field, attempt, or UDID.

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub node_id: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub field: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub attempt_id: Option<String>,
```

- [x] **Step 4: Compose FlowRuntime in AppState**

Add `pub flows: FlowRuntime`. Construct it after `JobQueue` with the existing DB, events, registry, control, `Arc::new(bundle.streams.clone())` as `GenerationFrameSource`, and `artifacts_dir.join("flows")`. Run startup artifact/attempt reconciliation before accepting commands.

```rust
let command_admission = Arc::new(CommandAdmissionState::new(false));
let flow_artifacts = FlowArtifactStore::new(artifacts_dir.join("flows"))?;
let flows = FlowRuntime::new(FlowRuntimeDeps {
    database: db.clone(),
    events: events.clone(),
    registry: registry.clone(),
    control: control.clone(),
    frames: Arc::new(bundle.streams.clone()),
    artifacts: flow_artifacts,
});
flows.recover_startup().await?;
command_admission.start_accepting();
```

Move `flows` into the `Self` initializer and add `pub flows: FlowRuntime` beside
`pub jobs: JobQueue`.

Also add one process-wide command admission state. Its `accepting_work` flag starts
`false`, changes to `true` with `Release` ordering only after all startup recovery
has completed, and changes back to `false` exactly once when exit begins. A plain
check-then-mutate is racy, so `ensure_accepting_work()` returns an RAII permit using
a double check around an in-flight increment:

```rust
struct CommandAdmissionState {
    accepting_work: AtomicBool,
    in_flight: AtomicUsize,
    changed: Notify,
}

struct CommandAdmission {
    state: Arc<CommandAdmissionState>,
}

impl CommandAdmissionState {
    fn new(accepting_work: bool) -> Self {
        Self {
            accepting_work: AtomicBool::new(accepting_work),
            in_flight: AtomicUsize::new(0),
            changed: Notify::new(),
        }
    }

    fn start_accepting(&self) {
        self.accepting_work.store(true, Ordering::Release);
    }

    fn ensure_accepting_work(self: &Arc<Self>) -> Result<CommandAdmission, CommandError> {
        if !self.accepting_work.load(Ordering::Acquire) {
            return Err(CommandError::application_shutting_down());
        }
        self.in_flight.fetch_add(1, Ordering::AcqRel);
        if !self.accepting_work.load(Ordering::Acquire) {
            self.finish_one();
            return Err(CommandError::application_shutting_down());
        }
        Ok(CommandAdmission { state: self.clone() })
    }

    fn reject_new_work(&self) {
        self.accepting_work.store(false, Ordering::Release);
        self.changed.notify_waiters();
    }

    async fn wait_until_drained(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.in_flight.load(Ordering::Acquire) == 0 {
                return;
            }
            changed.await;
        }
    }

    fn finish_one(&self) {
        if self.in_flight.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.changed.notify_waiters();
        }
    }
}

impl Drop for CommandAdmission {
    fn drop(&mut self) {
        self.state.finish_one();
    }
}
```

The permit contains only `Arc` plus atomics, is `Send`, and remains live across the
entire async command. This gives shutdown a linearization point without holding a
synchronous lock across `.await`. Registering the `Notified` future with `enable()`
before checking the counter is mandatory; otherwise the last permit can notify in
the check/await gap and hang exit. Expose delegating `AppState::ensure_accepting_work`,
`AppState::reject_new_work`, and `AppState::wait_for_mutating_commands` methods.

- [x] **Step 5: Implement typed commands**

Implement these exact commands in `flow_commands.rs`:

```rust
#[tauri::command] pub fn flow_action_catalog() -> Vec<ActionDefinition>;
#[tauri::command] pub fn flow_list(state: State<'_, AppState>, include_archived: bool) -> Result<Vec<FlowSummary>, CommandError>;
#[tauri::command] pub fn flow_get(state: State<'_, AppState>, id: String, revision: Option<u64>) -> Result<Option<FlowRevisionRecord>, CommandError>;
#[tauri::command] pub fn flow_validate(document: FlowDocumentV2) -> Result<CompiledRevision, Vec<CommandError>>;
#[tauri::command] pub fn flow_save_revision(state: State<'_, AppState>, document: FlowDocumentV2, expected_revision: Option<u64>) -> Result<FlowRevisionRecord, CommandError>;
#[tauri::command] pub fn flow_archive(state: State<'_, AppState>, id: String) -> Result<(), CommandError>;
#[tauri::command] pub fn flow_import_legacy(script_json: String) -> Result<LegacyImportResult, CommandError>;
#[tauri::command] pub fn flow_export(state: State<'_, AppState>, id: String, revision: Option<u64>) -> Result<String, CommandError>;
#[tauri::command] pub async fn flow_run(state: State<'_, AppState>, id: String, revision: Option<u64>, selection: FlowTargetSelection) -> Result<FlowRunRecord, CommandError>;
#[tauri::command] pub fn flow_cancel_run(state: State<'_, AppState>, run_id: String) -> Result<(), CommandError>;
#[tauri::command] pub async fn flow_retry_attempt(state: State<'_, AppState>, attempt_id: String) -> Result<FlowNodeAttemptRecord, CommandError>;
#[tauri::command] pub fn flow_list_runs(state: State<'_, AppState>, limit: usize) -> Result<Vec<FlowRunRecord>, CommandError>;
#[tauri::command] pub fn flow_get_run(state: State<'_, AppState>, run_id: String) -> Result<Option<FlowRunDetail>, CommandError>;
#[tauri::command] pub async fn flow_coordinate_frame(state: State<'_, AppState>, udid: String, bundle_id: String) -> Result<FlowCoordinateFrame, CommandError>;
```

`flow_coordinate_frame` first retains the current `StreamHub` frame in memory, then
uses `try_acquire_exclusive(..., DeviceWorkOwner::ManualControl)` to inspect an exact
target-qualified capability snapshot through
`inspect_flow_device(&context, &bundle_id)` and releases that context before returning.
A busy device returns
typed `DeviceBusy`. Build `profile_id` only through F0's shared
`qualified_geometry_profile_id(&snapshot)` helper. It does not start a stream, create
a session, or call WDA screenshot.

Decode the retained JPEG before acquisition and require nonzero dimensions. After
inspection, require those dimensions to equal the snapshot's qualified pixel width
and height; otherwise return `FrameGeometryMismatch`. No current frame returns
`FrameUnavailable`. Release the exclusive context on decode, inspection, mismatch,
serialization, and success paths.

Define its projection in `flow_commands.rs`:

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowCoordinateFrame {
    pub jpeg_base64: String,
    pub image_width: u32,
    pub image_height: u32,
    pub orientation: String,
    pub profile_id: String,
}
```

Register every command in `generate_handler!` and add `mod flow_commands;`.

Every mutating Tauri command obtains
`let _admission = state.ensure_accepting_work()?;` before its first side effect and
holds the permit until the command returns. This includes Flow save/archive/run/
cancel/retry/coordinate-frame commands and every existing command that writes DB,
credentials, settings, enqueues work, or acquires a `DeviceControlPlane` context.
Catalog/list/get/validate/export and other genuinely read-only projections do not
take a permit. Never acquire a permit only around part of an async mutation.

`flow_save_revision` clones the submitted draft, assigns
`expected_revision.unwrap_or(0) + 1` to the document, compiles that exact revision,
and passes the compiler's canonical bytes/hash to the repository. The repository's
`IMMEDIATE` transaction remains authoritative and returns `RevisionConflict` when
another writer advanced first. Never increment revision after hashing.

- [x] **Step 6: Enforce exit order**

At `RunEvent::Exit`, call `reject_new_work()` first and await
`wait_until_drained()` before stopping runtimes. Then call
`nurture.begin_shutdown()`, `flows.stop_all()`, and `jobs.stop_all()`, followed by
sampler, Flow, jobs, and control cleanup in the approved order. Log each error and
continue cleanup; never early-return after one failure. A command admitted before
the rejection may finish; no command that loses the second atomic check may mutate.

```rust
state.reject_new_work();
tauri::async_runtime::block_on(state.wait_for_mutating_commands());
state.nurture.begin_shutdown();
state.flows.stop_all();
state.jobs.stop_all();
if let Err(error) = tauri::async_runtime::block_on(state.shutdown_background_sampler()) {
    log::error!("background sampler shutdown failed: {error:#}");
}
if let Err(error) = tauri::async_runtime::block_on(state.flows.shutdown()) {
    log::error!("Flow runtime shutdown failed: {error:#}");
}
if let Err(error) = tauri::async_runtime::block_on(state.jobs.shutdown()) {
    log::error!("job queue shutdown failed: {error:#}");
}
if let Err(error) = tauri::async_runtime::block_on(state.control.shutdown_cleanup()) {
    log::error!("device cleanup shutdown failed: {error}");
}
```

- [x] **Step 7: Run tests and commit**

```powershell
cargo fmt --all
cargo test -p riviu-managers-phone flow_commands -- --nocapture
cargo test -p riviu-managers-phone exit_order -- --nocapture
git add apps/desktop/src-tauri/src
git commit -m "feat(flow): expose typed desktop commands"
```

### Task 2: Add Frontend Dependencies, Types, And API Client

**Files:**
- Modify: `apps/desktop/package.json`
- Modify: `apps/desktop/package-lock.json`
- Modify: `apps/desktop/vite.config.ts`
- Create: `apps/desktop/src/test/setup.ts`
- Modify: `apps/desktop/src/types.ts`
- Modify: `apps/desktop/src/api.ts`
- Test: `apps/desktop/src/flowApi.test.ts`

- [x] **Step 1: Install pinned dependencies**

Run:

```powershell
npm --prefix apps/desktop install --save-exact @xyflow/react@12.11.2 lucide-react@1.28.0
npm --prefix apps/desktop install --save-dev --save-exact @testing-library/dom@10.4.1 @testing-library/react@16.3.2 @testing-library/user-event@14.6.1 @testing-library/jest-dom@7.0.0 jsdom@27.4.0 @playwright/test@1.62.0
```

Require Node >=20.19 (the existing Vite 8 baseline). Commit the exact resolved
versions in `package-lock.json`; do not hand-edit the lock.

- [x] **Step 2: Configure jsdom tests**

Change `vite.config.ts` to import `defineConfig` from `vitest/config`, retain the React plugin, and add:

```ts
test: {
  environment: "jsdom",
  setupFiles: ["./src/test/setup.ts"],
  css: true,
},
```

In `src/test/setup.ts`, import `@testing-library/jest-dom/vitest` and stub `ResizeObserver` with observe/unobserve/disconnect methods.

- [x] **Step 3: Add exact TypeScript projections**

Mirror all F0/F1 camelCase Rust models in `types.ts`. Use discriminated unions for `ActionKind`, `EvidenceSpec`, `FlowTargetSelection`, and attempt state. Define `CommandError` with optional `nodeId`, `field`, existing `udid`, and `attemptId`. No `any` is permitted in Flow types. `ActionDefinition` must include the backend-owned required nullable field `disabledReason: string | null`; do not synthesize a parallel UI-only disabled-reason map or widen it to an optional field.

The coordinate and evidence projections must use these exact property names:

```ts
export type JsonValue = string | number | boolean | null | JsonObject | JsonValue[];
export interface JsonObject { [key: string]: JsonValue }

export type ScreenOrientation =
  | "portrait"
  | "portraitUpsideDown"
  | "landscapeLeft"
  | "landscapeRight";

export interface ImageCoordinateTarget {
  x: number;
  y: number;
  imageWidth: number;
  imageHeight: number;
  orientation: ScreenOrientation;
  profileId: string;
}

export interface FlowCoordinateFrame {
  jpegBase64: string;
  imageWidth: number;
  imageHeight: number;
  orientation: ScreenOrientation;
  profileId: string;
}

export type QualifiedElementLocator =
  | { strategy: "accessibilityId"; value: string }
  | { strategy: "className"; value: string };

export type FlowTargetSelection =
  | { mode: "one"; udid: string }
  | { mode: "selected"; udids: string[] }
  | { mode: "allEligible" };
```

- [x] **Step 4: Add API wrappers and invocation tests**

Add one wrapper for every command from Task 1. Mock `@tauri-apps/api/core` and assert command names and camelCase argument keys. For example:

```ts
export async function flowSaveRevision(
  document: FlowDocumentV2,
  expectedRevision: number | null,
) {
  return invoke<FlowRevisionRecord>("flow_save_revision", {
    document,
    expectedRevision,
  });
}

export async function flowCoordinateFrame(udid: string, bundleId: string) {
  return invoke<FlowCoordinateFrame>("flow_coordinate_frame", { udid, bundleId });
}
```

```ts
const startId = "00000000-0000-0000-0000-000000000011";
const endId = "00000000-0000-0000-0000-000000000012";
const documentFixture: FlowDocumentV2 = {
  schemaVersion: 2,
  id: "00000000-0000-0000-0000-000000000010",
  name: "API fixture",
  revision: 5,
  entryNodeId: startId,
  nodes: [
    { id: startId, kind: "start", position: { x: 0, y: 80 }, config: {} },
    { id: endId, kind: "end", position: { x: 320, y: 80 }, config: {} },
  ],
  edges: [{
    id: "00000000-0000-0000-0000-000000000013",
    sourceNodeId: startId,
    sourcePort: "flow",
    targetNodeId: endId,
    targetPort: "flow",
  }],
  viewport: { x: 0, y: 0, zoom: 1 },
};
const revisionFixture: FlowRevisionRecord = {
  document: documentFixture,
  compiledPlan: {
    schemaVersion: 2,
    flowId: documentFixture.id,
    revision: 5,
    nodes: {
      [startId]: { id: startId, kind: "start", config: { kind: "empty" } },
      [endId]: { id: endId, kind: "end", config: { kind: "empty" } },
    },
    executionOrder: [startId, endId],
    contextPlan: {
      requiresExclusive: false,
      requiresUiSession: false,
      requiresStream: false,
      requiresFreshTextSession: false,
      initialBundleId: null,
    },
    actionDefinitionVersions: { start: 1, end: 1 },
    requiredCapabilities: [],
  },
  planHash: "11".repeat(32),
  createdAt: "2026-07-30T00:00:00Z",
};

it("sends a typed save request with optimistic revision", async () => {
  vi.mocked(invoke).mockResolvedValueOnce(revisionFixture);
  await flowSaveRevision(documentFixture, 4);
  expect(invoke).toHaveBeenCalledWith("flow_save_revision", {
    document: documentFixture,
    expectedRevision: 4,
  });
});

it("qualifies a coordinate frame against the launch bundle", async () => {
  const coordinateFrameFixture: FlowCoordinateFrame = {
    jpegBase64: "fixture-jpeg",
    imageWidth: 375,
    imageHeight: 667,
    orientation: "portrait",
    profileId: "11".repeat(32),
  };
  vi.mocked(invoke).mockResolvedValueOnce(coordinateFrameFixture);
  await flowCoordinateFrame("device-a", "com.apple.Preferences");
  expect(invoke).toHaveBeenCalledWith("flow_coordinate_frame", {
    udid: "device-a",
    bundleId: "com.apple.Preferences",
  });
});
```

- [x] **Step 5: Run tests and commit**

```powershell
npm --prefix apps/desktop test -- flowApi.test.ts
npm --prefix apps/desktop run build
git add apps/desktop/package.json apps/desktop/package-lock.json apps/desktop/vite.config.ts apps/desktop/src/test apps/desktop/src/types.ts apps/desktop/src/api.ts apps/desktop/src/flowApi.test.ts
git commit -m "feat(flow): add typed frontend API contracts"
```

### Task 3: Build Pure Draft State, Graph Mapping, And Undo

**Files:**
- Create: `apps/desktop/src/components/flow/editorState.ts`
- Create: `apps/desktop/src/components/flow/editorState.test.ts`
- Create: `apps/desktop/src/components/flow/draftStorage.ts`
- Create: `apps/desktop/src/components/flow/draftStorage.test.ts`

- [x] **Step 1: Write reducer tests**

Cover new Start/End draft, add node between nodes, reconnect, delete executable
node, stable IDs, duplicate with remapped flow/node/edge UUIDs, position-only edits,
config/postcondition edits, bounded 50-entry undo/redo, dirty tracking, server
revision reset, local draft round-trip/version rejection, and serialization that
preserves layout but not React-only selection state. Also cover that every document
mutation increments the document epoch and invalidates validation/compiled state,
a validation completion with an old identity is ignored, save cannot start from an
old compilation, and a save response for an older draft never replaces or marks a
newer draft clean.

```ts
it("duplicates a flow with no shared domain identifiers", () => {
  const source = newFlowDocument("Source");
  const duplicate = duplicateDocument(source, "Copy");
  expect(duplicate.id).not.toBe(source.id);
  expect(duplicate.revision).toBe(0);
  expect(new Set(duplicate.nodes.map((node) => node.id))).not.toEqual(
    new Set(source.nodes.map((node) => node.id)),
  );
  const ids = new Set(duplicate.nodes.map((node) => node.id));
  expect(duplicate.edges.every((edge) =>
    ids.has(edge.sourceNodeId) && ids.has(edge.targetNodeId)
  )).toBe(true);
});

it("rejects a local draft from another storage schema", () => {
  localStorage.setItem("riviu.flowDraft.fixture", JSON.stringify({
    schemaVersion: 99,
    flowId: "fixture",
  }));
  expect(loadDraft("fixture")).toBeNull();
});

it("ignores validation and save completions for an older draft epoch", () => {
  const identity = { requestId: 7, flowId, documentEpoch: 0 };
  const started = reduce(initialEditorState(), {
    type: "validationStarted",
    identity,
  });
  const edited = reduce(started, { type: "renameFlow", name: "Edited" });
  const validated = reduce(edited, {
    type: "validationCompleted",
    identity,
    issues: [],
    compiled: compiledFixture,
  });
  expect(validated.compiled).toBeNull();

  const valid = reduce(reduce(initialEditorState(), {
    type: "validationStarted",
    identity,
  }), {
    type: "validationCompleted",
    identity,
    issues: [],
    compiled: compiledFixture,
  });
  const saving = reduce(valid, { type: "saveStarted", identity });
  const newerDraft = reduce(saving, { type: "renameFlow", name: "Edited during save" });
  const afterStaleSave = reduce(newerDraft, {
    type: "saveCompleted",
    identity,
    record: savedRevisionFixture,
  });
  expect(afterStaleSave.document.name).toBe("Edited during save");
  expect(afterStaleSave.dirty).toBe(true);
});
```

- [x] **Step 2: Run tests red**

```powershell
npm --prefix apps/desktop test -- editorState.test.ts
```

Expected: FAIL because the editor reducer is absent.

- [x] **Step 3: Implement domain/React mapping**

Define `FlowCanvasNode = Node<FlowNodeData, "flowAction">` and `FlowCanvasEdge = Edge`. Keep the domain UUID in both React and domain IDs. `toCanvas` maps positions/config; `toDocument` strips selection, measured dimensions, and React internals.

```ts
export interface FlowNodeData extends Record<string, unknown> {
  kind: ActionKind;
  config: JsonObject;
  postcondition: EvidenceSpec | null;
  issues: FlowValidationIssue[];
}

export type FlowCanvasNode = Node<FlowNodeData, "flowAction">;
export type FlowCanvasEdge = Edge;

export function toCanvas(document: FlowDocumentV2, issues: FlowValidationIssue[]) {
  return {
    nodes: document.nodes.map<FlowCanvasNode>((node) => ({
      id: node.id,
      type: "flowAction",
      position: node.position,
      data: {
        kind: node.kind,
        config: node.config,
        postcondition: node.postcondition,
        issues: issues.filter((issue) => issue.nodeId === node.id),
      },
    })),
    edges: document.edges.map<FlowCanvasEdge>((edge) => ({
      id: edge.id,
      source: edge.sourceNodeId,
      sourceHandle: edge.sourcePort,
      target: edge.targetNodeId,
      targetHandle: edge.targetPort,
    })),
  };
}

export function withCanvasLayout(
  document: FlowDocumentV2,
  nodes: FlowCanvasNode[],
  edges: FlowCanvasEdge[],
): FlowDocumentV2 {
  const positionById = new Map(nodes.map((node) => [node.id, node.position]));
  return {
    ...document,
    nodes: document.nodes.map((node) => ({
      ...node,
      position: positionById.get(node.id) ?? node.position,
    })),
    edges: edges.map((edge) => ({
      id: edge.id,
      sourceNodeId: edge.source,
      sourcePort: edge.sourceHandle ?? "flow",
      targetNodeId: edge.target,
      targetPort: edge.targetHandle ?? "flow",
    })),
  };
}
```

- [x] **Step 4: Implement reducer and history**

Use this state shape:

```ts
export interface DocumentRequestIdentity {
  requestId: number;
  flowId: string;
  documentEpoch: number;
}

export interface ValidatedCompilation {
  identity: DocumentRequestIdentity;
  value: CompiledRevision;
}

export type FlowEditorNotice = {
  code: "SaveCompletedForOlderDraft";
  savedRevision: number;
} | null;

export interface FlowEditorState {
  document: FlowDocumentV2;
  past: FlowDocumentV2[];
  future: FlowDocumentV2[];
  selectedNodeId: string | null;
  dirty: boolean;
  documentEpoch: number;
  validation: FlowValidationIssue[];
  validationRequest: DocumentRequestIdentity | null;
  compiled: ValidatedCompilation | null;
  saveRequest: DocumentRequestIdentity | null;
  notice: FlowEditorNotice;
}
```

Every semantic/layout mutation, including name, viewport, node position, edge,
config, postcondition, undo, redo, import, duplicate, new-flow, and server-document
replacement, advances a monotonically increasing `documentEpoch`. A mutation pushes
one cloned document to `past` when applicable, clears `future`, caps `past` at 50,
and atomically sets `validation=[]`, `validationRequest=null`, and `compiled=null`.
Never reset the epoch when switching documents, because an old response may have the
same flow ID and revision. Selection and validation actions do not enter history.
Do not mutate arrays in place.

The reducer accepts `validationStarted` only for the current flow ID and epoch. A
`validationCompleted` action carries the identical request object and is applied
only when its three scalar fields equal `state.validationRequest` and it still names
the current flow/epoch; never compare object references. Otherwise it is a no-op. A successful result stores
`{identity, value: compiled}`. This identity, not an empty issue array, is the proof
that compilation belongs to the visible draft.

`saveStarted` must carry the current `compiled.identity`; the reducer rejects it
unless that identity still matches the current document and no save is active.
`saveCompleted` may replace the document, clear history/draft storage, and mark it
clean only when its identity still matches both `saveRequest` and the current
flow/epoch. If the user edited while save was in flight, preserve the newer draft,
keep it dirty, clear `saveRequest`, refetch the Flow summary, and surface
`SaveCompletedForOlderDraft` with the saved revision. Never silently apply the old
record or clear local storage. A later explicit save may receive `RevisionConflict`
and follows the existing reload/duplicate resolution path. Document mutations do
not erase an in-flight `saveRequest`; it is retained only to classify the eventual
response and never proves that the current draft is valid. `saveFailed` likewise
clears only an exactly matching request. Store the stale-save result in `notice`;
the workspace renders it in the diagnostics band and refetches summaries from an
effect keyed by `notice.savedRevision`, keeping side effects out of the reducer.

`duplicateDocument` generates a new flow UUID, revision 0, new node/edge UUIDs, and
rewrites every edge/entry reference through a node-ID map. `draftStorage` writes only
`{schemaVersion:1, flowId, baseRevision, document, savedAt}` to
`localStorage["riviu.flowDraft.<flowId>"]`; malformed or mismatched versions return
`null`. Debounce writes by 300 ms and clear the local draft only after an
identity-current successful save.

- [x] **Step 5: Run tests and commit**

```powershell
npm --prefix apps/desktop test -- editorState.test.ts
npm --prefix apps/desktop run build
git add apps/desktop/src/components/flow/editorState.ts apps/desktop/src/components/flow/editorState.test.ts
git add apps/desktop/src/components/flow/draftStorage.ts apps/desktop/src/components/flow/draftStorage.test.ts
git commit -m "feat(flow): add deterministic visual draft state"
```

### Task 4: Build Palette, Controlled Canvas, And Custom Nodes

**Files:**
- Create: `apps/desktop/src/components/flow/FlowWorkspace.tsx`
- Create: `apps/desktop/src/components/flow/FlowPalette.tsx`
- Create: `apps/desktop/src/components/flow/FlowCanvas.tsx`
- Create: `apps/desktop/src/components/flow/FlowActionNode.tsx`
- Create: `apps/desktop/src/components/flow/FlowWorkspace.test.tsx`

- [x] **Step 1: Write interaction tests**

Render a catalog fixture, drag Wait onto the canvas, connect Start -> Wait -> End, select/delete/reorder nodes, zoom via controls, and verify disabled actions show the backend capability reason. Assert Start/End cannot be deleted and raw action kinds never render.

```tsx
const catalogFixture: ActionDefinition[] = [{
  kind: "wait",
  schemaVersion: 1,
  label: "Wait",
  disabledReason: null,
  category: "timing",
  configSchema: {
    type: "object",
    additionalProperties: false,
    required: ["durationMs"],
    properties: {
      durationMs: { type: "integer", minimum: 1, maximum: 60_000 },
    },
  },
  inputPorts: [{ name: "flow", valueType: "flow", required: true }],
  outputPorts: [{ name: "flow", valueType: "flow", required: true }],
  requiredCapabilities: [],
  resourceClass: "pureDesktop",
  sideEffectClass: "none",
  evidenceRequirement: "none",
  allowedEvidence: [],
  qualifiedDetectorIds: [],
  reconciliationPolicy: "none",
  defaultTimeoutMs: 60_000,
  retryPolicy: "never",
}];

vi.mock("../../api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api")>();
  return {
    ...actual,
    flowActionCatalog: vi.fn(),
    flowList: vi.fn(),
    flowValidate: vi.fn(),
  };
});

function FlowWorkspaceApiFixture({ catalog }: { catalog: ActionDefinition[] }) {
  vi.mocked(flowActionCatalog).mockResolvedValue(catalog);
  vi.mocked(flowList).mockResolvedValue([]);
  vi.mocked(flowValidate).mockImplementation(() => new Promise<CompiledRevision>(() => undefined));
  return <FlowWorkspace devices={[]} selectedUdids={[]} onDirtyChange={vi.fn()} />;
}

function dataTransfer(type: string, value: string): DataTransfer {
  const values = new Map<string, string>([[type, value]]);
  return {
    dropEffect: "none",
    effectAllowed: "all",
    files: [] as unknown as FileList,
    items: [] as unknown as DataTransferItemList,
    types: [type],
    clearData: (format?: string) => {
      if (format) values.delete(format);
      else values.clear();
    },
    getData: (format: string) => values.get(format) ?? "",
    setData: (format: string, data: string) => { values.set(format, data); },
    setDragImage: () => undefined,
  };
}

it("adds a typed Wait node from the palette", async () => {
  const user = userEvent.setup();
  render(<FlowWorkspaceApiFixture catalog={catalogFixture} />);
  const wait = screen.getByRole("button", { name: "Wait" });
  fireEvent.dragStart(wait, {
    dataTransfer: dataTransfer("application/riviu-flow-action", "wait"),
  });
  fireEvent.drop(screen.getByTestId("flow-canvas"), {
    clientX: 420,
    clientY: 240,
    dataTransfer: dataTransfer("application/riviu-flow-action", "wait"),
  });
  expect(await screen.findByText("Wait", { selector: ".flow-node-title" })).toBeVisible();
  await user.click(screen.getByLabelText("Undo"));
  expect(screen.queryByText("Wait", { selector: ".flow-node-title" })).toBeNull();
});

it("uses the backend catalog disabled reason without a parallel map", () => {
  const reason = "Requires a qualified text-capable artifact";
  render(<FlowWorkspaceApiFixture catalog={[{
    ...catalogFixture[0],
    disabledReason: reason,
  }]} />);
  const wait = screen.getByRole("button", { name: "Wait" });
  expect(wait).toBeDisabled();
  expect(wait).toHaveAttribute("title", reason);
});
```

- [x] **Step 2: Run tests red**

```powershell
npm --prefix apps/desktop test -- FlowWorkspace.test.tsx
```

Expected: FAIL because workspace components are absent.

- [x] **Step 3: Implement the action palette**

Group catalog entries by App, Input, Timing, and Evidence. Use native drag payload `application/riviu-flow-action` containing only the stable action kind. Render disabled entries as buttons with `disabled`, `aria-disabled`, and a title directly from `ActionDefinition.disabledReason`. The catalog response is authoritative; do not derive a second reasons prop or lookup.

```tsx
const CATEGORY_ORDER: Exclude<ActionCategory, "control">[] = ["app", "input", "timing", "evidence"];
const categoryLabels: Record<Exclude<ActionCategory, "control">, string> = {
  app: "App",
  input: "Input",
  timing: "Timing",
  evidence: "Evidence",
};

function beginActionDrag(event: React.DragEvent, action: ActionDefinition) {
  event.dataTransfer.effectAllowed = "copy";
  event.dataTransfer.setData("application/riviu-flow-action", action.kind);
}

return CATEGORY_ORDER.map((category) => (
  <section key={category} aria-label={category}>
    <h3>{categoryLabels[category]}</h3>
    {catalog.filter((action) => action.category === category).map((action) => (
      <button
        key={action.kind}
        type="button"
        draggable={!action.disabledReason}
        disabled={Boolean(action.disabledReason)}
        aria-disabled={Boolean(action.disabledReason)}
        title={action.disabledReason ?? action.label}
        onDragStart={(event) => beginActionDrag(event, action)}
      >
        {action.label}
      </button>
    ))}
  </section>
));
```

- [x] **Step 4: Implement the controlled canvas**

Use `ReactFlow`, `Background`, `Controls`, and `MiniMap`; pass controlled nodes/edges and explicit change/connect handlers. Use `screenToFlowPosition` on drop. Set `nodeTypes` outside the component. Enable grid snap at `[16,16]`, fit view only on first load, and keep the parent height fixed by CSS rather than inline viewport math.

Expose stable test IDs `flow-toolbar`, `flow-palette`, `flow-canvas`,
`flow-inspector`, and `flow-monitor` on their outer semantic regions; these are also
the boundaries used by F3 overlap checks.

When exactly one edge is selected, dropping a node atomically replaces that edge
with source -> new and new -> target. With no selected edge, apply the same split
when the drop is within 24 canvas pixels of one edge. A drop elsewhere creates an
unconnected node and backend validation reports it. This is the deterministic path
used by the Playwright release workflow.

```tsx
const nodeTypes: NodeTypes = { flowAction: FlowActionNode };

return (
  <div className="flow-canvas" data-testid="flow-canvas">
    <ReactFlow
      nodes={nodes}
      edges={edges}
      nodeTypes={nodeTypes}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      onConnect={onConnect}
      onDrop={onDrop}
      onDragOver={(event) => {
        event.preventDefault();
        event.dataTransfer.dropEffect = "copy";
      }}
      snapToGrid
      snapGrid={[16, 16]}
      fitView
    >
      <Background gap={16} />
      <MiniMap pannable zoomable />
      <Controls showInteractive={false} />
    </ReactFlow>
  </div>
);
```

`onDrop` rejects any MIME value absent from the current catalog, converts with
`screenToFlowPosition`, then dispatches one reducer action containing the new UUID,
position, and selected/nearest edge ID. The reducer performs the atomic edge split.

- [x] **Step 5: Implement custom nodes**

Render compact nodes with one input/output flow handle, action label, concise config
summary, error badge, and selected state. Start has output only; End has input only.
Import the named Lucide icons, including `PowerOff` for the F1-enabled Terminate App,
and add tooltips on unfamiliar actions.

```tsx
const actionPresentation: Partial<Record<ActionKind, { label: string; icon: LucideIcon }>> = {
  start: { label: "Start", icon: CirclePlay },
  end: { label: "End", icon: CircleStop },
  launchApp: { label: "Launch App", icon: Rocket },
  terminateApp: { label: "Terminate App", icon: PowerOff },
  wait: { label: "Wait", icon: Timer },
  tap: { label: "Tap", icon: MousePointerClick },
  swipe: { label: "Swipe", icon: MoveUp },
  typeText: { label: "Type Text", icon: Keyboard },
  screenshot: { label: "Screenshot", icon: Camera },
  home: { label: "Home", icon: House },
  assertVisible: { label: "Assert Visible", icon: ScanSearch },
};

function summarizeAction(kind: ActionKind, config: JsonObject): string {
  const text = (key: string) => {
    const value = config[key];
    return typeof value === "string" ? value : "";
  };
  const number = (value: JsonValue | undefined, key: string) => {
    if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
    const field = value[key];
    return typeof field === "number" ? field : null;
  };
  switch (kind) {
    case "launchApp":
    case "terminateApp": return text("bundleId");
    case "wait": return typeof config.durationMs === "number" ? config.durationMs + " ms" : "";
    case "tap": return text("accessibilityId") ||
      [number(config.point, "x"), number(config.point, "y")].filter((value) => value !== null).join(", ");
    case "swipe": return "Swipe " + (typeof config.durationMs === "number" ? config.durationMs + " ms" : "");
    case "typeText": return text("text").length + " characters";
    case "screenshot": return text("label");
    case "assertVisible": return text("accessibilityId");
    default: return "";
  }
}

export function FlowActionNode({ data, selected }: NodeProps<FlowCanvasNode>) {
  const presentation = actionPresentation[data.kind];
  if (!presentation) return null;
  const Icon = presentation.icon;
  return (
    <div className="flow-node" data-selected={selected || undefined}>
      {data.kind !== "start" && <Handle type="target" position={Position.Left} id="flow" />}
      <div className="flow-node-heading">
        <Icon aria-hidden="true" size={16} />
        <span className="flow-node-title">{presentation.label}</span>
        {data.issues.length > 0 && (
          <span className="flow-node-error" title={data.issues[0]?.message}>
            {data.issues.length}
          </span>
        )}
      </div>
      <div className="flow-node-summary">{summarizeAction(data.kind, data.config)}</div>
      {data.kind !== "end" && <Handle type="source" position={Position.Right} id="flow" />}
    </div>
  );
}
```

- [x] **Step 6: Run tests and commit**

```powershell
npm --prefix apps/desktop test -- FlowWorkspace.test.tsx
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
git add apps/desktop/src/components/flow
git commit -m "feat(flow): add controlled drag-and-drop canvas"
```

### Task 5: Add Typed Inspector, Validation, Import, And JSON View

**Files:**
- Create: `apps/desktop/src/components/flow/FlowInspector.tsx`
- Create: `apps/desktop/src/components/flow/FlowDiagnostics.tsx`
- Create: `apps/desktop/src/components/flow/FlowImportDialog.tsx`
- Create: `apps/desktop/src/components/flow/FlowJsonDialog.tsx`
- Create: `apps/desktop/src/components/flow/FlowCoordinatePicker.tsx`
- Create: `apps/desktop/src/components/flow/FlowInspector.test.tsx`
- Modify: `apps/desktop/src/components/flow/FlowWorkspace.tsx`

- [x] **Step 1: Write inspector/import tests**

Test Launch/Terminate Bundle ID, Terminate `ProcessAbsent` bundle equality, bounded
Wait, finite Tap/Swipe inputs, mutually exclusive Tap target
modes, Type Text read-back locator (`accessibilityId` or `className` only), screenshot label, evidence fields,
coordinate-frame click scaling, stored width/height/orientation/profile, field/node
error mapping, unsupported legacy diagnostics, JSON export, malformed JSON
rejection, and no save when backend validation fails.

```tsx
it("stores click coordinates in original frame space", async () => {
  const onPick = vi.fn();
  const frame: FlowCoordinateFrame = {
    jpegBase64: "fixture-jpeg",
    imageWidth: 375,
    imageHeight: 667,
    orientation: "portrait",
    profileId: "11".repeat(32),
  };
  render(<FlowCoordinatePicker frame={frame} onPick={onPick} />);
  const image = screen.getByRole("img", { name: "Device frame" });
  vi.spyOn(image, "getBoundingClientRect").mockReturnValue({
    x: 100, y: 50, left: 100, top: 50, right: 475, bottom: 717,
    width: 375, height: 667, toJSON: () => ({}),
  } as DOMRect);
  fireEvent.click(image, { clientX: 287.5, clientY: 383.5 });
  expect(onPick).toHaveBeenCalledWith({
    x: 187.5,
    y: 333.5,
    imageWidth: 375,
    imageHeight: 667,
    orientation: "portrait",
    profileId: "11".repeat(32),
  });
});
```

- [x] **Step 2: Run tests red**

```powershell
npm --prefix apps/desktop test -- FlowInspector.test.tsx
```

Expected: FAIL because inspector/dialog components are absent.

- [x] **Step 3: Implement schema-driven fields with explicit widgets**

Render strings as text inputs, numeric bounds as number inputs, enums as select menus, binary options as checkboxes, and coordinates as paired numeric inputs. Handle each JSON-schema property type explicitly and return `UnsupportedFieldSchema` in development for unknown schema shapes. Evidence options come only from the action's backend `allowedEvidence`; `qualifiedFramePredicate` also requires a detector ID in `qualifiedDetectorIds`. Release 1 publishes no detector IDs and never accepts free-form evidence JSON.

```tsx
interface JsonSchema {
  type: "object" | "string" | "number" | "integer" | "boolean";
  title?: string;
  minimum?: number;
  maximum?: number;
  enum?: string[];
  properties?: Record<string, JsonSchema>;
  required?: string[];
}

interface SchemaFieldProps {
  schema: JsonSchema;
  value: JsonValue | undefined;
  onChange: (value: JsonValue) => void;
}

function isJsonObject(value: JsonValue | undefined): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function SchemaObjectFields({ schema, value, onChange }: {
  schema: JsonSchema;
  value: JsonObject;
  onChange: (value: JsonValue) => void;
}) {
  return Object.entries(schema.properties ?? {}).map(([name, child]) => (
    <label key={name}>
      <span>{child.title ?? name}</span>
      <SchemaField schema={child} value={value[name]} onChange={(next) =>
        onChange({ ...value, [name]: next })} />
    </label>
  ));
}

function SchemaField({ schema, value, onChange }: SchemaFieldProps) {
  if (schema.enum) {
    return (
      <select value={String(value ?? "")} onChange={(event) => onChange(event.target.value)}>
        {schema.enum.map((option) => <option key={option} value={option}>{option}</option>)}
      </select>
    );
  }
  switch (schema.type) {
    case "string":
      return <input type="text" value={String(value ?? "")} onChange={(event) => onChange(event.target.value)} />;
    case "number":
    case "integer":
      return <input type="number" min={schema.minimum} max={schema.maximum}
        value={typeof value === "number" && Number.isFinite(value) ? value : ""}
        onChange={(event) => {
          const next = event.target.valueAsNumber;
          if (Number.isFinite(next)) onChange(next);
        }} />;
    case "boolean":
      return <input type="checkbox" checked={value === true}
        onChange={(event) => onChange(event.target.checked)} />;
    case "object":
      return <SchemaObjectFields schema={schema} value={isJsonObject(value) ? value : {}} onChange={onChange} />;
    default:
      throw new Error("UnsupportedFieldSchema");
  }
}
```

Use dedicated `CoordinateFields` and `ReadBackLocatorFields` for their nested
schemas so orientation is a menu, x/y are finite number inputs, profile ID is
read-only after picking, and locator strategy is a two-option segmented control.

- [x] **Step 4: Implement backend validation mapping**

Debounce `flow_validate` by 250 ms after edits. Give every request a monotonically
increasing ID plus the reducer's current flow ID and `documentEpoch`; clone the
document before starting the request. Dispatch completions unconditionally and let
the reducer perform the authoritative identity check. A component-local ref check
alone is insufficient because undo, import, flow switching, or a save completion
can replace the draft while an RPC is in flight. Map `nodeId/field` to inspector and
node badges; show document-level errors in the bottom diagnostics band.

```tsx
const validationSequence = useRef(0);
useEffect(() => {
  const identity: DocumentRequestIdentity = {
    requestId: ++validationSequence.current,
    flowId: state.document.id,
    documentEpoch: state.documentEpoch,
  };
  const snapshot = structuredClone(state.document);
  dispatch({ type: "validationStarted", identity });
  const timer = window.setTimeout(() => {
    flowValidate(snapshot).then(
      (compiled) => {
        dispatch({ type: "validationCompleted", identity, issues: [], compiled });
      },
      (error: unknown) => {
        dispatch({
          type: "validationCompleted",
          identity,
          issues: normalizeFlowIssues(error),
          compiled: null,
        });
      },
    );
  }, 250);
  return () => window.clearTimeout(timer);
}, [state.document, state.documentEpoch]);
```

`normalizeFlowIssues` accepts only arrays of objects containing string `code` and
`message`; it copies optional string `nodeId`/`field`/`udid`/`attemptId`. Any other
rejection becomes one document issue `{code:"ValidationTransportFailed",
message:String(error)}`.

- [x] **Step 5: Implement import and advanced JSON**

Legacy import shows every diagnostic and applies the returned document only when it is non-null. Flow JSON import parses locally, then calls backend validation before replacing the draft. Export uses `flow_export`. The JSON dialog is diagnostic/advanced mode and never auto-saves.

The coordinate picker calls `flow_coordinate_frame` for one selected device and
the bundle ID from the first compiled Launch App,
renders the uncropped frame at `object-fit: contain`, converts the click through
the displayed image rectangle, and stores `ImageCoordinateTarget` with original
image dimensions, orientation, and profile ID. It never reads coordinates from
the outer modal box.

```ts
export function projectContainedImageClick(
  frame: FlowCoordinateFrame,
  rect: DOMRect,
  clientX: number,
  clientY: number,
): ImageCoordinateTarget | null {
  const scale = Math.min(rect.width / frame.imageWidth, rect.height / frame.imageHeight);
  const shownWidth = frame.imageWidth * scale;
  const shownHeight = frame.imageHeight * scale;
  const left = rect.left + (rect.width - shownWidth) / 2;
  const top = rect.top + (rect.height - shownHeight) / 2;
  if (clientX < left || clientX > left + shownWidth || clientY < top || clientY > top + shownHeight) {
    return null;
  }
  return {
    x: (clientX - left) / scale,
    y: (clientY - top) / scale,
    imageWidth: frame.imageWidth,
    imageHeight: frame.imageHeight,
    orientation: frame.orientation,
    profileId: frame.profileId,
  };
}

async function importFlowJson(raw: string): Promise<FlowDocumentV2> {
  if (new TextEncoder().encode(raw).byteLength > 1_048_576) {
    throw new Error("FlowImportTooLarge");
  }
  const document: unknown = JSON.parse(raw);
  assertFlowDocumentShape(document);
  await flowValidate(document);
  return document;
}

function assertFlowDocumentShape(value: unknown): asserts value is FlowDocumentV2 {
  if (typeof value !== "object" || value === null) throw new Error("FlowJsonObjectRequired");
  const candidate = value as Record<string, unknown>;
  if (candidate.schemaVersion !== 2
      || typeof candidate.id !== "string"
      || typeof candidate.name !== "string"
      || !Array.isArray(candidate.nodes)
      || !Array.isArray(candidate.edges)) {
    throw new Error("FlowJsonShapeInvalid");
  }
}
```

Legacy import calls `flowImportLegacy(raw)` and replaces the draft only when
`result.document !== null && result.diagnostics.length === 0`. Flow export displays
only the string returned by `flowExport`; neither dialog invokes save.

- [x] **Step 6: Run tests and commit**

```powershell
npm --prefix apps/desktop test -- FlowInspector.test.tsx
npm --prefix apps/desktop run build
git add apps/desktop/src/components/flow
git commit -m "feat(flow): add typed inspector and import diagnostics"
```

### Task 6: Add Revision Toolbar, Device Selection, And Run Monitor

**Files:**
- Create: `apps/desktop/src/components/flow/FlowToolbar.tsx`
- Create: `apps/desktop/src/components/flow/FlowRunDialog.tsx`
- Create: `apps/desktop/src/components/flow/FlowRunMonitor.tsx`
- Create: `apps/desktop/src/components/flow/FlowRunMonitor.test.tsx`
- Modify: `apps/desktop/src/components/flow/FlowWorkspace.tsx`

- [x] **Step 1: Write run workflow tests**

Cover explicit Save revision, stale revision conflict, One/Selected/AllEligible payloads, zero target prevention, queued/running/partial terminal projection, per-device node attempts, artifacts, cancel, retry eligibility, disabled retry for Uncertain Tap/Swipe/Type, and event invalidation followed by refetch.

```tsx
const at = "2026-07-30T00:00:00Z";
const onRun = vi.fn();
const deviceFixtures: DeviceInfo[] = [
  {
    udid: "a", name: "Device A", model: "iPhone10,1", iosVersion: "16.7.15",
    connection: "mock", status: "ready", wdaReady: true,
  },
  {
    udid: "b", name: "Device B", model: "iPhone10,1", iosVersion: "16.7.15",
    connection: "mock", status: "ready", wdaReady: true,
  },
];
const runWithUncertainTap: FlowRunDetail = {
  run: {
    id: "00000000-0000-0000-0000-000000000301",
    flowId: "00000000-0000-0000-0000-000000000100",
    flowRevision: 1,
    planSha256: "44".repeat(32),
    selection: {
      requested: { mode: "one", udid: "a" },
      targetUdids: ["a"],
    },
    state: "partial",
    eventRevision: 3,
    error: null,
    createdAt: at,
    updatedAt: at,
  },
  deviceRuns: [{
    id: "00000000-0000-0000-0000-000000000302",
    runId: "00000000-0000-0000-0000-000000000301",
    udid: "a",
    state: "failed",
    capabilitySnapshot: null,
    releaseProof: null,
    error: null,
    startedAt: at,
    finishedAt: at,
  }],
  attempts: [{
    id: "00000000-0000-0000-0000-000000000303",
    deviceRunId: "00000000-0000-0000-0000-000000000302",
    nodeId: "00000000-0000-0000-0000-000000000103",
    actionKind: "tap",
    attemptNo: 1,
    sideEffectClass: "ambiguousUi",
    state: "uncertain",
    canonicalInput: null,
    evidenceBaseline: null,
    evidenceResult: null,
    retryAllowed: false,
    error: null,
    startedAt: at,
    updatedAt: at,
    finishedAt: at,
  }],
  artifacts: [],
};

it("does not offer retry for an uncertain tap", () => {
  render(<FlowRunMonitor run={runWithUncertainTap} onCancel={vi.fn()} onRetry={vi.fn()} />);
  expect(screen.getByText("Uncertain")).toBeVisible();
  expect(screen.queryByRole("button", { name: "Retry Tap" })).toBeNull();
});

it("builds Selected from the existing device selection", async () => {
  const user = userEvent.setup();
  render(<FlowRunDialog devices={deviceFixtures} selectedUdids={["a", "b"]} onRun={onRun} />);
  await user.click(screen.getByRole("radio", { name: "Selected" }));
  await user.click(screen.getByRole("button", { name: "Run on devices" }));
  expect(onRun).toHaveBeenCalledWith({ mode: "selected", udids: ["a", "b"] });
});
```

- [x] **Step 2: Run tests red**

```powershell
npm --prefix apps/desktop test -- FlowRunMonitor.test.tsx
```

Expected: FAIL because toolbar/dialog/monitor are absent.

- [x] **Step 3: Implement toolbar commands**

Use Lucide Save, Play, Upload, Download, Copy, Archive, Plus, Undo2, Redo2,
CheckCircle, and Braces icons. The toolbar includes a flow selector plus New,
Duplicate, and Archive commands. Save requires a successful compiled result whose
identity matches the current reducer flow/epoch, no validation request in flight,
and no save in flight; it sends `expectedRevision`. Duplicate uses Task 3's ID remap
and creates revision 1 on save. Run requires a saved, non-dirty revision with the
same current compilation proof. Every unfamiliar icon has a title and accessible
label.

Validate opens a compact compile preview listing `ContextPlan`, static capability
IDs, and non-null `disabledReason` values read directly from the backend action
catalog. It does not claim live device eligibility before runtime preflight.

```tsx
interface FlowToolbarProps {
  flows: FlowSummary[];
  currentFlowId: string | null;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  compiled: CompiledRevision | null;
  issues: FlowValidationIssue[];
  catalog: ActionDefinition[];
  validationPending: boolean;
  savePending: boolean;
  onSelectFlow: (id: string) => void;
  onNew: () => void;
  onDuplicate: () => void;
  onArchive: () => void;
  onSave: () => void;
  onRun: () => void;
  onImport: () => void;
  onExport: () => void;
  onJson: () => void;
  onUndo: () => void;
  onRedo: () => void;
}

function IconCommand({
  label,
  disabled = false,
  onClick,
  children,
}: React.PropsWithChildren<{
  label: string;
  disabled?: boolean;
  onClick: () => void;
}>) {
  return (
    <button type="button" className="flow-icon-command" title={label}
      aria-label={label} disabled={disabled} onClick={onClick}>
      {children}
    </button>
  );
}

export function FlowToolbar(props: FlowToolbarProps) {
  const [previewOpen, setPreviewOpen] = useState(false);
  const disabledActions = props.catalog.filter(
    (action): action is ActionDefinition & { disabledReason: string } =>
      action.disabledReason !== null,
  );
  const canSave = props.dirty && props.compiled !== null &&
    props.issues.length === 0 && !props.validationPending && !props.savePending;
  const canRun = !props.dirty && props.compiled !== null &&
    props.currentFlowId !== null && !props.validationPending && !props.savePending;
  return (
    <header className="flow-toolbar" data-testid="flow-toolbar">
      <select aria-label="Flow" value={props.currentFlowId ?? ""}
        onChange={(event) => props.onSelectFlow(event.target.value)}>
        <option value="" disabled>Select flow</option>
        {props.flows.map((flow) => (
          <option key={flow.id} value={flow.id}>{flow.name}</option>
        ))}
      </select>
      <IconCommand label="New flow" onClick={props.onNew}><Plus size={16} /></IconCommand>
      <IconCommand label="Duplicate flow" disabled={!props.currentFlowId}
        onClick={props.onDuplicate}><Copy size={16} /></IconCommand>
      <IconCommand label="Archive flow" disabled={!props.currentFlowId}
        onClick={props.onArchive}><Archive size={16} /></IconCommand>
      <span className="flow-toolbar-separator" />
      <IconCommand label="Undo" disabled={!props.canUndo} onClick={props.onUndo}>
        <Undo2 size={16} />
      </IconCommand>
      <IconCommand label="Redo" disabled={!props.canRedo} onClick={props.onRedo}>
        <Redo2 size={16} />
      </IconCommand>
      <IconCommand label="Save revision" disabled={!canSave} onClick={props.onSave}>
        <Save size={16} />
      </IconCommand>
      <IconCommand label="Validate flow" onClick={() => setPreviewOpen(true)}>
        <CheckCircle size={16} />
      </IconCommand>
      <IconCommand label="Import flow" onClick={props.onImport}><Upload size={16} /></IconCommand>
      <IconCommand label="Export flow" disabled={!props.currentFlowId}
        onClick={props.onExport}><Download size={16} /></IconCommand>
      <IconCommand label="View JSON" onClick={props.onJson}><Braces size={16} /></IconCommand>
      <button type="button" className="flow-run-command" disabled={!canRun}
        onClick={props.onRun}><Play size={16} />Run flow</button>
      {previewOpen && (
        <section role="dialog" aria-label="Compile preview" className="flow-compile-preview">
          <strong>{props.compiled ? "Valid" : "Invalid"}</strong>
          <code>{props.compiled ? JSON.stringify(props.compiled.plan.contextPlan) : "No context plan"}</code>
          <ul>{props.compiled?.plan.requiredCapabilities.map((id) => <li key={id}>{id}</li>)}</ul>
          <ul>{disabledActions.map((action) => (
            <li key={action.kind}>{action.label}: {action.disabledReason}</li>
          ))}</ul>
          <button type="button" onClick={() => setPreviewOpen(false)}>Close</button>
        </section>
      )}
    </header>
  );
}
```

The workspace exposes `state.compiled?.value` to the toolbar only when its embedded
identity equals the current flow ID and epoch. The save callback clones the document,
dispatches `saveStarted` with that exact identity, and calls `flowSaveRevision` with
the clone and clone.revision (or null for a new flow). It dispatches
`saveCompleted(identity, record)` rather than applying the record directly. The
reducer clears the local draft only for an exact current identity; an older response
follows Task 3's `SaveCompletedForOlderDraft` path. A `RevisionConflict` clears the
matching save request, leaves the draft dirty, and surfaces the server's actual
revision.

- [x] **Step 4: Implement target selection**

Use a segmented control for One, Selected, and All eligible. One uses a device
select; Selected uses existing selected UDIDs; AllEligible shows only its status
label until runtime snapshot, without instructional prose. Disable Run for empty
One/Selected.

```tsx
type RunMode = FlowTargetSelection["mode"];

export function targetSelection(
  mode: RunMode,
  oneUdid: string,
  selectedUdids: string[],
): FlowTargetSelection | null {
  if (mode === "one") {
    return oneUdid ? { mode: "one", udid: oneUdid } : null;
  }
  if (mode === "selected") {
    const udids = [...new Set(selectedUdids)].sort();
    return udids.length > 0 ? { mode: "selected", udids } : null;
  }
  return { mode: "allEligible" };
}

export function FlowRunDialog({
  devices,
  selectedUdids,
  onRun,
}: {
  devices: DeviceInfo[];
  selectedUdids: string[];
  onRun: (selection: FlowTargetSelection) => void;
}) {
  const [mode, setMode] = useState<RunMode>("selected");
  const [oneUdid, setOneUdid] = useState(devices[0]?.udid ?? "");
  const selection = targetSelection(mode, oneUdid, selectedUdids);
  return (
    <section role="dialog" aria-label="Run flow">
      <div className="segmented" role="radiogroup" aria-label="Targets">
        {(["one", "selected", "allEligible"] as const).map((value) => (
          <label key={value}>
            <input type="radio" name="flow-target-mode" value={value}
              checked={mode === value} onChange={() => setMode(value)} />
            <span>{value === "one" ? "One" : value === "selected" ? "Selected" : "All eligible"}</span>
          </label>
        ))}
      </div>
      {mode === "one" && (
        <select aria-label="Device" value={oneUdid}
          onChange={(event) => setOneUdid(event.target.value)}>
          {devices.map((device) => (
            <option key={device.udid} value={device.udid}>{device.name}</option>
          ))}
        </select>
      )}
      {mode === "selected" && <output>{selectedUdids.length} selected</output>}
      {mode === "allEligible" && <output>Preflight pending</output>}
      <button type="button" disabled={selection === null}
        onClick={() => selection && onRun(selection)}>Run on devices</button>
    </section>
  );
}
```

- [x] **Step 5: Implement monitor and event refresh**

```tsx
function flowRunEvent(value: unknown): { runId: string; revision: number } | null {
  if (typeof value !== "object" || value === null) return null;
  const event = value as Record<string, unknown>;
  return event.type === "flowRunUpdated"
    && typeof event.runId === "string"
    && typeof event.revision === "number"
      ? { runId: event.runId, revision: event.revision }
      : null;
}

function attemptDurationMs(attempt: FlowNodeAttemptRecord): number {
  if (!attempt.startedAt || !attempt.finishedAt) return 0;
  const start = Date.parse(attempt.startedAt);
  const finish = Date.parse(attempt.finishedAt);
  return Number.isFinite(start) && Number.isFinite(finish) && finish >= start
    ? finish - start
    : 0;
}

function formatEvidence(value: JsonValue | null): string {
  if (value === null) return "";
  const encoded = JSON.stringify(value);
  return encoded.length <= 160 ? encoded : encoded.slice(0, 157) + "...";
}

function displayFlowState(value: string): string {
  return value.length === 0 ? value : value.charAt(0).toUpperCase() + value.slice(1);
}

export function FlowRunMonitor({
  run,
  onCancel,
  onRetry,
  onOpenArtifact = () => undefined,
}: {
  run: FlowRunDetail;
  onCancel: (runId: string) => void;
  onRetry: (attemptId: string) => void;
  onOpenArtifact?: (artifactId: string) => void;
}) {
  const [detail, setDetail] = useState(run);
  useEffect(() => setDetail(run), [run]);
  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void listenRiviuEvents((payload) => {
      const event = flowRunEvent(payload);
      if (!event || event.runId !== detail.run.id
          || event.revision <= detail.run.eventRevision) return;
      void flowGetRun(event.runId).then((next) => {
        if (!disposed && next && next.run.eventRevision >= event.revision) setDetail(next);
      });
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
    };
  }, [detail.run.id, detail.run.eventRevision]);

  const artifacts = new Map(detail.artifacts.map((item) => [item.attemptId, item]));
  return (
    <section className="flow-monitor" data-testid="flow-monitor">
      <header>
        <strong>{displayFlowState(detail.run.state)}</strong>
        <button type="button" onClick={() => onCancel(detail.run.id)}
          disabled={!["queued", "running"].includes(detail.run.state)}>Cancel</button>
      </header>
      <table>
        <thead><tr><th>Device</th><th>Node</th><th>Attempt</th><th>Status</th>
          <th>Duration</th><th>Evidence</th><th>Artifact</th><th>Error</th><th /></tr></thead>
        <tbody>
          {detail.deviceRuns.flatMap((device) =>
            detail.attempts.filter((attempt) => attempt.deviceRunId === device.id)
              .map((attempt) => {
                const artifact = artifacts.get(attempt.id);
                return (
                  <tr key={attempt.id}>
                    <td>{device.udid}</td>
                    <td>{displayFlowState(attempt.actionKind)}</td>
                    <td>{attempt.attemptNo}</td>
                    <td>{displayFlowState(attempt.state)}</td>
                    <td>{attemptDurationMs(attempt)} ms</td>
                    <td>{formatEvidence(attempt.evidenceResult)}</td>
                    <td>{artifact ? <button type="button"
                      onClick={() => onOpenArtifact(artifact.id)}>{artifact.label}</button> : ""}</td>
                    <td>{attempt.error?.code ?? ""}</td>
                    <td>{attempt.retryAllowed && <button type="button"
                      onClick={() => onRetry(attempt.id)}>Retry {displayFlowState(attempt.actionKind)}</button>}</td>
                  </tr>
                );
              }),
          )}
        </tbody>
      </table>
    </section>
  );
}
```

The backend computes retryAllowed in its authoritative projection. It is true only
for failedBeforeDispatch, or for an idempotent-set failedVerified attempt whose
reconciler persisted proof that another dispatch is safe. The UI never derives
retry permission from action names or enables Uncertain attempts.

Render device rows and node attempts as a dense table, not nested cards. Show status, attempt number, duration, evidence result, artifact link, and typed error. On `flowRunUpdated`, refetch only the matching visible run when the event revision is newer.

- [x] **Step 6: Run tests and commit**

```powershell
npm --prefix apps/desktop test -- FlowRunMonitor.test.tsx
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
git add apps/desktop/src/components/flow
git commit -m "feat(flow): add revision and run workflow"
```

### Task 7: Integrate The Automation Page And Styling

**Files:**
- Modify: `apps/desktop/src/main.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/components/Sidebar.tsx`
- Modify: `apps/desktop/src/types.ts`
- Modify: `apps/desktop/src/index.css`
- Modify: `apps/desktop/src/App.css`
- Create: `apps/desktop/src/App.test.tsx`

- [x] **Step 1: Write page integration tests**

Test sidebar navigation, Flow title, preservation of legacy Jobs page, local draft
restore, unsaved sidebar/close prompt, desktop 1440x900 containment, and narrow
900x700 layout without toolbar/node/inspector overlap.

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";

vi.mock("./api", () => ({
  agentBulkRepair: vi.fn(),
  agentListStatuses: vi.fn(async () => []),
  authSession: vi.fn(async () => ({ showAuthUi: false, bypassed: true, user: null })),
  getStreamSettings: vi.fn(async () => ({
    fps: 24, tileSize: "medium", gridQuality: "medium", focusQuality: "high",
  })),
  listenRiviuEvents: vi.fn(async () => () => undefined),
  listDevices: vi.fn(async () => []),
  listJobs: vi.fn(async () => []),
  prepareDevice: vi.fn(),
  refreshDevices: vi.fn(async () => undefined),
  setStreamSettings: vi.fn(),
}));

vi.mock("./components/flow/FlowWorkspace", () => ({
  FlowWorkspace: ({ onDirtyChange }: { onDirtyChange: (dirty: boolean) => void }) => (
    <button type="button" onClick={() => onDirtyChange(true)}>Mark fixture dirty</button>
  ),
}));

beforeEach(() => {
  vi.clearAllMocks();
});

it("prompts once before leaving a dirty Flow draft", async () => {
  const user = userEvent.setup();
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
  render(<App />);
  await user.click(screen.getByRole("button", { name: "Flow" }));
  await user.click(screen.getByRole("button", { name: "Mark fixture dirty" }));
  await user.click(screen.getByRole("button", { name: "Jobs" }));
  expect(confirm).toHaveBeenCalledTimes(1);
  expect(screen.getByText("Flow", { selector: ".topbar-title" })).toBeVisible();
});
```

Render the real default-exported App; do not introduce production-only fixture props.

- [x] **Step 2: Import styles in the approved order**

In `main.tsx`:

```ts
import "@xyflow/react/dist/style.css";
import "./index.css";
```

No React Flow `@import` belongs after CSS rules in `index.css`.

- [x] **Step 3: Replace the Scripts page surface**

```tsx
const [flowDirty, setFlowDirty] = useState(false);
const [automationView, setAutomationView] = useState<"flow" | "legacy">("flow");

const requestPage = useCallback((next: PageId) => {
  if (next === page) return;
  if (flowDirty && !window.confirm("Discard unsaved Flow changes?")) return;
  setPage(next);
}, [flowDirty, page]);

const requestAutomationView = useCallback((next: "flow" | "legacy") => {
  if (next === automationView) return;
  if (flowDirty && !window.confirm("Discard unsaved Flow changes?")) return;
  setAutomationView(next);
}, [automationView, flowDirty]);

useEffect(() => {
  if (!flowDirty) return;
  const preventUnload = (event: BeforeUnloadEvent) => {
    event.preventDefault();
  };
  window.addEventListener("beforeunload", preventUnload);
  return () => window.removeEventListener("beforeunload", preventUnload);
}, [flowDirty]);

<Sidebar
  page={page}
  collapsed={asideCollapsed}
  selectedCount={selected.length}
  total={devices.length}
  readyCount={readyCount}
  groupMode={groupMode}
  onPage={requestPage}
  onToggleCollapse={() => setAsideCollapsed((value) => !value)}
/>

{page === "scripts" && (
  <section className="automation-surface">
    <div role="tablist" aria-label="Automation view" className="automation-tabs">
      <button role="tab" aria-selected={automationView === "flow"}
        onClick={() => requestAutomationView("flow")}>Flow</button>
      <button role="tab" aria-selected={automationView === "legacy"}
        onClick={() => requestAutomationView("legacy")}>Legacy</button>
    </div>
    {automationView === "flow" ? (
      <FlowWorkspace devices={devices} selectedUdids={selected}
        onDirtyChange={setFlowDirty} />
    ) : (
      <>
        <ScriptsPanel onUseInJobs={(json) => {
          setJobsScriptSeed(json);
          requestPage("jobs");
        }} />
        <ScheduleBlock devices={devices} selected={selected}
          onSelectUdids={setSelected} />
      </>
    )}
  </section>
)}
```

Change the Sidebar menu item to { id: "scripts", label: "Flow" }. When a
successful save resets the draft, FlowWorkspace calls onDirtyChange(false).
Authentication callbacks may continue to set their blocking Login/Register pages
directly. Keep the local Flow/Legacy tabs for one release. Flow is the default;
Legacy mounts the existing ScriptsPanel and ScheduleBlock unchanged, so unsupported
imports remain runnable and jobsScriptSeed still opens the existing Jobs page.

Mount `FlowWorkspace` for `page === "scripts"`, change its menu label/title to
`Flow`, and keep legacy `JobsPanel` and legacy commands reachable for the
one-release compatibility period. Remove `jobsScriptSeed` wiring only after the
existing legacy path remains directly usable. Lift dirty state to `App`; every
sidebar transition calls `requestPage(next)` and prompts once when dirty. Register
and remove `beforeunload` while dirty.

- [x] **Step 4: Add stable responsive layout CSS**

```css
.flow-workspace {
  display: grid;
  grid-template-rows: auto minmax(0, 1fr) minmax(160px, auto);
  min-width: 0;
  min-height: 620px;
  height: calc(100vh - 72px);
  overflow: hidden;
  background: var(--bg-panel);
  border: 1px solid var(--line);
  border-radius: 6px;
}

.flow-toolbar {
  display: flex;
  align-items: center;
  gap: 4px;
  min-height: 44px;
  overflow-x: auto;
  padding: 6px 8px;
  border-bottom: 1px solid var(--line);
  background: #fff;
}

.flow-layout {
  position: relative;
  display: grid;
  grid-template-columns: minmax(180px, 240px) minmax(420px, 1fr) minmax(260px, 320px);
  gap: 8px;
  min-height: 0;
  overflow: auto;
}

.flow-palette,
.flow-inspector {
  min-width: 0;
  overflow: auto;
  background: #fff;
}

.flow-palette {
  border-right: 1px solid var(--line);
}

.flow-inspector {
  border-left: 1px solid var(--line);
}

.flow-canvas-region {
  position: relative;
  min-width: 420px;
  min-height: 420px;
  overflow: hidden;
  background: var(--bg-muted);
}

.flow-monitor {
  min-width: 0;
  overflow: auto;
  border-top: 1px solid var(--line);
  background: #fff;
}

.flow-monitor table {
  width: 100%;
  min-width: 920px;
  border-collapse: collapse;
}

.flow-monitor th,
.flow-monitor td {
  padding: 6px 8px;
  border-bottom: 1px solid var(--line);
  text-align: left;
  white-space: nowrap;
}

@media (max-width: 1100px) {
  .flow-layout {
    grid-template-columns: minmax(420px, 1fr) minmax(260px, 300px);
  }

  .flow-palette[data-open="false"] {
    display: none;
  }

  .flow-palette[data-open="true"] {
    position: absolute;
    z-index: 20;
    width: 240px;
    inset: 44px auto 160px 0;
    box-shadow: var(--shadow);
  }
}

@media (max-width: 760px) {
  .flow-workspace {
    overflow-x: auto;
  }

  .flow-layout {
    grid-template-columns: minmax(420px, 1fr);
  }

  .flow-inspector[data-open="false"] {
    display: none;
  }

  .flow-inspector[data-open="true"] {
    position: fixed;
    z-index: 40;
    inset: 48px 0 0 auto;
    width: min(320px, 90vw);
    border-left: 1px solid var(--line);
    box-shadow: var(--shadow);
  }
}
```

Add stable data-testid values flow-toolbar, flow-palette, flow-canvas,
flow-inspector, and flow-monitor only on their top-level regions. Palette and
inspector toggles set exact data-open strings so responsive tests and assistive
state agree.

Use a full-width work surface with `grid-template-columns: minmax(180px, 240px) minmax(420px, 1fr) minmax(260px, 320px)`, a fixed `min-height: 620px`, and a monitor band below. At <=1100 px, collapse the palette behind its toolbar button and use `minmax(420px, 1fr) minmax(260px, 300px)` for canvas plus inspector, with no overlap. At <=760 px, put the inspector in a modal side sheet that is closed by default; the canvas remains at least 420 px and the document may scroll inside the work surface rather than overflow the page. Use 6 px or smaller radii, neutral surfaces, existing orange/green status accents, no gradients/orbs, and no cards nested inside panels.

- [x] **Step 5: Run frontend gate and commit**

```powershell
npm --prefix apps/desktop test
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
git add apps/desktop/src
git commit -m "feat(flow): integrate the visual automation workspace"
```

### Task 8: Close Desktop Gate F2

**Files:**
- Modify: `AGENTS.md`
- Modify: `docs/archive/plans/2026-07-30-riviu-flow-v2-desktop.md`

- [x] **Step 1: Run full Rust and frontend gates**

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
npm --prefix apps/desktop test
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
git diff --check
```

Expected: every command exits 0.

- [x] **Step 2: Record and commit F2**

Mark all F2 checkboxes complete. Record commit range, Rust/frontend counts, disabled nodes, next plan, and rollback commit in `AGENTS.md`.

```powershell
git add AGENTS.md docs/archive/plans/2026-07-30-riviu-flow-v2-desktop.md
git commit -m "docs(flow): record desktop gate F2"
```
