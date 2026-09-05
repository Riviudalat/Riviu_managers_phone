# TikTok Interaction Desktop Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver Gate G3: a durable Setup/Monitor desktop workflow for TikTok Interaction campaigns, a coordinated Open on Device command, and a restored Proxy page whose current unsupervised-device state is explicit and truthful.

**Architecture:** React owns only editable draft state, presentation state, and paged cache invalidation; Gate G1's backend resolver, planner, SQLite store, scheduler, retry classifier, and revisioned events remain authoritative. Every device navigation uses Gate G0's single `DeviceControlPlane` with `DeviceWorkOwner::ManualControl`, while proxy catalog edits and device assignments preserve Gate G1's non-secret revision snapshot contract.

**Tech Stack:** React 19, TypeScript 6, Vitest 4, Testing Library, happy-dom, Playwright, Tauri 2, Rust 2021, Tokio, rusqlite/SQLite WAL, Gate G0 `DeviceControlPlane`, Gate G1 Interaction domain/API, Gate G2 verified-action facade.

---

## Execution Preconditions

- Execute this plan only after G0, G1, and G2 are merged and their named verification commands pass. Read `AGENTS.md`, the approved design at `docs/archive/specs/2026-07-29-tiktok-interaction-campaign-design.md`, and the three prerequisite plans before Task 1.
- Use an isolated worktree created with the `using-git-worktrees` skill. The current shared checkout is dirty; never stage, reset, format, or overwrite unrelated work.
- Treat the G1 contracts as canonical: `InteractionCampaignRequest`, `TikTokTargetInput`, `ActorSelection`, `DistributionMode`, `ScheduleMode`, `InteractionDefaults`, `InteractionOverrides`, `ActionPolicy`, `RecipientPolicy`, `AccountBinding`, `CampaignStatus`, `AssignmentStatus`, `AssignmentResultCode`, `RetryBlockedReason`, `IdentityCopyIntent`, `CapabilitySnapshot`, `CapabilityState`, `CampaignSummary`, `Page<T>`, and the approved `interaction_*` commands. Do not introduce a second parser, planner, campaign store, scheduler, capability map, or client-side resolved-target request.
- Treat G0's `DeviceControlPlane`, `DeviceWorkOwner::ManualControl`, typed `DeviceBusy`, session-before-stream transition, and producer-counted stream budget as mandatory. `interaction_open_on_device` must not call a driver or `ensure_stream` directly.
- Gate G4 is not a prerequisite. Save, Repost, and Direct Message remain visible through generic capability-driven controls but disabled with the exact backend reason before their independent G4 qualification. Do not infer availability from `AgentStatus.features`.
- Keep `sidecars/wda/RiviuAgent.ipa`, `sidecars/wda/agent-manifest.json`, and `sidecars/wda/WebDriverAgent/**` byte-identical.
- Each commit command below stages only paths named by its task. Run `git diff --cached --name-only` before every commit.

## Product Boundaries For G3

- The `Tương tác` button sits immediately after `Nuôi TT`. Nurture and Interaction use one discriminated `activeTool` state, so exactly one tool panel is mounted.
- The Interaction panel is a 720-780 px desktop tool surface with responsive narrowing. It overlays from the right without replacing or reinitializing the device grid.
- Setup sends raw TikTok URLs plus overrides to G1. Parsed normalized URLs are display data only; preview, start, and schedule always re-resolve in Rust.
- Monitor is rebuilt from SQLite after mount/reconnect. `InteractionUpdated` is an invalidation hint, not an event-sourced state cache.
- `Uncertain` and issued-but-unconfirmed identity work is read-only. Only backend-projected `retryEligible=true` assignments may be selected for retry. The backend may authorize either a Failed/Interrupted optional effect after Confirmed identity or a terminal pre-Copy failure whose current intent is `None`; both paths append a new Pending/None identity attempt before device work, so the UI never reuses or resets a prior Copy Link row and never derives eligibility from status/code alone.
- Direct Message recipient policy uses G1 `RecipientPolicy { mode: RecipientMode, allowlist }` with `RecipientMode::{Allowlist, RandomVisible}`. Current G2 capabilities return `Deferred/GateNotQualified`, so the action cannot be selected in production before G4.
- Proxy endpoint reachability means a bounded TCP connection from the desktop to the configured endpoint. It is independent from the operator's manual iPhone confirmation and is never displayed as device egress, device IP verification, or applied system proxy state.
- Existing proxy credential storage and explicit export remain compatible. Passwords and authenticated URLs never enter Interaction snapshots, `InteractionUpdated`, proxy-state events, evidence, traces, or endpoint-test errors.

## File Map

**Create**

- `apps/desktop/vitest.config.ts`: component-test environment and setup.
- `apps/desktop/src/test/setup.ts`: Testing Library cleanup.
- `apps/desktop/src/components/InlineNotice.tsx`: reusable inline error/status surface that replaces modal alerts in G3 flows.
- `apps/desktop/src/components/InlineNotice.test.tsx`: notice accessibility test.
- `apps/desktop/src/components/DeviceTile.test.tsx`: parked, stale, waiting, and frame-age presentation tests.
- `apps/desktop/src/interaction/interactionDraft.ts`: pure draft reducer, stale parser revision guard, validation, override serialization, and request construction.
- `apps/desktop/src/interaction/interactionDraft.test.ts`: draft, validation, selection, schedule, policy, and serialization tests.
- `apps/desktop/src/interaction/InteractionPolicyControl.tsx`: Off/Required/Probability/Inherit control with capability gating.
- `apps/desktop/src/interaction/InteractionActorGrid.tsx`: account/device selection and inherited proxy/capability display.
- `apps/desktop/src/interaction/InteractionTargetTable.tsx`: parsed target rows and inline per-target overrides.
- `apps/desktop/src/interaction/InteractionSetup.tsx`: complete Setup tab presentation.
- `apps/desktop/src/interaction/InteractionSetup.test.tsx`: Setup behavior and conditional-field tests.
- `apps/desktop/src/interaction/interactionClient.ts`: injectable interface backed only by the canonical G1 Tauri wrappers.
- `apps/desktop/src/interaction/useInteractionSetup.ts`: defaults/accounts/parser/preview controller with stale-response protection.
- `apps/desktop/src/interaction/useInteractionSetup.test.tsx`: deferred-response and preview invalidation tests.
- `apps/desktop/src/interaction/InteractionPanel.tsx`: responsive Setup/Monitor shell and run/schedule submission lifecycle.
- `apps/desktop/src/interaction/InteractionPanel.test.tsx`: panel submission, request-ID, and inline-error tests.
- `apps/desktop/src/interaction/useInteractionMonitor.ts`: durable restore, revision invalidation, and cursor-backed page caches.
- `apps/desktop/src/interaction/VirtualizedRows.tsx`: fixed-row virtual window used by large target/assignment lists.
- `apps/desktop/src/interaction/InteractionMonitor.tsx`: campaign summary, paged target/actor/action/evidence detail, and commands.
- `apps/desktop/src/interaction/InteractionMonitor.test.tsx`: restore, pagination, event, uncertain, stop/retry/open tests.
- `apps/desktop/src/interaction/interaction.css`: panel, Setup, Monitor, status, and responsive layout.
- `crates/core/src/proxy.rs`: revision-keyed proxy annotations and injected endpoint connector.
- `crates/core/tests/proxy_state.rs`: migration, assignment, invalidation, redaction, and endpoint-CAS tests.
- `apps/desktop/src/pages/ProxyPage.tsx`: focused proxy CRUD/export/assignment/reachability/manual-confirmation page.
- `apps/desktop/src/pages/ProxyPage.test.tsx`: truthful proxy-state and no-secret-event UI tests.
- `apps/desktop/playwright.config.ts`: deterministic visual test server configuration.
- `apps/desktop/interaction-harness.html`: browser-only G3 visual harness entry.
- `apps/desktop/src/test/interactionHarnessMain.tsx`: deterministic Setup/Monitor/Proxy fixture renderer.
- `apps/desktop/e2e/interaction-workflow.spec.ts`: width, overflow, responsive, and panel/grid coexistence checks.

**Modify**

- `apps/desktop/package.json`
- `apps/desktop/package-lock.json`
- `apps/desktop/src-tauri/src/interaction_commands.rs`
- `apps/desktop/src-tauri/src/farm_commands.rs`
- `apps/desktop/src-tauri/src/lib.rs`
- `apps/desktop/src-tauri/src/state.rs`
- `crates/core/src/interaction/schema.rs`
- `crates/core/src/interaction/store.rs`
- `crates/core/src/interaction/types.rs`
- `crates/core/src/lib.rs`
- `crates/core/src/db.rs`
- `crates/core/src/types.rs`
- `apps/desktop/src/types.ts`
- `apps/desktop/src/api.ts`
- `apps/desktop/src/interactionApi.test.ts`
- `apps/desktop/src/App.tsx`
- `apps/desktop/src/App.css`
- `apps/desktop/src/components/ProfileToolbar.tsx`
- `apps/desktop/src/components/DeviceTile.tsx`
- `apps/desktop/src/frameStore.ts`
- `apps/desktop/src/pages/FarmPages.tsx`
- `apps/desktop/src/components/Sidebar.tsx`
- `AGENTS.md`

**Do Not Modify**

- `crates/core/src/nurture/**`
- `crates/core/src/tiktok_actions/**`
- `crates/ios-driver/**`
- `sidecars/wda/**`
- production capability registry entries

---

### Task 1: Establish Component Tests And Inline Runtime Feedback

**Files:**
- Modify: `apps/desktop/package.json`
- Modify: `apps/desktop/package-lock.json`
- Create: `apps/desktop/vitest.config.ts`
- Create: `apps/desktop/src/test/setup.ts`
- Create: `apps/desktop/src/components/InlineNotice.test.tsx`
- Create: `apps/desktop/src/components/InlineNotice.tsx`

- [ ] **Step 1: Install the desktop component-test dependencies**

Run:

```powershell
npm --prefix apps/desktop install --save-dev @testing-library/react @testing-library/user-event @testing-library/jest-dom happy-dom
```

Expected: `package.json` and `package-lock.json` add only the four development dependencies; existing runtime dependencies do not change.

- [ ] **Step 2: Configure Vitest for React DOM cleanup**

Create `apps/desktop/vitest.config.ts`:

```ts
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "happy-dom",
    setupFiles: ["./src/test/setup.ts"],
    css: true,
  },
});
```

