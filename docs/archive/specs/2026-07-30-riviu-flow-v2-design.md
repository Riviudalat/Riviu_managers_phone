# Riviu Flow V2 Visual Automation Design

**Status:** Approved by user on 30/07/2026

## 1. Context

Riviu currently exposes automation as an `AutomationScript` version 1 JSON array.
The executor can launch or terminate an app, wait, tap, swipe, type text, capture a
screenshot, go Home, and assert an accessibility identifier. The desktop editor is
a raw JSON textarea and a job has one shared step-status array even when it targets
multiple devices.

That model is useful as a compatibility format, but it is not a workflow engine. It
cannot represent stable node identity, typed inputs and outputs, per-device evidence,
branches, bounded repetition, cross-device dependencies, crash reconciliation, or
the future sequence where device A comments and device B replies.

The project does not contain RouterMMO desktop source. This work recreates the useful
visual-authoring behavior against Riviu's typed device contracts; it does not copy or
execute vendor binaries, private endpoints, or an unverified workflow runtime.

## 2. Decisions

1. The authoring format is a graph from the first release, while release 1 permits
   only one deterministic linear path. This avoids another migration when branches
   and cross-device dependencies are added.
2. React owns canvas layout and editable draft state. Rust owns schemas, validation,
   compilation, persistence, capability checks, scheduling, and execution.
3. The runtime executes an immutable compiled plan, never the mutable canvas graph.
4. Existing JSON v1 scripts remain supported. The semantics-preserving subset can be
   imported as straight-line Flow v2 graphs; unsupported shapes receive diagnostics
   and remain untouched in the legacy store.
5. TikTok campaigns will remain a dedicated domain engine after the pending
   Interaction roadmap implements and qualifies that engine. A future visual node
   invokes its typed public contract; it never expands Like, Comment, or identity
   proof into unverified raw coordinate taps.
6. Every device run and node attempt has independent state. A shared job-level step
   array is not used by Flow v2.
7. Device lifecycle remains owned by `DeviceControlPlane`; the flow runtime cannot
   call a driver, relay, WDA endpoint, or stream helper directly.
8. Production RT-MMO and Riviu Agent artifacts remain unchanged by this project.
9. A transport or gesture acknowledgement records only dispatch. A side-effecting
   node reaches `Succeeded` only after its action-specific postcondition is observed
   through an approved evidence channel.

## 3. Release Scope

### 3.1 Release 1

Release 1 delivers a usable visual replacement for the current JSON editor:

- drag nodes from an action palette and reorder/connect one linear path;
- typed property inspector and inline validation;
- Start and End nodes;
- Launch App, Wait, Tap, Swipe, Type Text, Screenshot, Home, and Assert Visible;
- Terminate App only after the sidecar performs a real terminate and verifies the
  result instead of returning a best-effort success;
- select one, selected, or all currently eligible devices;
- save immutable revisions, duplicate flows, and import/export Flow v2 JSON;
- import the supported legacy `AutomationScript` v1 subset with typed diagnostics
  for every step that cannot preserve semantics;
- compile preview showing required capabilities and device resources;
- independent per-device and per-node run progress, artifacts, and errors;
- cancellation and restart reconciliation;
- advanced JSON view for diagnostics without making JSON the primary editor.

Release 1 rejects arbitrary cycles, conditions, repetition, subflows, cross-device
edges, TikTok domain nodes, and side-effecting nodes without a qualified
postcondition with a typed compile error.

### 3.2 Later Releases

- Release 2 adds `If`, `Repeat(maxIterations)`, and `ForEach` through explicit,
  bounded compiler constructs rather than arbitrary graph cycles.
- Release 3 adds qualified TikTok nodes after Interaction G0-G3 pass on the exact
  live tuple.
- Release 4 adds cross-device barriers and typed output bindings. A future
  `Comment` output may feed a `ReplyToComment` input only when the persisted artifact
  contains a separately qualified comment identity.
- Save, Repost, Direct Message, and Reply remain disabled until their own fixture and
  live gates pass.

## 4. Non-Goals

