# Interaction Campaign And Flow V2 Delivery Plan

**Status: SHIPPED. Historical record, not an active plan** (label corrected 27/08/2026).

Both halves of this plan landed and run live: the Interaction workflow drives a 20-phone
fleet, and the Flow V2 editor ships as `apps/desktop/src/components/flow/FlowWorkspace.tsx`
against `crates/core/src/flow/`. It read `**Status:** Active delivery plan, approved scope
31/07/2026.` for the four weeks after it was delivered — the one file in this directory
that carried a status line was the one whose status line was wrong.

Current state of anything named here: `AGENTS.md` section 9, newest entry first.

**Goal:** Ship the operator-facing TikTok Interaction workflow and Flow V2 visual
editor without weakening the existing per-UDID owner, stream budget, durable
evidence, startup recovery, or production Agent rollback guarantees.

**Baseline:** `main` commit
`d4523a13146aef26730915fc6a495d74c1377e7f`. Flow F0/F1 and Interaction
G0.1-G0.11 source/fixture work are already complete. G0.12 remains a Mac/device
gate.

This plan coordinates the existing detailed plans. Their task-level tests and file
lists remain authoritative unless this document explicitly resolves a conflict:

- `2026-07-30-riviu-flow-v2-desktop.md`;
- `2026-07-30-riviu-flow-v2-acceptance.md`;
- `2026-07-29-tiktok-interaction-campaign-core.md`;
- `2026-07-29-tiktok-interaction-verified-actions.md`;
- `2026-07-29-tiktok-interaction-desktop-workflow.md`;
- `2026-07-29-tiktok-interaction-fleet-acceptance.md`.

---

## 1. User-Visible Scope

### 1.1 TikTok Interaction

- Add `Tương tác` immediately after `Nuôi TT` on the device-control toolbar.
- Keep Nurture and Interaction mutually exclusive while preserving the device grid
  and its parked last frames behind the active panel.
- Accept multiline TikTok direct video/photo URLs, `vt.tiktok.com` short URLs, and
  other supported TikTok post URLs. Rust parses/resolves every line, validates every
  redirect hop, deduplicates by content identity, and returns typed line errors.
- Select `Tất cả máy online` or explicit devices. Production exposes only
  `device:<udid>:default`; no account-switch UI is added.
- Select `Tất cả` (every actor processes every target) or `Phân bổ`
  (deterministic round-robin).
- Configure campaign defaults plus per-target overrides for Watch, Like, Follow,
  and Comment. Probability is sampled once into the immutable plan.
- Support Run Now and one-time scheduling.
- Persist campaigns, targets, actor snapshots, assignments, action attempts,
  evidence, cancellation, retry eligibility, and restart recovery in SQLite.
- Provide Setup and Monitor views, paged restore after restart, cancel, eligible
  retry, and coordinated Open on Device.
- Continue eligible assignments when one device/target fails; never replay an
  ambiguous issued side effect.

Save, Repost, Direct Message, account switching, and A-comment/B-reply remain
disabled until their later independent gates.

### 1.2 Flow V2 Release 1

- Replace the JSON-first Automation surface with a controlled React Flow editor.
- Provide a palette, drag/drop, one release-1 linear path, typed ports, node
  inspector, inline diagnostics, coordinate picker, undo/redo, and local draft
  recovery.
- Save immutable revisions; support duplicate, import/export Flow V2 JSON, and
  semantics-preserving legacy-script import with typed diagnostics.
- Run on one, selected, or all eligible devices; monitor per-device/per-node state,
  artifacts, errors, cancel, and only backend-approved retry.
- Keep legacy Jobs available as a separate page.
- Release 1 exposes only the existing typed generic nodes. TikTok domain nodes are
  not added until Interaction G0-G3 are complete and separately integrated.

---

## 2. Deferred Scope Stays Recorded

The following remain in `AGENTS.md` and are not pulled into this delivery:

- replacing production RT-MMO with the reconstructed Riviu Agent candidate;
- multiple TikTok accounts per iPhone and automatic switching;
- automatic system-wide proxy apply on the current unsupervised fleet;
- MDM/supervision/AdminControl and remote fleet policy;
- Flow branches, loops, subflows, cross-device edges, Comment-to-Reply bindings;
- Interaction Save, Repost, Direct Message, and Reply gates.

G3 may keep the existing truthful proxy catalog/manual-state screen required by its
desktop plan, but it must not claim iOS system proxy application or expand the proxy
engine.

---

## 3. Conflict Resolutions Before Implementation

1. **G0.12 remains a hard boundary.** Flow F2 may proceed on Windows now. G1/G2/G3
   production execution starts only after the exact Mac/device G0.12 report passes
   and the reviewed qualification tuple is installed. Fixture output never enables
   production Interaction.
2. **Use one database migration ledger.** Flow F0 already owns
   `schema_migrations` versions 1 and 2. Interaction schema is additive migration
   version 3 in `crates/core/src/db/migrations.rs`; do not create another ledger or
   a second migration runner.
3. **Use one frontend test stack.** Flow F2 installs and pins
   `@xyflow/react`, `lucide-react`, Testing Library, jest-dom, jsdom, and Playwright.
   Keep one `vite.config.ts`, one `src/test/setup.ts`, and one
   `playwright.config.ts`. Interaction G3 reuses them; it does not add a competing
   happy-dom/Vitest configuration.
4. **Serialize integration-file ownership.** Only one slice at a time edits
   `state.rs`, `lib.rs`, `events.rs`, `App.tsx`, `Sidebar.tsx`, `types.ts`, `api.ts`,
   `package*.json`, test configs, or `AGENTS.md`. Domain-only work may run in
   parallel worktrees.
5. **Populate devices before recovery and keep artifact roots separate.** Bootstrap
   performs the initial metadata device scan and updates `DeviceRegistry` before
   calling either runtime recovery; an unperformed/failed scan must not look like an
   authoritative empty fleet. Flow then runs `recover_startup()`, queries every
   committed Flow artifact row through a new repository API, and only then calls
   Flow orphan reconciliation. Interaction follows the same order in its separate
   `artifacts/interaction` namespace. Never reconcile only nonterminal-run rows.
6. **Use one shutdown owner.** Atomically reject every mutating command, including
   device actions, save/archive, start/schedule/cancel/retry, settings, credentials,
   database writes, and queue insertion. Drain already admitted mutations, signal
   Nurture/Jobs/Flow/Interaction, stop and join the background sampler, join
   Interaction and Flow workers, join Jobs, then call
   `DeviceControlPlane::shutdown_cleanup()`. No worker may outlive control cleanup.
7. **Events are invalidation hints.** `FlowUpdated`, `FlowRunUpdated`, and
   `InteractionUpdated` are post-commit monotonic notifications. React refetches
   authoritative projections; it does not reconstruct durable state from events.
   Flow F2 must add a tested executor post-commit invalidation callback or bounded
   polling while a run is nonterminal; the current F1 completion-only events are not
   sufficient for live per-node status.
8. **Keep production artifacts byte-identical.** Do not modify the production IPA,
   manifest, or qualification registry except through their named reviewed live-gate
   transactions.
9. **Create every named G1 test target.** Add
   `crates/core/tests/interaction_planner.rs` and
   `crates/core/tests/interaction_aggregation.rs` before invoking the detailed G1
   `--test` commands; do not leave verification referring to nonexistent targets.
10. **Map typed command errors centrally.** Flow and Interaction commands downcast a
    public typed service error into stable command codes such as
    `RevisionConflict`, `RetryNotAllowed`, `NoEligibleDevice`, and
    `ApplicationShuttingDown`. Never infer a code by parsing arbitrary `anyhow`
    display text or expose database/filesystem internals.