Create `apps/desktop/src/test/setup.ts`:

```ts
import { cleanup } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";

afterEach(() => cleanup());
```

- [ ] **Step 3: Write the failing inline-notice test**

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { InlineNotice } from "./InlineNotice";

describe("InlineNotice", () => {
  it("announces an interaction error without a modal", () => {
    render(<InlineNotice tone="error">Thiết bị đang bận</InlineNotice>);
    expect(screen.getByRole("alert").textContent).toContain("Thiết bị đang bận");
  });

  it("uses status semantics for non-error feedback", () => {
    render(<InlineNotice tone="info">Đã lưu lịch</InlineNotice>);
    expect(screen.getByRole("status").textContent).toContain("Đã lưu lịch");
  });
});
```

Run:

```powershell
npm --prefix apps/desktop test -- InlineNotice.test.tsx
```

Expected: FAIL because `InlineNotice.tsx` does not exist.

- [ ] **Step 4: Implement the bounded notice surface**

```tsx
import type { PropsWithChildren } from "react";

export function InlineNotice({
  tone,
  children,
}: PropsWithChildren<{ tone: "info" | "warning" | "error" }>) {
  return (
    <div
      className={`inline-notice ${tone}`}
      role={tone === "error" ? "alert" : "status"}
      aria-live={tone === "error" ? "assertive" : "polite"}
    >
      {children}
    </div>
  );
}
```

Add restrained `.inline-notice` styles to `apps/desktop/src/App.css`; use existing `--line`, `--danger`, `--warn`, and `--bg-muted` tokens. Do not add a floating toast or modal.

- [ ] **Step 5: Verify and commit**

```powershell
npm --prefix apps/desktop test -- InlineNotice.test.tsx
npm --prefix apps/desktop run build
git add apps/desktop/package.json apps/desktop/package-lock.json apps/desktop/vitest.config.ts apps/desktop/src/test/setup.ts apps/desktop/src/components/InlineNotice.tsx apps/desktop/src/components/InlineNotice.test.tsx apps/desktop/src/App.css
git diff --cached --name-only
git commit -m "test(desktop): add interaction component harness"
```

Expected: two notice tests pass and the desktop build succeeds.

---

### Task 2: Complete The Canonical Open On Device Command

**Files:**
- Modify: `apps/desktop/src-tauri/src/interaction_commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/src/state.rs`
- Modify: `apps/desktop/src/types.ts`
- Modify: `apps/desktop/src/api.ts`
- Modify: `apps/desktop/src/interactionApi.test.ts`

- [ ] **Step 1: Write failing Rust coordination tests**

Add plain-helper tests beside the G1 command tests. Seed an immutable assignment and record every control-plane transition:

```rust
#[tokio::test]
async fn open_on_device_uses_manual_control_and_session_before_stream() {
    let fixture = open_fixture().await;
    let result = open_on_device_impl(&fixture.service, fixture.assignment_id)
        .await
        .unwrap();

    assert_eq!(result.udid, "fixture-udid");
    assert_eq!(fixture.calls(), vec![
        "resolve:content-1",
        "acquire:manualControl",
        "inspect",
        "reserveUiCapacity",
        "foreground:com.ss.iphone.ugc.Ame",
        "session:standard",
        "stream:firstFrame",
        "openUrl:content-1",
        "frame:afterOpen",
        "release",
    ]);
    assert_eq!(fixture.store.assignment_revision(fixture.assignment_id).await, 0);
    assert_eq!(fixture.copy_link_count(), 0);
    assert_eq!(fixture.store.count_opening_attempts(fixture.assignment_id).await.unwrap(), 0);
}

#[tokio::test]
async fn open_on_device_returns_device_busy_before_navigation() {
    let fixture = open_fixture_owned_by(DeviceWorkOwner::Interaction).await;
    let error = open_on_device_impl(&fixture.service, fixture.assignment_id)
        .await
        .unwrap_err();

    assert_eq!(error.code, "device_busy");
    assert!(error.retryable);
    assert_eq!(fixture.open_url_count(), 0);
    assert_eq!(fixture.current_owner(), DeviceWorkOwner::Interaction);
}
```

Also test target re-resolution returning changed/unverified, unsupported `openUrl`, session failure, first-frame failure, open failure, and cancellation of the Tauri future. Every branch releases the exact context/stream reservation it acquired and never changes assignment/action status.

Run:

```powershell
cargo test -p riviu-managers-phone interaction_commands::tests::open_on_device -- --nocapture
```

Expected: FAIL because `open_on_device_impl` and the registered command do not exist.

- [ ] **Step 2: Implement the manual-control helper through G0**

Add the response DTO at the Tauri boundary:

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionOpenOnDeviceResult {
    pub assignment_id: String,
    pub udid: String,
    pub target_key: String,
    pub frame_generation: u64,
}
```

`open_on_device_impl` must perform this fixed sequence:

1. Load the G1 `AssignmentView` and immutable target/account snapshot.
2. Re-resolve through the existing G1 `InteractionTargetResolver`; reject `Changed` and `Unverified` before device acquisition.
3. Call the one G0 `DeviceControlPlane::try_acquire_exclusive(udid, DeviceWorkOwner::ManualControl)`.
4. Inspect and require the current tuple's G0 `open_url` capability; no install/repair is initiated from this manual command.
5. Reserve foreground stream capacity, foreground TikTok, create the profile-approved standard session, then start MJPEG and await its first current-generation JPEG.
6. Send the profile-specific typed `open_url` request with the resolver-produced URL, await one newer frame, and return only assignment ID, UDID, target key, and frame generation.
7. Close transient ownership and release the matching context in all success/error/cancellation paths. The background sampler may later reacquire capacity.

Do not run Copy Link, mark identity, perform optional actions, mutate the assignment, or retain a second manual lease after the command returns. Map G0 `DeviceBusy { current_owner, .. }` to the existing serializable `InteractionCommandError` with code `device_busy`; the message may name the typed owner but must contain no URL, UDID, token, filesystem path, or proxy field.

- [ ] **Step 3: Register the approved command and write the TypeScript RED test**

Register exactly `interaction_commands::interaction_open_on_device` in the Tauri handler. Add to `apps/desktop/src/interactionApi.test.ts`:

```ts
it("opens an assignment through the canonical manual-control command", async () => {
  vi.mocked(invoke).mockResolvedValue({
    assignmentId: "assignment-1",
    udid: "fixture-udid",
    targetKey: "content:7657447099239271697",
    frameGeneration: 7,
  });

  await openInteractionAssignment("assignment-1");

  expect(invoke).toHaveBeenCalledWith("interaction_open_on_device", {
    assignmentId: "assignment-1",
  });
});
```

Run:

```powershell
npm --prefix apps/desktop test -- interactionApi.test.ts
```

Expected: FAIL because `openInteractionAssignment` is missing.

- [ ] **Step 4: Add the thin TypeScript wrapper**

Mirror `InteractionOpenOnDeviceResult` in `apps/desktop/src/types.ts` and add:

```ts
export async function openInteractionAssignment(assignmentId: string) {
  return invoke<InteractionOpenOnDeviceResult>("interaction_open_on_device", {
    assignmentId,
  });
}
```

Keep the G1 `InteractionCommandError` code/retryable shape intact. Do not parse localized error strings to detect `DeviceBusy`.

- [ ] **Step 5: Verify and commit**

```powershell
cargo test -p riviu-managers-phone interaction_commands::tests::open_on_device -- --nocapture
npm --prefix apps/desktop test -- interactionApi.test.ts
cargo check -p riviu-managers-phone
npm --prefix apps/desktop run build
git add apps/desktop/src-tauri/src/interaction_commands.rs apps/desktop/src-tauri/src/lib.rs apps/desktop/src-tauri/src/state.rs apps/desktop/src/types.ts apps/desktop/src/api.ts apps/desktop/src/interactionApi.test.ts
git diff --cached --name-only
git commit -m "feat(desktop): open interaction target with manual control"
```

Expected: Busy is returned before navigation, successful call order is session-before-stream, and no campaign row changes.

---

### Task 3: Build The Pure Interaction Draft And Validation Model

**Files:**
- Create: `apps/desktop/src/interaction/interactionDraft.test.ts`
- Create: `apps/desktop/src/interaction/interactionDraft.ts`

- [ ] **Step 1: Write failing policy, selection, override, and parser-revision tests**

Use the canonical G1 camel-case DTOs. The tests must cover:

```ts
it("serializes Inherit by omitting the target field", () => {
  const draft = validDraft();
  draft.targets[0].overrides.like = { mode: "inherit" };
  draft.targets[0].overrides.comment = { mode: "probability", percent: 35 };

  const request = buildCampaignRequest(draft, "request-1");

  expect(request.targets[0].overrides.like).toBeUndefined();
  expect(request.targets[0].overrides.comment).toEqual({
    probability: { percent: 35 },
  });
  expect(request.targets[0]).toEqual({
    url: "https://www.tiktok.com/@fixture/video/7657447099239271697",
    overrides: expect.any(Object),
  });
  expect(request.targets[0]).not.toHaveProperty("contentId");
  expect(request.targets[0]).not.toHaveProperty("resolvedUrl");
});

it("requires an account only in explicit mode", () => {
  expect(validateDraft(validDraft({ actorMode: "allOnline" }))).toEqual([]);
  expect(validateDraft(validDraft({ actorMode: "explicit", accountIds: [] })))
    .toContainEqual(expect.objectContaining({ code: "explicit_actor_required" }));
});

it("ignores a parser response older than the current input revision", () => {
  let state = initialDraftState();
  state = draftReducer(state, { type: "linksChanged", rawText: "first" });
  const firstRevision = state.parseRevision;
  state = draftReducer(state, { type: "linksChanged", rawText: "second" });
  const currentRevision = state.parseRevision;

  state = draftReducer(state, parsedSuccess(currentRevision, "content:second"));
  state = draftReducer(state, parsedSuccess(firstRevision, "content:first"));

  expect(state.targets.map((target) => target.targetKey)).toEqual(["content:second"]);
});
```

Add cases for `All` and `RoundRobin`, duplicate parsed content, at least one valid URL, one-time schedule in the future, watch range `1..=300`, min <= max for all duration/pacing pairs, pacing `0..=60_000`, probability `0..=100`, Comment requiring AI runtime or nonblank fallback pool, exact ASCII `@handle` allowlist entries, duplicate-label conflict, and Direct Message Allowlist requiring at least one recipient only when the action is runnable.