- No general-purpose code, shell, HTTP-request, or raw WDA endpoint node.
- No direct interpretation of graph edges in React.
- No automatic retry of an ambiguous tap, swipe, text input, comment, repost, or
  message side effect.
- No account switching, MDM, supervision, proxy application, backup, or restore as
  part of Flow v2 release 1.
- No claim of RouterMMO feature parity from an Agent feature string or route path.

## 5. Authoring Model

```rust
struct FlowDocumentV2 {
    schema_version: u32,       // exact value 2
    id: FlowId,
    name: String,
    revision: u64,
    entry_node_id: NodeId,
    nodes: Vec<FlowNode>,
    edges: Vec<FlowEdge>,
    viewport: FlowViewport,
}

struct FlowNode {
    id: NodeId,
    kind: ActionKind,
    position: CanvasPoint,
    config: serde_json::Value,
    postcondition: Option<EvidenceSpec>,
}

struct FlowEdge {
    id: EdgeId,
    source_node_id: NodeId,
    source_port: String,
    target_node_id: NodeId,
    target_port: String,
}
```

Node and edge IDs are stable UUIDs. Position and viewport are authoring metadata and
are excluded from the execution hash. Node configuration is validated against the
server-provided action definition for the exact schema version.

`postcondition` is forbidden for control/read-only nodes and required according to
the side-effecting action definition. The UI offers only evidence variants backed by
an implemented verifier; it never accepts an arbitrary expression and labels that
as proof.

The frontend may perform the same pure checks for immediate feedback, but backend
validation is authoritative on save, preview, and run.

## 6. Action Registry

The backend exposes an `ActionDefinition` catalog:

```rust
struct ActionDefinition {
    kind: ActionKind,
    schema_version: u32,
    label: String,
    disabled_reason: Option<String>,
    category: ActionCategory,
    config_schema: serde_json::Value,
    input_ports: Vec<PortDefinition>,
    output_ports: Vec<PortDefinition>,
    required_capabilities: Vec<CapabilityId>,
    resource_class: ResourceClass,
    side_effect_class: SideEffectClass,
    evidence_requirement: EvidenceRequirement,
    reconciliation_policy: ReconciliationPolicy,
    default_timeout_ms: u32,
    retry_policy: RetryPolicy,
}
```

`ResourceClass` distinguishes pure desktop work, bridge-only device work,
control-session work, and session-plus-stream work. The catalog publishes static
requirements. The UI may show a provisional disabled reason from the latest device
snapshot, but the fresh run-time preflight is authoritative.

The action registry never publishes secrets, route tokens, private WDA paths, or
proxy passwords. It publishes capability identifiers and form schemas only. A
backend-owned `disabled_reason` is the sole producer for provisional palette
disablement; the frontend never derives capability policy from an action name.

Approved evidence variants for release 1 are typed contracts such as
`ActiveAppEquals`, `ProcessAbsent`, `FrameDigestChanged`, `QualifiedFramePredicate`,
`AccessibilityVisible`, `TextReadBackEquals`, and `ArtifactDecodedAndHashed`.
Tap, Swipe, and Type Text require a pre-action frame plus an action-specific
postcondition from `FrameSource`; Type Text additionally requires exact read-back or
a qualified visual predicate supplied by the backend action catalog. A WDA HTTP 200,
gesture callback, or `/wda/keys` acknowledgement is never evidence by itself.
Evidence variants are action-scoped: generic Tap cannot use an unrelated whole-frame
digest change as semantic proof. Screenshot captures the latest frame from the
owned stream generation, decodes it, and publishes its hash through the artifact
protocol; it never calls WDA `GET /screenshot`.

## 7. Compiler And Immutable Plan

`riviu-core::flow::model` owns the versioned authoring and compiled-plan types so
the existing dependency direction remains `riviu-script-engine -> riviu-core`.
`riviu-script-engine` gains the Flow v2 parser, validator, legacy importer, and
pure compiler. `FlowRuntime` in `riviu-core` consumes only the compiled core type
and never depends back on `riviu-script-engine`. The compiler performs these checks
in order:

1. exact document and action schema versions;
2. unique canonical node and edge IDs;
3. exactly one Start and one reachable End;
4. valid ports and type-compatible connections;
5. release-1 linear topology with no cycles or disconnected executable nodes;
6. canonical config validation, including finite coordinates and bounded waits;
7. exactly one Tap targeting mode: point or supported selector;
8. static capability-ID and resource requirement aggregation;
9. required evidence, reconciliation, side-effect, and retry-policy compatibility;
10. deterministic canonical serialization and SHA-256 plan hash.

The compiler produces `CompiledFlowPlanV2`, which contains no canvas positions.
It contains canonical nodes, execution order, its monotonic `ContextPlan`, typed
`CompiledActionConfig` values (never raw authoring JSON), action-definition versions,
and required capability IDs.
Device selection is a typed `flow_run` input, not part of the reusable flow
revision. Its `One`, `Selected`, or `AllEligible` policy is resolved when a run
starts, then exact UDIDs are persisted as the immutable run selection snapshot.
Exact UDIDs are not embedded in the saved plan hash. Canonical stored compiled bytes
include the assigned revision. The execution hash uses the same canonical material
with only that top-level revision omitted, so layout-only saves and otherwise
identical execution at a later revision retain the same hash; flow ID, typed config,
action-definition versions, context, and capability IDs remain covered. The saved
revision, compiled bytes, and hash are immutable for a run.

Every plan requiring a UI session must have Launch App as its first executable node
so target-qualified preflight and session creation are defined. Wait and the
bridge-only Terminate App may run without Launch because Terminate carries its own
bundle ID. The first Launch is executed exactly once through the ordinary durable
attempt sequence; session and optional stream startup occur after its effect dispatch
and before its active-app verification.

Release 1 rejects XPath, predicate, and class-chain selectors. Generic Tap and
Assert Visible retain accessibility ID; qualified text read-back additionally
permits the exact WDA `class name` strategy already exercised by the Settings gate.
It does not silently ignore unsupported selector fields or choose one of conflicting
point/selector inputs.

## 8. Persistence

The existing database has one unversioned `CREATE TABLE IF NOT EXISTS` batch. Flow
V2 first adds a transaction-based migration runner and `schema_migrations` ledger.
It recognizes the exact populated legacy schema as baseline version 1, records that
baseline without rewriting legacy rows, and then applies each numbered migration
once. An unknown or partially matching legacy schema fails closed with a diagnostic
and no mutation.

Numbered additive migrations create:

- `flow_documents`: mutable name, latest revision, archived flag;
- `flow_revisions`: immutable authoring JSON, compiled plan, plan hash, created time;
- `flow_runs`: revision/hash snapshot, selection snapshot, aggregate state;
- `flow_device_runs`: one row per target device with independent state;
- `flow_node_attempts`: device run, node ID, attempt, intent, status, timings, error;
- `flow_artifacts`: managed path, kind, size, SHA-256, producing attempt;
- `flow_events`: monotonic revisioned state transitions for monitor invalidation.

SQLite is the runnable-work source. Tauri events only invalidate UI caches. No
SQLite transaction may cross `.await`, device acquisition, USB I/O, stream transfer,
or network work.

Saving a new flow revision, its compiled plan, and hash is one transaction. Starting
a run persists the run and all initial device/node states before any device side
effect. A later edit creates a new revision and cannot alter an active or historical
run.

### 8.1 Artifact Publication

Artifact paths are server-generated UUID paths under the managed Flow artifact root;
user-provided screenshot names are labels only. Labels reject traversal, separators,
reserved device names, control bytes, and unsupported extensions.

Publication writes to a same-volume temporary file, validates and decodes the
content, computes size and SHA-256, flushes it, and atomically renames it to its final
contained path. A following SQLite transaction inserts the artifact row and marks
the producing attempt successful. If the transaction fails, the final file is
removed. Startup reconciliation removes stale temporary files, quarantines final
files with no row, and marks a row whose file is absent or hash-mismatched as a typed
artifact failure. No success event is emitted before the file and database record
agree.

## 9. Execution Model