11. **Artifacts need a real command boundary.** A monitor artifact control calls a
    backend command with an artifact ID. The backend reloads the row, validates the
    canonical path is contained in the namespace root, enforces kind/size/hash, and
    returns bounded image bytes or opens that exact file. UI links are never no-op
    and never accept arbitrary paths.
12. **Evidence remains action-specific.** Launch/Home use active-app proof,
    Terminate uses exact process absence, Type Text uses read-back or a qualified
    visual predicate, Screenshot uses decoded artifact proof, and Tap/Swipe use
    generation-qualified frame evidence. F3 must not demand frame evidence for every
    side effect.

---

## 4. Execution Order

### Phase 0: Commit The Plan, Freeze Baseline, And Re-Prove Shared Control

- Commit and push this coordination plan plus its `AGENTS.md` checkpoint first.
  Resolve the implementation base with
  `git log -1 --format=%H -- docs/archive/plans/2026-07-31-interaction-flow-delivery.md`
  and create the isolated implementation worktree from that commit. Keep `d4523a1`
  as the rollback baseline; a worktree created directly from it would omit this plan.
- Record production IPA and canonical-LF manifest hashes.
- Run full Rust, sidecar, and frontend baseline verification.
- Re-run focused owner, stream-budget, shutdown, and generation tests.

**Exit:** clean baseline; one owner per UDID and producer budget are proven before
new runtimes are composed.

### Phase 1: Flow F2 Backend Composition

- Implement `flow_commands.rs` and a centralized typed service-error mapping;
  reject string-derived command codes.
- Compose `FlowRuntime`, `FlowArtifactStore`, repositories, compiler, and event sink
  in `AppState`.
- Complete the initial metadata device scan before Flow recovery. Add a repository
  query for all committed Flow artifact rows; run `FlowRuntime::recover_startup()`
  before reconciling against that complete row set.
- Add command admission/drain, a mutating-command inventory, and combined exit-order
  tests. Every mutating Tauri handler retains its permit for the complete operation.
- Add post-commit node-attempt invalidation or bounded nonterminal-run polling so the
  run monitor updates before a whole device run finishes.
- Expose catalog/list/get/validate/save/archive/import/export/run/cancel/retry/run
  detail/coordinate-frame commands.
- Expose a bounded artifact-read/open command that validates the persisted row,
  containment, format, size, and hash before returning data.
- Define and test the target-bound protected-auth/geometry/readiness snapshot
  contract now, while leaving real Pmd enablement denied until Phase 6A live proof.
- Keep the Pmd UI path fail-closed until a runtime-qualified protected-auth and
  geometry snapshot exists.

**Exit:** all Flow Tauri helper tests pass; no React code is required for this gate.

### Phase 2: Flow F2 Visual Editor And Monitor

- Pin the shared frontend/test dependencies and configure jsdom once.
- Add Flow TypeScript DTOs and exact invoke-contract tests.
- Implement pure draft reducer, graph mapping, 50-entry undo/redo, dirty epochs,
  validation invalidation, and versioned local draft storage.
- Build palette, controlled canvas, custom nodes, inspector, diagnostics, import,
  JSON view, and coordinate picker.
- Reject blank/non-finite numeric input before it enters the draft; never persist
  `valueAsNumber` when it is `NaN`.
- Add revision toolbar, One/Selected/AllEligible selection, run dialog, and per-device
  run monitor.
- Integrate a dedicated Flow/Automation page without removing legacy Jobs.
- Verify 1440x900 and 900x700 containment with no toolbar/node/inspector overlap.

**Exit F2:** Rust workspace, clippy, Vitest, lint, frontend build, and `git diff
--check` pass; checkpoint and disabled real-device limitations are recorded.

### Phase 3: Flow F3 Fixture And Browser Acceptance