Run:

```powershell
npm --prefix apps/desktop test -- interactionDraft.test.ts
```

Expected: FAIL because the draft module does not exist.

- [ ] **Step 2: Define the closed draft model**

Create these UI-only editing types; serialized API types continue to come from `../types`:

```ts
export type ActionPolicyDraft =
  | { mode: "off" }
  | { mode: "required" }
  | { mode: "probability"; percent: number };

export type OverridePolicyDraft =
  | { mode: "inherit" }
  | ActionPolicyDraft;

export type OverrideValue<T> =
  | { mode: "inherit" }
  | { mode: "value"; value: T };

export interface DraftIssue {
  path: string;
  code:
    | "valid_target_required"
    | "explicit_actor_required"
    | "invalid_probability"
    | "invalid_range"
    | "comment_source_required"
    | "recipient_required"
    | "invalid_recipient_handle"
    | "recipient_label_conflict"
    | "schedule_time_required"
    | "schedule_time_not_future";
  message: string;
}

export interface ParsedDraftTarget {
  targetKey: string;
  sourceUrl: string;
  normalizedUrl: string;
  author: string | null;
  kind: "video" | "photo";
  overrides: TargetOverrideDraft;
}
```

`InteractionDraftState` contains `rawLinks`, `parseRevision`, `parseState`, `lineResults`, deduplicated `targets`, `actorMode`, ordered `selectedAccountIds`, `distribution`, `defaults`, `scheduleMode`, `scheduleAtLocal`, and current preview state. Product defaults must match G1 exactly: Watch Required, 4-12 seconds, action delay 600-1,800 ms, target delay 1,500-4,000 ms, every optional action Off, empty AI instruction/fallback pool, and empty Allowlist recipient policy.

- [ ] **Step 3: Implement explicit action-policy conversion**

Use the G1 adjacent enum representation already exported by `types.ts`; keep conversion in one function:

```ts
export function serializePolicy(policy: ActionPolicyDraft): ActionPolicy {
  switch (policy.mode) {
    case "off":
      return "off";
    case "required":
      return "required";
    case "probability":
      return { probability: { percent: policy.percent } };
  }
}

export function serializeOverridePolicy(
  policy: OverridePolicyDraft,
): ActionPolicy | undefined {
  return policy.mode === "inherit" ? undefined : serializePolicy(policy);
}
```

This exact external-tag representation matches G1's serde contract: unit variants are strings and the Probability struct variant is `{ probability: { percent } }`. Do not alter the Rust enum or add a second accepted shape.

- [ ] **Step 4: Implement validation and raw-request construction**

`validateDraft` must return all field issues without throwing. `buildCampaignRequest` runs only with zero issues and constructs:

```ts
return {
  requestId,
  actorSelection:
    draft.actorMode === "allOnline"
      ? { mode: "allOnline" }
      : { mode: "explicit", accountIds: draft.selectedAccountIds },
  distribution: draft.distribution,
  schedule:
    draft.scheduleMode === "runNow"
      ? { mode: "runNow" }
      : { mode: "once", at: new Date(draft.scheduleAtLocal).toISOString() },
  defaults: serializeDefaults(draft.defaults),
  targets: draft.targets.map((target) => ({
    url: target.sourceUrl,
    overrides: serializeOverrides(target.overrides),
  })),
} satisfies InteractionCampaignRequest;
```

`serializeOverrides` omits every Inherit field rather than copying current defaults. Preserve parsed target order and explicit account order. The request contains no normalized URL, redirect result, content ID, author, post kind, capability claim, proxy secret, or planner count.

- [ ] **Step 5: Verify and commit**

```powershell
npm --prefix apps/desktop test -- interactionDraft.test.ts
npm --prefix apps/desktop run build
git add apps/desktop/src/interaction/interactionDraft.ts apps/desktop/src/interaction/interactionDraft.test.ts
git diff --cached --name-only
git commit -m "feat(desktop): model interaction campaign drafts"
```

Expected: all policy boundaries, actor/distribution modes, stale parse ordering, schedule validation, and Inherit omission pass.

---

### Task 4: Render The Complete Setup Tab

**Files:**
- Create: `apps/desktop/src/interaction/InteractionPolicyControl.tsx`
- Create: `apps/desktop/src/interaction/InteractionActorGrid.tsx`
- Create: `apps/desktop/src/interaction/InteractionTargetTable.tsx`
- Create: `apps/desktop/src/interaction/InteractionSetup.tsx`
- Create: `apps/desktop/src/interaction/InteractionSetup.test.tsx`
- Create: `apps/desktop/src/interaction/interaction.css`

- [ ] **Step 1: Write failing Setup interaction tests**

Render with deterministic accounts, device rows, parse results, preview, and capability states. Assert:

```tsx
it("shows the approved selectors and fixed identity prerequisite", async () => {
  const user = userEvent.setup();
  render(<InteractionSetup {...fixtureProps()} />);

  expect(screen.getByRole("button", { name: "Tất cả máy online" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Chỉ định" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Tất cả" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Phân bổ" })).toBeTruthy();
  expect(screen.getByText("Xác minh bài bằng Copy Link")).toBeTruthy();
  expect(screen.queryByLabelText("Tắt xác minh bài")).toBeNull();

  await user.click(screen.getByRole("button", { name: "Chỉ định" }));
  expect(screen.getByRole("grid", { name: "Thiết bị và tài khoản" })).toBeTruthy();
});

it("keeps target overrides as Inherit until explicitly changed", async () => {
  const user = userEvent.setup();
  const props = fixtureProps();
  render(<InteractionSetup {...props} />);

  await user.click(screen.getByRole("button", { name: "Sửa bài 1" }));
  expect(screen.getByLabelText("Like cho bài 1")).toHaveValue("inherit");
  await user.selectOptions(screen.getByLabelText("Like cho bài 1"), "required");

  expect(props.onTargetOverride).toHaveBeenCalledWith(
    "content:7657447099239271697",
    "like",
    { mode: "required" },
  );
});
```

Add tests for multiline input with per-line error text, normalized video/photo target display, expected actor count, action summary, parse/preview state, watch and both pacing ranges, every optional action's Off/Required/Probability control, probability input appearance, Comment AI instruction/fallback pool only when Comment can run, and no nested card structure.

Add two Direct Message tests:

1. `Deferred { code: GateNotQualified }` disables the action and renders the typed reason `gate_not_qualified`.
2. A future fixture with `Ready` allows Required and reveals the generic `Allowlist | Ngẫu nhiên trong danh sách hiện có` recipient control plus exact `@handle` rows.

Run:

```powershell
npm --prefix apps/desktop test -- InteractionSetup.test.tsx
```

Expected: FAIL because Setup components do not exist.

- [ ] **Step 2: Implement segmented modes and the actor grid**

Use buttons with `aria-pressed` for the two actor modes and two distribution modes. `InteractionActorGrid` groups canonical `AccountBinding` rows by `deviceUdid`, but each checkbox value is `account.id`. The current backend returns only the one default binding per iPhone; retaining account IDs and slot labels keeps the UI shape ready for a later qualified account-switch feature without exposing dormant non-default rows now.

Each fixed-height grid row displays device name/model/connection, account label/slot, online state, inherited `device_meta.proxy_id` display label, and typed capability summary. It has no proxy editor. In `allOnline` mode checkboxes are disabled and the preview actor snapshot is labeled provisional; in `explicit` mode at least one checked account is required.

- [ ] **Step 3: Implement the policy and target override controls**

`InteractionPolicyControl` accepts this closed surface:

```ts
interface InteractionPolicyControlProps {
  id: string;
  label: string;
  value: ActionPolicyDraft | OverridePolicyDraft;
  allowInherit: boolean;
  capability: CapabilityState;
  onChange: (next: ActionPolicyDraft | OverridePolicyDraft) => void;
}
```

Render a compact select with Inherit only for target overrides and Off/Required/Probability for optional actions. Render a numeric percent stepper only for Probability. When capability is `Unavailable` or `Deferred`, disable the complete control and show the exact typed reason code beside it; never turn the value to Ready in the client.

`InteractionTargetTable` uses one unframed table/list, an edit icon button with an accessible `Sửa bài N` name, and one expanded override band. It shows normalized target, expected actors, effective action summary, and parser/preview state. It sends `sourceUrl` and override changes back to the reducer; it never edits `ResolvedTikTokTarget` fields.

- [ ] **Step 4: Implement the full Setup form**

Order the form as:

1. actor mode and account/device grid;
2. distribution mode;
3. multiline TikTok links with line-numbered outcomes;
4. compact target table and one expanded override;
5. fixed Copy Link verification row;
6. Watch duration and action/target pacing;
7. optional action policies;
8. conditional Comment AI instruction/fallback pool;
9. conditional Direct Message recipient policy;
10. preview summary and warnings;
11. Run Now/Once control and submit action slot.

Use `textarea`, segmented controls, checkboxes, selects, and numeric inputs. Do not add explanatory marketing copy, nested cards, oversized headings, gradients, or viewport-scaled fonts.

In `interaction.css`, constrain row tracks with `minmax(0, 1fr)`, apply `overflow-wrap: anywhere` to normalized URLs and typed errors, and keep all fixed controls from resizing on status changes.

- [ ] **Step 5: Verify and commit**

```powershell
npm --prefix apps/desktop test -- InteractionSetup.test.tsx interactionDraft.test.ts
npm --prefix apps/desktop run build
git add apps/desktop/src/interaction/InteractionPolicyControl.tsx apps/desktop/src/interaction/InteractionActorGrid.tsx apps/desktop/src/interaction/InteractionTargetTable.tsx apps/desktop/src/interaction/InteractionSetup.tsx apps/desktop/src/interaction/InteractionSetup.test.tsx apps/desktop/src/interaction/interaction.css
git diff --cached --name-only
git commit -m "feat(desktop): render interaction setup workflow"
```

Expected: Setup exposes All/Specified, All/RoundRobin, per-line errors, Inherit overrides, fixed Copy Link, watch/pacing, AI Comment, and capability-gated DM controls.

---

### Task 5: Connect Backend Parsing, Accounts, Capabilities, And Preview