`FlowRuntime` lives in `riviu-core` and consumes only compiled plans. It requests
typed ownership from `DeviceControlPlane` with `DeviceWorkOwner::Script`.

Release 1 executes devices concurrently only as their independent device leases and
resource limits permit. Each device has its own node-attempt state and artifact
namespace. A worker acquires one device only; it never holds device A while waiting
to acquire device B.

The compiler produces one monotonic `ContextPlan` per release-1 device chain. The
worker acquires `DeviceExclusiveContext` once, performs exact-tuple preflight and any
bridge-only preparation, then upgrades at most once to `UiSessionContext` and at
most once again to `UiWithStreamContext`. It never closes and reacquires ownership
between dependent Launch, Wait, Tap, Swipe, Type Text, Screenshot, Home, or Assert
Visible nodes. Since the current control plane has no downgrade operation, a bridge
action that cannot run through the upgraded context must appear before the upgrade
or compilation fails.

When any node requires frame evidence, stream capacity is reserved while the
exclusive context is held. The first Launch attempt then commits intent/effect,
foregrounds exactly once, starts the approved UI session, and only afterward starts
MJPEG. `FlowRuntime` receives the core `FrameSource` contract and binds it to that
exact `UiWithStreamContext`; it does not poll WDA screenshots. Generation advance is
an explicit stream event that immediately invalidates an old verifier. Every frame
wait has cancellation plus an absolute deadline. Every exit path closes the highest
context exactly once and persists release proof.

Release-1 `Wait` is an in-chain UI delay capped at 60 seconds, so another workflow
cannot change the screen between dependent gestures. Longer delays belong to the
scheduler and are rejected by the Wait node. Future cross-device barriers are safe
release boundaries: all device contexts close before the barrier persists and
waits, preventing multi-device lease deadlocks.

### 9.1 Runtime Capability Preflight

Compilation aggregates static capability IDs only. At run start, `One` and
`Selected` snapshot their requested devices; `AllEligible` snapshots all connected
candidates. Each device worker acquires its exclusive context and records a fresh
exact-tuple capability snapshot before that device performs application effects.
An ineligible `One` device fails the run. In `Selected`, an ineligible device fails
its device run while independently eligible devices may proceed, yielding an
explicit partial aggregate; there is no cross-device atomicity. `AllEligible` marks
ineligible candidates `Skipped` with a typed reason and fails if none remain.

Text input also requires a fresh approved session and a successful
`supports_text_input()` probe before the first text effect. Assert Visible is
published only for a profile whose current selector contract is qualified. Flow
must never raise stock WDA `snapshotMaxDepth` above 1 to make a selector work.

### 9.2 Desktop Shutdown Order

Tauri exit first rejects new work and calls `nurture.begin_shutdown()`,
`flows.stop_all()`, and `jobs.stop_all()`. It then stops the background sampler,
awaits `flows.shutdown()` to join every Flow worker, awaits `jobs.shutdown()`, and
only then calls `DeviceControlPlane::shutdown_cleanup()`. No Flow task may outlive
control-plane cleanup. All operation requests have local deadlines; Flow shutdown
also has a 30-second join deadline and aborts then joins any task still violating
those contracts before control cleanup proceeds.

## 10. Side Effects, Retry, And Cancellation

The exhaustive durable attempt states are `Queued`, `IntentCommitted`,
`EffectDispatched`, `Verifying`, `Succeeded`, `FailedBeforeDispatch`,
`FailedVerified`, `Uncertain`, `Cancelled`, and `Interrupted`. Before any device
effect, the runtime commits `IntentCommitted` with canonical input and the evidence
baseline. It then commits `EffectDispatched` immediately before invoking the device.
After the call returns it commits `Verifying`; only a satisfied postcondition can
commit `Succeeded`.

Recovery may convert `IntentCommitted` to `FailedBeforeDispatch`, because dispatch
was never recorded. `EffectDispatched` or `Verifying` must run the action's typed
reconciler: conclusive desired-state proof becomes `Succeeded`, conclusive transport
non-delivery becomes `FailedBeforeDispatch`, and every ambiguous result becomes
`Uncertain`. A read-only or timer attempt may become `Interrupted` and restart under
its bounded policy.