- Add end-to-end mock Flow coverage for save/run/cancel/retry/restart/artifacts.
- Use a dedicated integration-test `DeviceDriver` and frame source with two identical
  qualified geometry snapshots and explicit call counters. Do not depend from core
  tests on `riviu-ios-driver`, assume unsupported `MockIosDriver` injection APIs, or
  call the legacy device screenshot path.
- Assert each node's approved evidence kind rather than requiring frame evidence for
  every side effect.
- Add deterministic Playwright editor and monitor workflows plus screenshots.
- Run desktop pixel/containment checks. Label fixture output `FIXTURE_ONLY`.
- Leave the Mac/iPhone portion of F3 open until the exact protected-auth/geometry
  tuple is available.

**Exit:** F3 mock/browser evidence passes; F3 itself remains pending live device.

**Checkpoint 31/07/2026:** Phases 1 and 2 are PASS on Windows. Phase 3
mock/browser evidence is PASS and labeled `FIXTURE_ONLY`; the isolated pre-F0
rollback proof is also PASS. The Mac/iPhone portion of F3 remains open and still
depends on Phase 4A. Interaction G0.12 and Phases 5-8 are unchanged and remain
blocked by their documented live gates.

### Phase 4: Interaction G0.12 On Mac

- Close desktop, harnesses, 3uTools, and other XCTest runners.
- Run the exact Gate 0 probe from `docs/re/interaction-gate0/README.md` on the
  pinned iPhone/TikTok/RT-MMO tuple.
- Require protected URL/clipboard/geometry/session-before-MJPEG/Copy-Link-reference
  evidence, cleanup, redaction, and unchanged production hashes.
- Review and transactionally add exactly the qualified registry entry.

**Exit G0:** live report PASS and production registry contains only the reviewed
exact tuple. A failure keeps Interaction production execution disabled.

### Phase 4A: Qualify The Real Pmd Snapshot Used By Flow

- Complete the source contract started in Phase 1 for a target-bound runtime
  snapshot with protected-auth proof, `QualifiedGeometry`, orientation/profile
  identity, and explicit readiness before and after stream parking. Never promote
  cached `Starting` state to `Ready`.
- Obtain the snapshot inside the existing exclusive control-plane ownership chain;
  do not call generic preflight or create an unowned session/stream.
- Add focused Pmd/Flow preflight tests and a Mac live micro-gate on the exact tuple.
- Keep coordinate picker and Pmd UI Flow execution fail-closed until this gate passes.

**Exit:** Flow has a real, target-qualified auth/geometry/readiness contract rather
than manifest dimensions or fabricated metadata. This phase is required before the
live portion of F3, but does not enable an Interaction action by itself.

### Phase 5: Interaction G1 Campaign Core

- Add `crates/core/src/interaction/**`: versioned domain, backend URL parser/resolver,
  deterministic planner, exhaustive aggregation, migration 3, serialized store,
  immutable actor/target/assignment snapshots, transitions, pagination, artifacts,
  scheduler, dispatcher, recovery, and retention.
- Use `DeviceControlPlane` and `StreamBudgetManager` as the only allocator; do not
  add campaign-local UDID locks or producer semaphores.
- Add `interaction_commands.rs`, TypeScript DTOs/API, post-commit events, startup
  recovery, and shutdown ownership.
- Create the named `interaction_planner` and `interaction_aggregation` integration
  test targets used by the detailed gate commands.
- Extend the combined admission/exit-order tests with a real Interaction dispatcher
  and scheduler blocked at deterministic barriers; F2 alone proves only the
  pre-Interaction portion of shutdown.
- Keep the production batch executor typed-disabled; mock mode may use deterministic
  fake actions for durability tests.

**Exit G1:** durability/restart/cancel/retry/idempotency suites and full regression
pass, while real execution remains disabled pending G2.

### Phase 6: Interaction G2 Verified Actions

- Extract shared frame observation and coordinate rail locator without regressing
  Nurture.