**Files:**
- Create: `apps/desktop/src/interaction/interactionClient.ts`
- Create: `apps/desktop/src/interaction/useInteractionSetup.ts`
- Create: `apps/desktop/src/interaction/useInteractionSetup.test.tsx`
- Modify: `apps/desktop/src/interaction/InteractionSetup.tsx`
- Modify: `apps/desktop/src/interaction/InteractionSetup.test.tsx`

- [ ] **Step 1: Define an injectable client using only G1 wrappers**

```ts
export interface InteractionClient {
  parseLinks(rawText: string): Promise<ParseLinksResult>;
  preview(request: InteractionCampaignRequest): Promise<InteractionPreview>;
  getDefaults(): Promise<InteractionDefaults>;
  saveDefaults(settings: InteractionDefaults): Promise<InteractionDefaults>;
  listAccounts(udids: string[]): Promise<AccountBinding[]>;
  start(request: InteractionCampaignRequest): Promise<CampaignSummary>;
  schedule(request: InteractionCampaignRequest): Promise<CampaignSummary>;
  get(campaignId: string): Promise<CampaignDetail>;
  list(cursor: string | null, limit: number, statuses: CampaignStatus[]): Promise<Page<CampaignSummary>>;
  listTargets(campaignId: string, cursor: string | null, limit: number): Promise<Page<TargetView>>;
  listAssignments(campaignId: string, filters: AssignmentListFilter, cursor: string | null, limit: number): Promise<Page<AssignmentView>>;
  getAssignment(assignmentId: string): Promise<AssignmentView>;
  listActionRuns(assignmentId: string, cursor: string | null, limit: number): Promise<Page<ActionRunView>>;
  cancel(campaignId: string): Promise<CampaignSummary>;
  retry(campaignId: string, assignmentIds: string[], retryRequestId: string): Promise<CampaignSummary>;
  openOnDevice(assignmentId: string): Promise<InteractionOpenOnDeviceResult>;
  onUpdated(handler: (event: InteractionUpdated) => void): Promise<() => void>;
}
```

`tauriInteractionClient` delegates directly to the wrappers added by G1/Task 2. It does not cache, normalize URLs, hash requests, infer capability, or translate result codes.

- [ ] **Step 2: Write the failing stale-response controller test**

Use deferred promises so the second parse resolves first:

```tsx
it("never applies an older Tauri parser response", async () => {
  const first = deferred<ParseLinksResult>();
  const second = deferred<ParseLinksResult>();
  const client = fixtureClient({ parseLinks: vi.fn()
    .mockReturnValueOnce(first.promise)
    .mockReturnValueOnce(second.promise) });

  const { result } = renderHook(() => useInteractionSetup({
    client,
    devices: fixtureDevices(),
    initiallySelectedUdids: [],
  }));

  act(() => result.current.setRawLinks("first"));
  await advanceParseDebounce();
  act(() => result.current.setRawLinks("second"));
  await advanceParseDebounce();

  await act(() => second.resolve(parseResult("content:second")));
  await act(() => first.resolve(parseResult("content:first")));

  expect(result.current.state.targets[0].targetKey).toBe("content:second");
});
```

Also test that preview response N is ignored after any link/default/actor/distribution/override/schedule revision N+1; parser errors remain per-line; and preview failure appears inline without erasing a valid parsed draft.

Run:

```powershell
npm --prefix apps/desktop test -- useInteractionSetup.test.tsx
```

Expected: FAIL because the controller does not exist.

- [ ] **Step 3: Implement bounded asynchronous setup state**

On mount, fetch G1 defaults and `interaction_list_accounts` for the current device UDIDs in parallel. Assert the phase-1 command contract returns at most one binding per device and never renders a dormant non-default slot; `is_default` remains an internal store column rather than a new public DTO field. If the main grid had selected UDIDs when the panel first opened, seed `Explicit` with every returned account whose `deviceUdid` matches; otherwise seed `AllOnline`. Do this once per panel instance so later grid clicks cannot silently rewrite a campaign draft.

Use a 300 ms link debounce. Tauri invokes are not cancellable, so capture the reducer's `parseRevision` and dispatch results only when it still matches. Build targets from `ParseLinksResult.validTargets`, keeping overrides by stable `targetKey` when that exact key remains present. Map every nonblank `ParsedTargetLine` to a line row; duplicate valid lines remain visible but only one target is submitted.

After a valid local draft change, debounce preview by 250 ms and call canonical `interaction_preview`. Capture a separate `previewRevision`; accept only the newest response. Preview remains provisional and displays backend actor/target/assignment counts, per-target expected actors, typed warnings, and the complete `CapabilitySnapshot.actions` map.

Do not invoke preview when there is a parser error, no valid target, invalid explicit selection, invalid range, invalid probability, invalid schedule, or missing Comment/recipient prerequisite.

- [ ] **Step 4: Bind Setup to the controller**

Pass `AccountBinding[]`, device metadata, line results, targets, preview counts, and typed capability states into the presentational components. The Start slot remains disabled when:

```ts
const canSubmit =
  issues.length === 0 &&
  parseState === "ready" &&
  previewState === "ready" &&
  preview.canStart &&
  targets.length > 0;
```

The backend's `preview.canStart` and warnings are displayed, but only start/schedule persistence is authoritative. Never mutate the draft to hide a backend warning.

- [ ] **Step 5: Verify and commit**

```powershell
npm --prefix apps/desktop test -- useInteractionSetup.test.tsx InteractionSetup.test.tsx interactionApi.test.ts
npm --prefix apps/desktop run build
git add apps/desktop/src/interaction/interactionClient.ts apps/desktop/src/interaction/useInteractionSetup.ts apps/desktop/src/interaction/useInteractionSetup.test.tsx apps/desktop/src/interaction/InteractionSetup.tsx apps/desktop/src/interaction/InteractionSetup.test.tsx
git diff --cached --name-only
git commit -m "feat(desktop): connect interaction setup preview"
```

Expected: delayed parser/preview responses cannot overwrite current state, accounts remain account-ID based, and all display capability states originate in the backend.

---

### Task 6: Integrate One Responsive Tool Panel And Run Or Schedule

**Files:**
- Create: `apps/desktop/src/interaction/InteractionPanel.tsx`
- Create: `apps/desktop/src/interaction/InteractionPanel.test.tsx`
- Modify: `apps/desktop/src/interaction/interaction.css`
- Modify: `apps/desktop/src/components/ProfileToolbar.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/App.css`

- [ ] **Step 1: Write failing toolbar-order and single-panel tests**

Update `ProfileToolbar` tests or create them beside the component:

```tsx
it("places Tương tác immediately after Nuôi TT", () => {
  render(<ProfileToolbar {...toolbarProps({ activeTool: null })} />);
  const buttons = screen.getAllByRole("button").map((button) => button.textContent?.trim());
  const nurture = buttons.findIndex((text) => text === "Nuôi TT");
  expect(buttons[nurture + 1]).toBe("Tương tác");
});

it("models mutually exclusive tool panels", () => {
  let state: ControlTool = null;
  state = toggleControlTool(state, "nurture");
  expect(state).toBe("nurture");
  state = toggleControlTool(state, "interaction");
  expect(state).toBe("interaction");
  state = toggleControlTool(state, "interaction");
  expect(state).toBeNull();
});
```

Run:

```powershell
npm --prefix apps/desktop test -- ProfileToolbar InteractionPanel.test.tsx
```

Expected: FAIL because `activeTool`, `Tương tác`, and `InteractionPanel` are absent.

- [ ] **Step 2: Replace independent booleans with one tool state**

In `App.tsx` define:

```ts
export type ControlTool = "nurture" | "interaction" | null;

export function toggleControlTool(current: ControlTool, requested: Exclude<ControlTool, null>): ControlTool {
  return current === requested ? null : requested;
}
```

Replace `nurtureOpen` with `activeTool`. `ProfileToolbar` receives `activeTool`, `onNurture`, and `onInteraction`; render `Tương tác` immediately after `Nuôi TT` using an existing icon from `Icons.tsx`. Mount `NurturePopup` only for `activeTool === "nurture"` and `InteractionPanel` only for `activeTool === "interaction"`. Do not remount the device grid or start streams when switching tools.

- [ ] **Step 3: Write failing run/schedule submission tests**

```tsx
it("uses one stable request id for an ambiguous Run Now response", async () => {
  const user = userEvent.setup();
  const client = readyClient();
  client.start = vi.fn().mockRejectedValueOnce({ code: "transport_unknown", retryable: true });
  render(<InteractionPanel {...panelProps(client)} />);

  await user.click(screen.getByRole("button", { name: "Chạy ngay" }));
  const firstId = vi.mocked(client.start).mock.calls[0][0].requestId;
  await user.click(screen.getByRole("button", { name: "Thử gửi lại" }));
  const secondId = vi.mocked(client.start).mock.calls[1][0].requestId;

  expect(secondId).toBe(firstId);
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("routes Once only to interaction_schedule", async () => {
  const user = userEvent.setup();
  const client = readyClient();
  render(<InteractionPanel {...panelProps(client, { scheduleMode: "once" })} />);
  await user.click(screen.getByRole("button", { name: "Hẹn một lần" }));
  expect(client.schedule).toHaveBeenCalledTimes(1);
  expect(client.start).not.toHaveBeenCalled();
});
```

Expected first run: submission lifecycle is missing.

- [ ] **Step 4: Implement Setup/Monitor shell and idempotent submission**

`InteractionPanel` has a compact title bar, close icon, and two tabs: `Thiết lập` and `Theo dõi`. Closing only calls `onClose`; it never calls cancel. Keep one generated UUID per submission attempt group. A retry after a transport-unknown response reuses that UUID so G1 idempotency returns the persisted campaign; a validation response permits editing and creates a new UUID only after the normalized draft changes.

For `runNow`, call `client.start`; for `once`, call `client.schedule`. On committed success, retain the campaign ID, show an inline status, and switch to Monitor. Display `InteractionCommandError.message` inline with its typed code; never call `window.alert`, `window.confirm`, or a new global toast from Interaction code.

- [ ] **Step 5: Implement the responsive 720-780 px panel**

Use these stable constraints in `interaction.css`:

```css
.interaction-layer {
  position: fixed;
  inset: 0;
  z-index: 46;
  pointer-events: none;
}

.interaction-panel {
  pointer-events: auto;
  position: absolute;
  right: 16px;
  bottom: 16px;
  width: clamp(720px, 62vw, 780px);
  max-width: calc(100vw - 32px);
  max-height: calc(100vh - 80px);
  display: grid;
  grid-template-rows: 36px 38px minmax(0, 1fr);
  overflow: hidden;
}

@media (max-width: 752px) {
  .interaction-panel {
    left: 12px;
    right: 12px;
    width: auto;
    max-width: none;
  }
}
```

At supported desktop widths of 1024 px and above, the panel remains 720-780 px. It is right-anchored so some of the underlying device canvas stays visible where viewport space permits. At narrow widths it fits without horizontal page overflow; internal two-column Setup sections collapse to one column. Do not use viewport-dependent font sizes or negative letter spacing.

- [ ] **Step 6: Verify and commit**

```powershell
npm --prefix apps/desktop test -- ProfileToolbar InteractionPanel.test.tsx useInteractionSetup.test.tsx
npm --prefix apps/desktop run build
git add apps/desktop/src/interaction/InteractionPanel.tsx apps/desktop/src/interaction/InteractionPanel.test.tsx apps/desktop/src/interaction/interaction.css apps/desktop/src/components/ProfileToolbar.tsx apps/desktop/src/App.tsx apps/desktop/src/App.css
git diff --cached --name-only
git commit -m "feat(desktop): integrate interaction tool panel"
```

Expected: the toolbar order is exact, only one tool panel mounts, Run Now/Once use the right canonical command, and all errors remain inline.

---

### Task 7: Restore Campaigns Into A Paged Monitor

**Files:**
- Create: `apps/desktop/src/interaction/useInteractionMonitor.ts`
- Create: `apps/desktop/src/interaction/VirtualizedRows.tsx`
- Create: `apps/desktop/src/interaction/InteractionMonitor.tsx`
- Create: `apps/desktop/src/interaction/InteractionMonitor.test.tsx`
- Modify: `apps/desktop/src/interaction/InteractionPanel.tsx`
- Modify: `apps/desktop/src/interaction/interaction.css`

- [ ] **Step 1: Write failing restore and revision-event tests**

```tsx
it("restores the active or most recent campaign after panel reopen", async () => {
  const client = monitorClient({
    campaigns: [summary("campaign-running", "running"), summary("campaign-old", "succeeded")],
  });
  const first = renderHook(() => useInteractionMonitor({ client, preferredCampaignId: null }));
  await waitFor(() => expect(first.result.current.campaign?.id).toBe("campaign-running"));
  first.unmount();

  const reopened = renderHook(() => useInteractionMonitor({ client, preferredCampaignId: null }));
  await waitFor(() => expect(reopened.result.current.campaign?.id).toBe("campaign-running"));
  expect(client.list).toHaveBeenCalledWith(null, 50, expect.any(Array));
  expect(client.get).toHaveBeenCalledWith("campaign-running");
});

it("treats InteractionUpdated as an invalidation hint", async () => {
  const client = monitorClient();
  const { result } = renderHook(() => useInteractionMonitor({
    client,
    preferredCampaignId: "campaign-1",
  }));
  await waitFor(() => expect(result.current.campaign?.revision).toBe(3));

  emitInteractionUpdated(client, { summary: summaryAtRevision(5), changedAssignmentIds: ["a-2"] });
  await waitFor(() => expect(client.get).toHaveBeenCalledTimes(2));
  expect(result.current.campaign?.revision).toBe(5);
});
```

Also prove a lower/equal revision is ignored, missed events are recovered by mount reads, event payload detail is never treated as the assignment graph, and listener cleanup runs on unmount.

Run:

```powershell
npm --prefix apps/desktop test -- InteractionMonitor.test.tsx
```

Expected: FAIL because monitor modules do not exist.

- [ ] **Step 2: Implement durable campaign selection and paged caches**

On mount:

1. If `preferredCampaignId` exists, call `interaction_get`.
2. Otherwise list up to 50 campaigns and choose the newest `scheduled`, `queued`, or `running`; if none exists choose the first row.
3. Fetch campaign detail and the first target page with `cursor=null`, `limit=50`.
4. Subscribe to `InteractionUpdated` only after initial reads; if its revision is newer for the active campaign, refetch campaign detail and invalidate affected loaded assignment/action pages.

Store opaque `nextCursor` strings unchanged. `loadMoreTargets`, `loadMoreAssignments`, and `loadMoreActionRuns` append by stable ID, reject concurrent duplicate loads, and stop at `nextCursor=null`. Never calculate offsets or request over 200; G3 uses page size 50.

When a target is selected/expanded, call `interaction_list_assignments` with its target filter. When an assignment is selected, call `interaction_get_assignment` and then `interaction_list_action_runs`. Do not preload every assignment/action for a campaign.

- [ ] **Step 3: Implement fixed-row virtualization**

`VirtualizedRows<T>` accepts `items`, `rowHeight`, `viewportHeight`, `overscan`, `getKey`, `renderRow`, and `onNearEnd`. It renders only the calculated slice plus top/bottom spacers:

```ts
const first = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
const visible = Math.ceil(viewportHeight / rowHeight) + overscan * 2;
const last = Math.min(items.length, first + visible);
```

Use 48 px target rows and 52 px assignment rows. Selecting a row opens one detail band below the virtualized list rather than placing variable-height children inside fixed rows. Trigger `onNearEnd` only when the final rendered index is within 10 rows of the loaded end.

- [ ] **Step 4: Render exhaustive status and evidence detail**

The Monitor summary renders waiting (`scheduled`, `queued`, `waitingCapacity`), running (`preparing`, `session`, `stream`, `opening`, `verifying`, `acting`), succeeded, partial, failed, and skipped (`skippedUnavailable`, `skippedUnsupported`) counts without flattening `cancelled`, `interrupted`, `missed`, or `uncertain`.

Target rows expand into actor assignments. Assignment detail renders every action run, its exact `ActionStatus`, typed outcome/error code, timing, `EvidenceRef`, and the maximum-two Opening-attempt rows nested under an identity action. Each Opening row shows only attempt number, typed status/reason/code, and timestamps; it does not expose URLs or infer retryability. Evidence uses an in-panel anchor to bounded metadata:

```tsx
<a href={`#evidence-${evidence.artifactId}`} className="interaction-evidence-link">
  {evidence.kind}
</a>
<dl id={`evidence-${evidence.artifactId}`} className="interaction-evidence-meta">
  <dt>SHA-256</dt><dd>{evidence.sha256}</dd>
  <dt>Ghi nhận</dt><dd>{formatTimestamp(evidence.observedAt)}</dd>
</dl>
```

Do not construct a filesystem path, read artifact bytes, or place evidence in an event. A purged/missing evidence status remains a typed metadata row.

For assignment `uncertain`, an action with issued-but-unconfirmed identity intent, or any backend retry block reason, render a read-only banner with the exact typed reason. Render `TargetUnverified` as a deterministic failed identity and `TargetIdentityAmbiguous` as an uncertain tap/read outcome; never translate one to the other. Identity action history shows `attemptNo` plus `identityCopyIntent`, and the current-attempt badge comes only from `AssignmentView.currentIdentityAttemptNo/currentIdentityIntent`. Do not offer a retry checkbox for blocked rows.

- [ ] **Step 5: Verify and commit**

```powershell
npm --prefix apps/desktop test -- InteractionMonitor.test.tsx InteractionPanel.test.tsx
npm --prefix apps/desktop run build
git add apps/desktop/src/interaction/useInteractionMonitor.ts apps/desktop/src/interaction/VirtualizedRows.tsx apps/desktop/src/interaction/InteractionMonitor.tsx apps/desktop/src/interaction/InteractionMonitor.test.tsx apps/desktop/src/interaction/InteractionPanel.tsx apps/desktop/src/interaction/interaction.css
git diff --cached --name-only
git commit -m "feat(desktop): restore paged interaction monitor"
```

Expected: panel reopen/restart reads backend state, events only invalidate, cursor pages remain bounded, and uncertain work is read-only.

---

### Task 8: Add Stop, Eligible Retry, And Open On Device Controls

**Files:**
- Modify: `apps/desktop/src/interaction/InteractionMonitor.tsx`
- Modify: `apps/desktop/src/interaction/useInteractionMonitor.ts`
- Modify: `apps/desktop/src/interaction/InteractionMonitor.test.tsx`
- Modify: `apps/desktop/src/interaction/InteractionPanel.tsx`

- [ ] **Step 1: Write failing command-state tests**

```tsx
it("retries only backend-eligible terminal assignments", async () => {
  const user = userEvent.setup();
  const client = monitorClientWithAssignments([
    assignment("failed-eligible", "failed", {
      retryEligible: true,
      identityState: "confirmed",
      currentIdentityAttemptNo: 1,
      currentIdentityIntent: "issued",
    }),
    assignment("failed-blocked", "failed", {
      retryEligible: false,
      retryBlockedReason: "effectIntentIssued",
      identityState: "confirmed",
    }),
    assignment("partial-eligible", "partial", {
      retryEligible: true,
      identityState: "confirmed",
      currentIdentityAttemptNo: 1,
      currentIdentityIntent: "issued",
    }),
    assignment("uncertain", "uncertain", {
      retryEligible: false,
      retryBlockedReason: "targetIdentityAmbiguous",
      resultCode: "targetIdentityAmbiguous",
      currentIdentityIntent: "issued",
    }),
  ]);
  render(<InteractionMonitor {...monitorProps(client)} />);

  await user.click(await screen.findByLabelText("Chọn retry failed-eligible"));
  await user.click(screen.getByLabelText("Chọn retry partial-eligible"));
  expect(screen.getByLabelText("Chọn retry failed-blocked")).toBeDisabled();
  expect(screen.getByLabelText("Chọn retry uncertain")).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Retry Failed" }));

  expect(client.retry).toHaveBeenCalledWith(
    "campaign-1",
    ["failed-eligible", "partial-eligible"],
    expect.any(String),
  );
});