`flow_retry_attempt` is enabled only for `FailedBeforeDispatch`, or when an
action-specific idempotent reconciler re-reads state and proves retry cannot duplicate
the effect. It rejects `Uncertain` Tap, Swipe, and Type Text attempts. A fresh
operator action does not silently reuse the old attempt or overwrite its evidence.

Cancellation is cooperative while waiting for a lease, between nodes, and inside
the Wait node. A WDA request already in flight completes under its own request
deadline; the runtime then closes the exact session context and persists the final
attempt state before releasing ownership.

On desktop startup, persisted `Queued` work can be reclaimed. Every other nonterminal
state follows the transition rules above; release 1 never silently resumes an
ambiguous side effect. An eligible retry creates a new attempt and reuses the
immutable plan snapshot.

## 11. TikTok And Cross-Device Boundary

`InteractionCampaignEngine` is a pending Interaction-roadmap component, not current
code. Flow V2 release 1 defines no concrete port against it. After that engine,
its persistence, and G0-G3 are implemented and qualified, future TikTok nodes become
typed adapters over its public action facade. The generic flow runtime must not
duplicate its identity proof, frame evidence, action intent, or retry logic.

An A-comment/B-reply flow is modeled as separate device segments connected through
a durable artifact:

```text
Device A: VerifiedTarget -> PreparedComment -> Comment -> CommentArtifact
                           persist + release A
Barrier:                  validate qualified comment identity
Device B:                 OpenVerifiedTarget -> ReplyToComment(CommentArtifact)
```

Text, author label, or configured account handle alone is not reply identity. The
Reply node stays disabled until a platform comment ID or a separately gated
author/text locator provides unique live proof.

## 12. Desktop UX

Use the maintained `@xyflow/react` package as a controlled canvas. Riviu owns all
custom node rendering and styling; no generated UI kit is required. The required
React Flow stylesheet is imported once from `main.tsx` before Riviu's `index.css`,
so the project styles remain the final override. The canvas parent has a stable
constrained height so loading, selection, and node labels cannot resize the tool
surface.

The Automation page becomes one work-focused surface:

- left: searchable action palette grouped by App, Input, Timing, Evidence, and later
  TikTok;
- center: canvas with minimap, zoom controls, grid snap, keyboard selection, and
  edge creation;
- right: typed node inspector with validation at the field and node level;
- bottom: compile errors and run monitor, selectable per device and node attempt;
- top toolbar: Save revision, Validate, Run, Import, Export, Undo, and Redo.

The UI keeps a bounded undo/redo history for the draft. It autosaves only a local
draft; a server revision is created by an explicit Save command. Closing with an
unsaved draft prompts once. Buttons use existing icon conventions and disabled
nodes show a tooltip with the backend capability reason.

The Tap and Swipe inspectors can enter numeric coordinates or pick them from the
latest device frame. A picked point stores image dimensions and the qualified
orientation/profile; compile rejects a runtime tuple that cannot safely map it.

## 13. Tauri API And Events

Add typed commands:

- `flow_action_catalog`
- `flow_list`, `flow_get`, `flow_save_revision`, `flow_archive`
- `flow_validate`, `flow_import_legacy`, `flow_export`
- `flow_run`, `flow_cancel_run`, `flow_retry_attempt`
- `flow_list_runs`, `flow_get_run`

All mutating commands return typed serializable errors with `code`, `message`, and
optional `nodeId`, `field`, existing `udid`, or `attemptId`. They do not return formatted
Rust error chains to the UI.

`FlowUpdated` and `FlowRunUpdated` events carry identifiers and monotonically
increasing revisions. The frontend refetches authoritative SQLite projections.

## 14. Compatibility And Rollback

- Existing `scripts` and `jobs` tables remain readable.
- Import maps the semantics-preserving v1 subset to Flow v2 nodes in array order.
  Waits above 60 seconds, XPath/predicate selectors, point/selector conflicts,
  non-finite coordinates, unqualified Terminate App, and actions lacking a required
  evidence contract produce node-scoped diagnostics and are not silently changed.