- Implement mandatory Copy Link identity attempts with durable intent.
- Add frame-verified Watch, Like, Follow, prepared Comment, text health recovery,
  and the G0-owned device batch adapter.
- Run Nurture parity tests and the fixed Mac/device G2 probe.
- Promote only action capabilities that pass on the exact tuple.

**Exit G2:** target identity and the enabled action set have fixture plus live proof;
disabled actions keep exact backend reasons.

### Phase 7: Interaction G3 Operator Workflow

- Reuse the F2 test/tooling setup.
- Build the pure Interaction draft and typed client.
- Add the `Tương tác` button and one responsive Setup/Monitor panel.
- Implement link parsing feedback, target table overrides, default account rows,
  All/Explicit actor selection, All/RoundRobin distribution, preview, Run Now, Once,
  and schedule validation.
- Restore paged campaigns after reopen/restart and expose cancel, eligible retry, and
  coordinated Open on Device.
- Preserve honest parked device tiles. Keep proxy state manual/unsupervised as
  documented; no system apply claim.
- Run component, browser visual, redaction, and full G0-G2 regression gates.

**Exit G3:** the requested menu/paste/select/schedule/campaign workflow is usable
with only live-qualified actions.

### Phase 8: Live Acceptance And Packaging

- Before G5, run G0/G2 live qualification for both acceptance devices. If their
  registry dimensions differ, add a separately reviewed exact tuple; if they match,
  the evidence must still prove both devices map to that already attested tuple. G5
  observes capabilities and never creates a new capability claim.
- Finish Flow F3 on Mac/iPhone only after Phase 4A, with qualified
  geometry/auth/session/stream evidence.
- Run Interaction G5 with two devices for All and RoundRobin, producer budgets 1/2,
  Unicode Comment, cancellation, busy partial, crash recovery, no replay, package
  smoke, and rollback.
- G4 optional actions are not required for this delivery.

**Exit:** F3 and G5 reports PASS, packaged desktop smoke passes, and rollback proof
is recorded. Production artifacts are changed only by their separate Agent release
gate.

---

## 5. Acceptance Checklist

### Interaction

- Direct video, photo, and short links parse with per-line status and identity
  deduplication.
- AllOnline/Explicit and All/RoundRobin produce deterministic previews and immutable
  execution snapshots.
- Run Now and Once survive desktop restart without duplicate work.
- One failed assignment does not stop unrelated assignments.
- Monitor restores from SQLite and shows per-device/target/action state and evidence.
- Cancel/retry obey backend eligibility; ambiguous issued work is never replayed.
- Existing Nurture remains functional and cannot share a UDID with Interaction.

### Flow V2

- Operator can create, connect, inspect, validate, save, reopen, duplicate, import,
  export, and run a release-1 graph without editing JSON.
- Draft layout never changes the immutable execution hash.
- One/Selected/AllEligible runs persist independent device/node attempts.
- Cancel/retry/restart and screenshot artifacts follow exact evidence rules.
- Narrow and desktop viewports remain usable with no incoherent overlap.

### Shared Safety And Operations

- One screen-changing owner per UDID and one global producer budget.
- Session always precedes MJPEG; stale generations never publish.
- No SQLite transaction spans an await/device/network operation.
- No token, raw clipboard, proxy password, or unredacted UDID enters events/reports.
- Startup recovery precedes namespace reconciliation; shutdown joins every worker.
- Production IPA and manifest retain the locked hashes throughout this delivery.

---

## 6. Verification Matrix

Run after every Rust slice:

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

Run after every desktop slice:

```powershell
npm --prefix apps/desktop test -- --run
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
```

Run the named Python suites for touched sidecar/probe code. Playwright and live Mac
commands come from the detailed F3/G0/G2/G3/G5 plans and must retain fixed thresholds.

Every gate commit updates `AGENTS.md` with commit range, exact test counts, enabled
and disabled behavior, remaining Mac/device work, artifact hashes, and rollback.