it("shows DeviceBusy inline and does not focus the device", async () => {
  const user = userEvent.setup();
  const onOpenDevice = vi.fn();
  const client = monitorClient();
  client.openOnDevice = vi.fn().mockRejectedValue({
    code: "device_busy",
    message: "Thiết bị đang được Interaction sử dụng",
    retryable: true,
  });
  render(<InteractionMonitor {...monitorProps(client, { onOpenDevice })} />);

  await user.click(screen.getByRole("button", { name: "Mở trên thiết bị" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("device_busy");
  expect(onOpenDevice).not.toHaveBeenCalled();
});
```

Add Stop tests proving the first click reveals an inline confirmation strip and the confirm click calls `interaction_cancel`; closing the panel does not. Add retry-request idempotency tests that reuse the same ID after a transport-unknown response and generate a new one after a committed retry. Add successful Open on Device test that calls `onOpenDevice(result.udid)` only after the Tauri command resolves.

Add a committed-retry refresh test whose next `interaction_get_assignment` returns `currentIdentityAttemptNo=2`, `currentIdentityIntent="none"`, and `identityState="pending"`; its action page retains immutable Confirmed identity attempt 1 and adds Pending attempt 2 before the new effect row. Add blocked tests for issued post-Copy deterministic `TargetUnverified`, `TargetIdentityAmbiguous`, and issued-but-unconfirmed identity, proving none invokes `client.retry`.

Add a separate backend-eligible pre-Copy fixture whose projection has `currentIdentityIntent="none"`, whose latest terminal identity row has `identityCopyIntent="none"`, and which has no prior Confirmed anchor. The checkbox is enabled only because `retryEligible=true`; after commit the refresh shows a new Pending identity attempt and no mutation of the old no-intent row. Pair it with an otherwise identical issued `TargetUnverified` fixture that stays disabled, proving the UI does not decide from the result code alone.

Add a `Partial` campaign fixture with one immutable successful device and one backend-eligible failed/partial assignment. Selecting Retry Failed must send only the eligible assignment ID, refresh the campaign to `queued`, leave the successful device history unchanged, and show the new identity/effect attempts only under the selected assignment.

Run:

```powershell
npm --prefix apps/desktop test -- InteractionMonitor.test.tsx
```

Expected: FAIL because monitor commands are not wired.

- [ ] **Step 2: Implement Stop without a browser modal**

Enable Stop only for `scheduled`, `queued`, or `running`. First click sets `confirmingCancel=true` and renders `Xác nhận dừng` plus `Giữ chạy`; the confirmation calls `client.cancel(campaign.id)`, then refetches from backend. Do not optimistically force terminal state.

- [ ] **Step 3: Implement backend-authorized retry selection**

Checkboxes exist only for loaded assignments whose canonical projection says `retryEligible=true`. Before calling retry, intersect selected IDs with the latest loaded eligible set; reject an empty result inline. Use one `crypto.randomUUID()` per retry attempt group and preserve it across ambiguous transport retries. After a committed response, clear selection and refetch campaign, assignment, and action pages so the new backend-created identity attempt appears before execution.

`Uncertain`, `TargetIdentityAmbiguous`, issued deterministic `TargetUnverified`, issued-but-unconfirmed identity, and any issued side-effect intent remain disabled with their backend `retryBlockedReason`. A Failed/Interrupted optional effect after Confirmed identity or a terminal pre-Copy failure with `currentIdentityIntent="none"` may be enabled only when the backend projection says so. The UI does not reset intent or synthesize attempts; G1 atomically appends the new Pending identity attempt and eligible effect attempts. The UI never changes an identity state or result code.

- [ ] **Step 4: Implement Open on Device and focus handoff**

Call `client.openOnDevice(assignment.id)`. On success pass only the returned UDID to `App.tsx`'s existing `setFocusUdid`; the command has already navigated under `ManualControl`. On typed `device_busy`, `target_changed`, `target_unverified`, or capability error, render code/message inline and leave focus unchanged. Do not fall back to `deviceTap`, `groupInput`, Safari, launch arguments, or a direct driver call.

- [ ] **Step 5: Verify the no-modal boundary and commit**

```powershell
npm --prefix apps/desktop test -- InteractionMonitor.test.tsx InteractionPanel.test.tsx
$paths = Get-ChildItem apps/desktop/src/interaction -Recurse -File
$hits = $paths | Select-String -Pattern 'window\.alert|window\.confirm'
if ($hits) { $hits; exit 1 }
npm --prefix apps/desktop run build
git add apps/desktop/src/interaction/InteractionMonitor.tsx apps/desktop/src/interaction/useInteractionMonitor.ts apps/desktop/src/interaction/InteractionMonitor.test.tsx apps/desktop/src/interaction/InteractionPanel.tsx
git diff --cached --name-only
git commit -m "feat(desktop): control monitored interaction campaigns"
```

Expected: no G3 interaction file uses browser alerts/confirms; Stop, Retry, and Open use typed backend results.

---

### Task 9: Make Proxy Revisions And Manual State Durable

**Files:**
- Create: `crates/core/src/proxy.rs`
- Create: `crates/core/tests/proxy_state.rs`
- Modify: `crates/core/src/interaction/schema.rs`
- Modify: `crates/core/src/interaction/store.rs`
- Modify: `crates/core/src/interaction/types.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/db.rs`
- Modify: `crates/core/src/types.rs`
- Modify: `apps/desktop/src-tauri/src/farm_commands.rs`
- Modify: `apps/desktop/src-tauri/src/state.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/types.ts`
- Modify: `apps/desktop/src/api.ts`

- [ ] **Step 1: Write the failing additive-migration and invalidation tests**

Create an existing database with G1 migration applied, one proxy, and one `device_meta.proxy_id`. Assert migration `2026072902` is additive and backfills a nonblank assignment revision. Then test:

```rust
#[tokio::test]
async fn proxy_or_assignment_edit_invalidates_both_annotations() {
    let fixture = proxy_fixture().await;
    fixture.assign("device-a", Some("proxy-a")).await.unwrap();
    fixture.record_endpoint("device-a", ProxyCheckState::Passed).await.unwrap();
    fixture.confirm_manual("device-a", true).await.unwrap();
    assert!(fixture.state("device-a").await.manually_confirmed);

    fixture.edit_proxy_host("proxy-a", "new.fixture.invalid").await.unwrap();
    let after_proxy_edit = fixture.state("device-a").await;
    assert_eq!(after_proxy_edit.effective.endpoint_check, ProxyCheckState::Invalidated);
    assert!(!after_proxy_edit.effective.manually_confirmed);

    fixture.assign("device-a", Some("proxy-b")).await.unwrap();
    let after_assignment = fixture.state("device-a").await;
    assert_eq!(after_assignment.effective.endpoint_check, ProxyCheckState::Invalidated);
    assert!(!after_assignment.effective.manually_confirmed);
}

#[tokio::test]
async fn manual_confirmation_never_promotes_unsupervised_capability() {
    let fixture = proxy_fixture().await;
    fixture.assign("device-a", Some("proxy-a")).await.unwrap();
    fixture.confirm_manual("device-a", true).await.unwrap();
    let state = fixture.state("device-a").await;
    assert_eq!(state.effective.apply_capability, ProxyApplyCapability::UnsupportedUnsupervised);
    assert_eq!(state.effective.configuration_state, ProxyConfigurationState::ManualRequired);
    assert!(state.effective.manually_confirmed);
}
```

Also test edit of port/username/password, delete/unassign, same-value metadata update, stale endpoint completion after a concurrent revision edit, a reachable fake connector, timeout/refusal failure codes, and serialization/redaction. The serialized `ProxyDeviceState` must not contain host, username, password, authenticated URL, or generic error text.

Run:

```powershell
cargo test -p riviu-core --test proxy_state -- --nocapture
```

Expected: FAIL because migration, annotation store, and service are missing.

- [ ] **Step 2: Add revision-keyed non-secret schema**

Extend the numbered migration system with version `2026072902`:

```sql
ALTER TABLE device_meta ADD COLUMN proxy_assignment_revision TEXT;

CREATE TABLE device_proxy_annotations (
  udid TEXT PRIMARY KEY,
  proxy_id TEXT NOT NULL,
  proxy_configuration_revision TEXT NOT NULL,
  assignment_revision TEXT NOT NULL,
  endpoint_check_state TEXT NOT NULL,
  endpoint_checked_at TEXT,
  endpoint_latency_ms INTEGER,
  endpoint_error_code TEXT,
  manually_confirmed INTEGER NOT NULL DEFAULT 0 CHECK(manually_confirmed IN (0, 1)),
  manually_confirmed_at TEXT
);
```

As with G1's proxy revision migration, inspect columns before `ALTER TABLE` and backfill each existing device row with a generated UUID. Do not hash a password to derive either revision. Annotation rows store only IDs, revisions, typed states/timestamps/latency, and typed error codes.

- [ ] **Step 3: Define one canonical device proxy view**

Reuse G1 `ProxyCheckState`, `ProxyApplyCapability`, `ProxyConfigurationState`, and `EffectiveProxySnapshot`; do not add semantic duplicates. Add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProxyDeviceState {
    pub udid: String,
    pub proxy_name: Option<String>,
    pub assignment_revision: String,
    pub effective: EffectiveProxySnapshot,
    pub endpoint_checked_at: Option<DateTime<Utc>>,
    pub endpoint_latency_ms: Option<u32>,
    pub endpoint_error_code: Option<ProxyEndpointErrorCode>,
    pub manually_confirmed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProxyEndpointErrorCode {
    DnsFailed,
    ConnectionRefused,
    ConnectTimeout,
    NetworkUnavailable,
}
```

For the current fleet, `effective.apply_capability` is always `UnsupportedUnsupervised`, `effective.configuration_state` is `ManualRequired` when assigned, and `effective.manually_confirmed` is an independent annotation. Never produce `AppliedVerified` in G3.

- [ ] **Step 4: Serialize catalog and assignment mutations through G1's writer**

Add `InteractionStore` methods for save/delete catalog entry, assign device proxy, list device states, record endpoint result by exact tuple, and set manual confirmation by exact tuple. Each catalog save rotates `proxies.configuration_revision`; each proxy assignment change rotates `device_meta.proxy_assignment_revision`. Reads accept an annotation only when proxy ID, proxy revision, and assignment revision all match. A mismatch projects `ProxyCheckState::Invalidated` and `manually_confirmed=false`.

Update the existing low-level `Database::upsert_device_meta` defense so any caller that changes `proxy_id` performs revision rotation and annotation invalidation in the same SQLite transaction. Update delete to unassign affected devices and rotate their assignment revisions transactionally. No database transaction spans a network await.

- [ ] **Step 5: Implement bounded desktop endpoint reachability**

Create an injected seam:

```rust
#[async_trait::async_trait]
pub trait ProxyEndpointConnector: Send + Sync {
    async fn connect(
        &self,
        host: &str,
        port: u16,
        deadline: std::time::Duration,
    ) -> Result<u32, ProxyEndpointErrorCode>;
}
```

The production connector resolves and attempts one TCP connection with a fixed 5-second total deadline, then returns latency milliseconds. It does not send HTTP, SOCKS authentication, credentials, or an external IP request. `ProxyService::check_endpoint(udid)` loads the private current proxy tuple, connects outside SQLite, and commits Passed/Failed only if the tuple is still current; a revision race returns typed `stale_proxy_revision` and stores nothing.

Tracing includes only operation, typed outcome, latency, and a redacted proxy ID hash. It never includes host, username, password, URL, raw socket error, or UDID.

- [ ] **Step 6: Expose typed proxy commands and wrappers**

Keep existing `list_proxies`, `save_proxy`, `delete_proxy`, and `export_proxy_config` compatibility. Add and register:

```text
list_proxy_device_states(udids)
assign_device_proxy(udid, proxy_id)
check_proxy_endpoint(udid)
confirm_device_proxy(udid, confirmed)
```

The new commands return only `ProxyDeviceState` or `Vec<ProxyDeviceState>`. `proxy_id=null` unassigns. Export remains an explicit credential-bearing return to the local operator and is never emitted as an event/log/evidence. Endpoint errors use a stable `{ code, message, retryable }` Tauri error without source-chain formatting.

Mirror exact camel-case types/wrappers in `types.ts` and `api.ts`.

- [ ] **Step 7: Verify and commit**

```powershell
cargo test -p riviu-core --test proxy_state -- --nocapture
cargo test -p riviu-managers-phone farm_commands -- --nocapture
cargo check -p riviu-managers-phone
npm --prefix apps/desktop run build
git add crates/core/src/proxy.rs crates/core/tests/proxy_state.rs crates/core/src/interaction/schema.rs crates/core/src/interaction/store.rs crates/core/src/interaction/types.rs crates/core/src/lib.rs crates/core/src/db.rs crates/core/src/types.rs apps/desktop/src-tauri/src/farm_commands.rs apps/desktop/src-tauri/src/state.rs apps/desktop/src-tauri/src/lib.rs apps/desktop/src/types.ts apps/desktop/src/api.ts
git diff --cached --name-only
git commit -m "feat(core): track manual proxy assignment state"
```

Expected: revision edits invalidate endpoint/manual annotations, the current fleet remains `unsupportedUnsupervised/manualRequired`, and no new state DTO carries a secret.

---

### Task 10: Restore Proxy Under System Navigation

**Files:**
- Create: `apps/desktop/src/pages/ProxyPage.tsx`
- Create: `apps/desktop/src/pages/ProxyPage.test.tsx`
- Modify: `apps/desktop/src/pages/FarmPages.tsx`
- Modify: `apps/desktop/src/components/Sidebar.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/App.css`

- [ ] **Step 1: Write failing navigation and state-separation tests**

```tsx
it("lists Proxy under Hệ thống", () => {
  render(<Sidebar {...sidebarProps()} />);
  const system = screen.getByText("Hệ thống").closest(".menu-group")!;
  expect(within(system).getByRole("button", { name: "Proxy" })).toBeTruthy();
});

it("keeps desktop reachability separate from manual iPhone confirmation", async () => {
  const user = userEvent.setup();
  const client = proxyClient({
    endpointCheck: "passed",
    manuallyConfirmed: false,
    applyCapability: "unsupportedUnsupervised",
    configurationState: "manualRequired",
  });
  render(<ProxyPage devices={fixtureDevices()} client={client} />);

  expect(await screen.findByText("Desktop: Có thể kết nối endpoint")).toBeTruthy();
  expect(screen.getByText("iPhone: Cần cấu hình thủ công")).toBeTruthy();
  await user.click(screen.getByRole("checkbox", { name: "Đã cấu hình thủ công trên iPhone" }));
  expect(client.confirmDevice).toHaveBeenCalledWith("fixture-udid", true);
  expect(screen.queryByText(/device IP verified/i)).toBeNull();
  expect(screen.queryByText(/^applied$/i)).toBeNull();
});
```

Add CRUD/edit/delete tests, explicit Export to clipboard, device assignment, endpoint Unchecked/Passed/Failed/Invalidated labels, revision invalidation after edit and reassignment, password input masking, actor/default proxy read-only display, inline errors, and no secret in emitted notice text.

Run:

```powershell
npm --prefix apps/desktop test -- ProxyPage.test.tsx
```

Expected: FAIL because Proxy is absent from System navigation and the focused page does not exist.

- [ ] **Step 2: Extract the existing Proxy page without changing credential compatibility**

Move `ProxyPage` out of `FarmPages.tsx` into the focused file. Preserve:

- create/edit fields for name, type, host, port, username, password, notes;
- local password storage semantics already used by `ProxyConfig`;
- explicit Export that copies the existing backend text to clipboard;
- delete behavior.

Use `type="password"` and `autoComplete="new-password"`. Replace the old `flash()`/browser alert calls with `InlineNotice`. Never include the password in a confirmation or error message.

- [ ] **Step 3: Add device assignment and two independent state columns**

Pass `devices` from `App.tsx`. Load `listProxyDeviceStates(devices.map(d => d.udid))`. Render one fixed row per device/account default with:

- device/account identity;
- a proxy select whose value writes `device_meta.proxy_id` through `assign_device_proxy`;
- `Desktop: Chưa kiểm tra | Có thể kết nối endpoint | Không thể kết nối endpoint | Cấu hình đã đổi, cần kiểm tra lại`;
- `iPhone: Cần cấu hình thủ công | Đã xác nhận cấu hình thủ công`;
- endpoint test button;
- manual-confirmation checkbox.

The row may display the inherited device proxy, but it cannot edit a separate account-level proxy. Manual confirmation remains a Boolean annotation; it does not change the typed capability/configuration state.

Place a persistent restrained notice above the rows: `Kiểm tra endpoint chạy từ máy tính; không xác minh IP hoặc lưu lượng của iPhone.` Do not display applied, verified-device-IP, MDM, supervision, or automatic-system-proxy claims.

- [ ] **Step 4: Restore navigation and routing**

Add `{ id: "proxy", label: "Proxy" }` under the `system` group's children in `Sidebar.tsx`. Import `ProxyPage` directly in `App.tsx`, add `proxy: "Proxy"` to `PAGE_TITLE`, and render `<ProxyPage devices={devices} />` for `page === "proxy"`.

Do not add a new top-level marketing page or hide the existing Account/API/Settings entries.

- [ ] **Step 5: Verify truthful wording and commit**

```powershell
npm --prefix apps/desktop test -- ProxyPage.test.tsx
$ui = Get-Content apps/desktop/src/pages/ProxyPage.tsx -Raw
if ($ui -match '(?i)device IP verified|\bapplied\b') { exit 1 }
npm --prefix apps/desktop run build
git add apps/desktop/src/pages/ProxyPage.tsx apps/desktop/src/pages/ProxyPage.test.tsx apps/desktop/src/pages/FarmPages.tsx apps/desktop/src/components/Sidebar.tsx apps/desktop/src/App.tsx apps/desktop/src/App.css
git diff --cached --name-only
git commit -m "feat(desktop): restore truthful proxy management"
```

Expected: Proxy appears under System; CRUD/export/assignment work; reachability and manual confirmation remain visibly separate.

---

### Task 11: Preserve Honest Parked Device Tiles Behind The Panel

**Files:**
- Modify: `apps/desktop/src/frameStore.ts`
- Modify: `apps/desktop/src/components/DeviceTile.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Modify: `apps/desktop/src/App.css`
- Create: `apps/desktop/src/components/DeviceTile.test.tsx`

- [ ] **Step 1: Write failing frame-age and parked-state tests**

Build on G0's canonical `TileStreamState` rather than defining another stream status:

```tsx
it("shows a parked last frame with its receive time", () => {
  pushFrame("device-a", "jpeg-base64", "2026-07-29T10:00:00Z");
  render(<DeviceTile {...tileProps({ tileStreamState: "parked" })} />);
  expect(screen.getByAltText("iPhone A")).toBeTruthy();
  expect(screen.getByText("Đã tạm dừng")).toBeTruthy();
  expect(screen.getByText(/Khung cuối/)).toBeTruthy();
});

it("does not imply a producer before the first sampled frame", () => {
  render(<DeviceTile {...tileProps({ tileStreamState: "parked" })} />);
  expect(screen.getByText("Đang chờ khung hình đầu tiên")).toBeTruthy();
  expect(screen.queryByRole("progressbar")).toBeNull();
  expect(screen.queryByText("Live")).toBeNull();
});
```

Also test `live`, `sampling`, `stale`, and `error`; a parked/stale frame stays aspect-ratio constrained and cannot receive tap/swipe events as though it were live.

Run:

```powershell
npm --prefix apps/desktop test -- DeviceTile.test.tsx
```

Expected: FAIL because frame timestamps/honest parked rendering are incomplete.

- [ ] **Step 2: Retain frame metadata without reopening streams**

Change `frameStore` values from a raw string to:

```ts
export interface DeviceFrameRecord {
  jpegBase64: string;
  receivedAt: string;
}
```

`pushFrame` accepts the event timestamp when supplied and otherwise records `new Date().toISOString()`. Preserve identical-JPEG suppression without overwriting the original `receivedAt` on duplicate events. Expose `useDeviceFrame` and `peekFrame` as records, then update their current consumers.

This is display metadata only. It must not start/restart MJPEG, modify G0 budget state, or claim a device frame was captured at server time when only receive time exists.

- [ ] **Step 3: Render honest overlays and disable stale gestures**

For G0 `tileStreamState`:

- `live`: enable gestures and show the ordinary live state;
- `sampling`: show a compact sampling label, with no indeterminate spinner that suggests every tile has a producer;
- `parked`: retain the last image, overlay `Đã tạm dừng`, and show localized last-frame receive time;
- `stale`: retain the image, overlay `Khung hình cũ`, and disable gestures;
- `error`: retain a prior image only as stale evidence, display the typed error, and disable gestures.

If no record exists, render `Đang chờ khung hình đầu tiên`. Do not show a stale image as live and do not call `onPrepare` automatically from render/effect.

- [ ] **Step 4: Verify and commit**

```powershell
npm --prefix apps/desktop test -- DeviceTile.test.tsx InteractionPanel.test.tsx
npm --prefix apps/desktop run build
git add apps/desktop/src/frameStore.ts apps/desktop/src/components/DeviceTile.tsx apps/desktop/src/components/DeviceTile.test.tsx apps/desktop/src/App.tsx apps/desktop/src/App.css
git diff --cached --name-only
git commit -m "fix(desktop): label parked device frames honestly"
```

Expected: the device canvas remains usable/visible behind G3, while parked and unsampled tiles never imply an active producer.

---

### Task 12: Run Visual, Integration, Redaction, And Handoff Gates

**Files:**
- Modify: `apps/desktop/package.json`
- Modify: `apps/desktop/package-lock.json`
- Create: `apps/desktop/playwright.config.ts`
- Create: `apps/desktop/interaction-harness.html`
- Create: `apps/desktop/src/test/interactionHarnessMain.tsx`
- Create: `apps/desktop/e2e/interaction-workflow.spec.ts`
- Modify: `AGENTS.md`

- [ ] **Step 1: Install Playwright and create a deterministic browser harness**

```powershell
npm --prefix apps/desktop install --save-dev @playwright/test
Push-Location apps/desktop
npx playwright install chromium
Pop-Location
```

Create `interaction-harness.html` with a root element and module import of `src/test/interactionHarnessMain.tsx`. The harness renders the real `InteractionPanel`, an underlying fixed device grid, and the real `ProxyPage` using injected in-memory clients. It provides buttons to switch `Setup`, `Monitor`, and `Proxy`. Fixtures use typed G1/G2 DTOs, G2 Ready for Watch/Like/Follow/Comment, and `Deferred/GateNotQualified` for Save/Repost/DirectMessage.

No harness branch is imported by `main.tsx`, included in the Tauri route, or selected by a production environment variable.

- [ ] **Step 2: Write fixed viewport and containment tests**

Configure Playwright web server as `npm run dev -- --host 127.0.0.1 --port 1421` and base URL `http://127.0.0.1:1421/interaction-harness.html`.

```ts
test("Setup and Monitor fit the approved desktop panel", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/interaction-harness.html");
  const panel = page.locator(".interaction-panel");
  const box = await panel.boundingBox();
  expect(box!.width).toBeGreaterThanOrEqual(720);
  expect(box!.width).toBeLessThanOrEqual(780);
  await expect(page.locator(".fixture-device-grid")).toBeVisible();
  expect(await panel.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);

  await page.getByRole("tab", { name: "Theo dõi" }).click();
  expect(await panel.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
});

test("narrow responsive mode contains every control", async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 900 });
  await page.goto("/interaction-harness.html");
  const panel = page.locator(".interaction-panel");
  const box = await panel.boundingBox();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(720);
  expect(await panel.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
});
```

Add Proxy at 1024x768, both Setup/Monitor at 1280x800, longest typed error strings, 500-target paged fixture, zero-device state, Direct Message disabled reason, expanded override, and parked tile overlay. Check no text/button overflows its box and no control overlays another control. Save diagnostic screenshots under ignored `apps/desktop/test-results/`; do not commit generated screenshots.

- [ ] **Step 3: Run the focused browser gate**

```powershell
Push-Location apps/desktop
npx playwright test e2e/interaction-workflow.spec.ts --project=chromium
Pop-Location
```

Expected: all viewport/containment tests pass; canvas remains visible at wide desktop; 720 px responsive mode stays within viewport; Setup, Monitor, and Proxy have no incoherent overlaps.

- [ ] **Step 4: Run full Rust and desktop regression gates**

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
npm --prefix apps/desktop test -- --run
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run build
```

Expected: all G0/G1/G2 regressions and every G3 test pass. Existing Nurture behavior remains unchanged.

- [ ] **Step 5: Verify secrets, modal boundary, and production artifacts**

```powershell
$g3 = Get-ChildItem apps/desktop/src/interaction,apps/desktop/src/pages/ProxyPage.tsx -Recurse -File
$modalHits = $g3 | Select-String -Pattern 'window\.alert|window\.confirm'
if ($modalHits) { $modalHits; exit 1 }

$gate2Reports = @(
  'docs/re/interaction-gate2/gate-2.json',
  'docs/re/interaction-gate2/gate-2.md'
) | Where-Object { Test-Path $_ }
if ($gate2Reports.Count -gt 0) {
  $args = @('run', '-q', '-p', 'rtmmo-re', '--', 'verify-redaction')
  foreach ($report in $gate2Reports) { $args += @('--input', $report) }
  & cargo @args
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

$ipa = (Get-FileHash sidecars/wda/RiviuAgent.ipa -Algorithm SHA256).Hash.ToLowerInvariant()
$manifest = (Get-FileHash sidecars/wda/agent-manifest.json -Algorithm SHA256).Hash.ToLowerInvariant()
if ($ipa -ne '8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea') { exit 1 }
if ($manifest -ne 'e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a') { exit 1 }
```

Expected: no G3 flow uses browser modals, redaction succeeds, and both production rollback artifacts remain byte-identical.

- [ ] **Step 6: Record the G3 handoff in `AGENTS.md`**

Add a dated checkpoint containing:

- G3 commit and exact test counts/commands;
- Setup/Monitor panel behavior and backend-restore rule;
- `interaction_open_on_device` uses `DeviceWorkOwner::ManualControl` and typed `DeviceBusy`;
- Proxy is CRUD/export/assignment plus desktop endpoint reachability and manual confirmation only;
- current proxy capability remains `unsupported_unsupervised/manual_required`;
- Save, Repost, and Direct Message remain `Deferred/GateNotQualified` until `docs/archive/plans/2026-07-29-tiktok-interaction-new-action-gates.md` completes;
- production IPA/manifest checksums above remain unchanged;
- rollback is the G3 commit range only; additive proxy annotations/history may remain.

- [ ] **Step 7: Commit the visual gate and handoff**

```powershell
git add apps/desktop/package.json apps/desktop/package-lock.json apps/desktop/playwright.config.ts apps/desktop/interaction-harness.html apps/desktop/src/test/interactionHarnessMain.tsx apps/desktop/e2e/interaction-workflow.spec.ts
git add -p AGENTS.md
git diff --cached --name-only
git commit -m "test(desktop): gate interaction workflow UX"
```

Expected staged paths: exactly the six desktop paths above plus only the reviewed G3 hunk from `AGENTS.md`.

---

## Gate G3 Acceptance Checklist

- [ ] `Tương tác` is immediately after `Nuôi TT`; only Nurture or Interaction is mounted.
- [ ] Panel width is 720-780 px on supported desktop viewports and narrows without overflow at 720 px.
- [ ] Device tiles remain mounted; parked frames show `Đã tạm dừng` plus last-frame time, and no-frame tiles show an explicit waiting state.
- [ ] URL parsing is backend-only, per-line, deduplicated by backend result, and stale-response safe.
- [ ] Actor modes are `AllOnline` and `Explicit`; distribution modes are `All` and `RoundRobin`.
- [ ] Per-target absent overrides serialize as field omission and render `Inherit`.
- [ ] Copy Link verification is fixed; Watch, pacing, optional policy, AI Comment, fallback pool, and recipient policy controls match G1 contracts.
- [ ] Direct Message recipient UI is generic but production capability remains typed Deferred before G4.
- [ ] Preview is provisional; start/schedule re-resolve and persist transactionally through G1.
- [ ] Monitor restores from SQLite after close/reopen/restart and uses revision events only as invalidation hints.
- [ ] Target, assignment, action, and evidence detail are cursor-backed, bounded to 50 per page, and virtualized for large lists.
- [ ] Stop is inline-confirmed; Retry uses backend eligibility, displays the new append-only identity attempt after commit, and keeps Uncertain/issued-unconfirmed identity work read-only.
- [ ] Open on Device acquires `ManualControl`, returns typed `DeviceBusy`, and never bypasses G0.
- [ ] No G3 runtime error uses `window.alert` or a new browser modal.
- [ ] Proxy is under System with CRUD/export/device assignment.
- [ ] Desktop endpoint reachability and manual iPhone confirmation are independent revision-keyed facts.
- [ ] No G3 label claims current unsupervised devices have an applied proxy or verified device IP.
- [ ] Proxy passwords stay out of campaign snapshots, events, evidence, traces, and errors.
- [ ] G0/G1/G2/full workspace tests, desktop build/lint, Playwright viewports, redaction, and rollback checksum gates pass.

## Plan Self-Check

Run these checks against this plan before execution handoff:

```powershell
$plan = 'docs/archive/plans/2026-07-29-tiktok-interaction-desktop-workflow.md'
$text = Get-Content $plan -Raw
$fences = ([regex]::Matches($text, '(?m)^```')).Count
if (($fences % 2) -ne 0) { throw 'Unbalanced code fences' }

$forbidden = @('T' + 'ODO', 'T' + 'BD', 'fill' + ' in details', 'similar' + ' to Task')
foreach ($term in $forbidden) {
  if ($text.Contains($term)) { throw "Forbidden pending marker: $term" }
}

$required = @(
  'InteractionCampaignRequest', 'TikTokTargetInput', 'InteractionOverrides',
  'InteractionDefaults', 'ActionPolicy', 'RecipientPolicy', 'AccountBinding',
  'ActorSelection', 'DistributionMode', 'ScheduleMode', 'CampaignStatus',
  'AssignmentStatus', 'CapabilitySnapshot', 'CapabilityState',
  'DeviceControlPlane', 'DeviceWorkOwner::ManualControl', 'DeviceBusy',
  'interaction_parse_links', 'interaction_preview', 'interaction_start',
  'interaction_schedule', 'interaction_open_on_device'
)
foreach ($name in $required) {
  if (-not $text.Contains($name)) { throw "Missing canonical contract: $name" }
}

$selfCheckAt = $text.IndexOf("## Plan Self-Check")
$planBody = if ($selfCheckAt -ge 0) { $text.Substring(0, $selfCheckAt) } else { $text }
$tasks = ([regex]::Matches($planBody, '(?m)^### Task \d+:')).Count
$commits = ([regex]::Matches($planBody, '(?m)^git commit -m')).Count
if ($tasks -ne 12 -or $commits -ne 12) {
  throw "Expected 12 tasks and 12 task commits; got $tasks tasks and $commits commits"
}
```

Expected: balanced fences, no pending markers, every canonical G0/G1 name present, and exactly 12 independently committable tasks.