- The original v1 JSON remains untouched and runnable through the legacy path while
  the operator resolves import diagnostics.
- Legacy execution commands remain available during one release cycle.
- Flow v2 uses additive tables and a separate runtime, so rollback disables the new
  page/commands without rewriting legacy rows.
- No migration deletes or rewrites existing scripts, jobs, artifacts, Agent files,
  or capability registries.

## 15. Corrections Required Before Enablement

The current sidecar `terminate` command returns success without terminating the app,
and `syslog` returns sample text. The terminate correction applies to both legacy and
Flow execution so the compatibility path cannot retain a false success. Before
Terminate App appears in the action catalog:

1. implement bounded DVT termination for the exact bundle/process;
2. verify the process is absent or the app state changed as specified;
3. return a typed error on unsupported or ambiguous outcomes;
4. add Python contract tests and Rust integration tests.

After these gates pass, Terminate is a typed release-1 action with
`ProcessAbsent { bundle_id }` evidence and read-only process reconciliation. It uses
the same per-UDID `DeviceControlPlane` ownership as legacy jobs; recovery may query
process state but never issue another kill merely to decide whether retry is safe.
The attempt persists the pre-effect PID: absence after dispatch proves success, the
same positive PID proves non-delivery, and a different positive PID is uncertain.

Syslog is not a release-1 flow node. The Diagnostics product path must implement a
real bounded os_trace relay before a future syslog node can be advertised.

## 16. Testing

### Rust

- parser/schema and canonical hash golden tests;
- legacy import success for the supported subset and exact diagnostics for every
  unsupported but currently valid v1 shape;
- cycle, disconnected node, invalid port, selector conflict, coordinate, and wait
  boundary tests;
- migration from a populated current database, per-migration rollback failpoints,
  reopen/idempotence, constraints/indexes, and byte-preservation of legacy rows;
- immutable revision, per-device state, pagination, and recovery;
- compiler monotonic-context planning and static capability aggregation;
- cancellation at acquire, between nodes, during Wait, and after a session exists;
- ambiguous side-effect classification and retry eligibility;
- evidence-baseline ordering and proof that gesture acknowledgement alone never
  reaches `Succeeded`;
- artifact traversal/reserved-name rejection and crash failpoints before rename,
  after rename, before DB commit, and after DB commit;
- two-device execution proving independent progress and no overlapping ownership;
- Tauri exit ordering with cancellation during lease acquisition, Wait, and an
  in-flight WDA request; Flow workers join before control cleanup starts.

### Frontend

- palette drag/drop, connect, reorder, delete, inspector forms, disabled reasons;
- undo/redo, unsaved draft, import/export, validation mapping, and per-device monitor;
- coordinate picker scaling and orientation display;
- Vitest component tests plus Playwright desktop and narrow-window screenshots.

### Live Gate

Run one linear flow on the designated iPhone with Launch App, Tap, Swipe, Type Text,
Screenshot, and Home. Capture the configured pre/post frame or read-back evidence
for every side effect, prove an injected gesture ACK without matching evidence does
not succeed, verify artifact hashes, cancel one run, restart the desktop during one
controlled run, and prove process/session/stream cleanup. The live report may qualify
only the exact device/app/Agent/adapter tuple exercised.

## 17. Release-1 Acceptance

- An operator builds and runs a flow without editing JSON.
- A supported legacy example imports to an equivalent straight-line graph, while
  every unsupported v1 shape receives a node-scoped diagnostic without mutation.
- Backend validation rejects malformed or unsupported nodes before device work.
- Two mock devices maintain independent node histories and artifacts.
- The runtime never exceeds the shared device/stream budgets.
- No Tap, Swipe, Type Text, Launch, Home, or Terminate attempt succeeds from transport
  acknowledgement alone.
- Closing, cancellation, and startup reconciliation leave no owned session or stream.
- Terminate App is either genuinely implemented and verified or absent from the
  catalog; no false success remains.
- Existing Nurture, Interaction gates, manual control, legacy rows, production IPA,
  and capability registry behavior remain unchanged; the terminate false-success
  correction is the only intentional legacy execution change.
