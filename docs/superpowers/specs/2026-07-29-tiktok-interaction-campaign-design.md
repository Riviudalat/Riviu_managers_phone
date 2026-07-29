# TikTok Interaction Campaign Design

**Status:** Approved in conversation on 29/07/2026

## 1. Context

Riviu currently has a live-tested TikTok nurture flow and a desktop device grid,
but it does not have a target-driven workflow for a list of TikTok URLs. The new
`Tương tác` workflow must let an operator paste direct or short TikTok links,
choose devices, and run verified actions against the intended posts.

The current fleet model is one TikTok account per iPhone. The design must preserve
that simple operating model while reserving stable account identifiers for a future
multi-account-per-device implementation. The current fleet remains unsupervised;
this project must not add MDM or silently claim that a proxy was applied to iOS.

## 2. Goals

- Add a `Tương tác` control next to `Nuôi TT` on the device-control page.
- Accept multiple TikTok URL forms, normalize them, resolve short links, and
  deduplicate targets by content identity.
- Support both execution distributions:
  - `Tất cả`: every eligible actor processes every target.
  - `Phân bổ`: targets are assigned round-robin across eligible actors.
- Support run-now and one-time scheduled campaigns.
- Let campaign defaults be overridden per target.
- Give every action an `Off`, `Required`, or `Probability(percent)` policy.
- Support watch, like, follow, comment, save, copy link, repost, and direct share.
- Generate comments from video context and per-target instructions, with a fixed
  fallback pool.
- Support direct-share recipients by allowlist or by random visible recipient.
- Persist per-target and per-actor progress, evidence, errors, and retry state.
- Continue other assignments when one assignment fails.
- Keep a minimal extension point for a future A-comments/B-replies workflow.
- Restore proxy CRUD and actor assignment without adding unavailable iOS policy
  controls.

## 3. Non-Goals And Deferred Scope

The following are outside this project:

- Multiple active TikTok accounts on one device and automatic account switching.
- A generic workflow editor or a complete dependency DAG.
- Global HTTP Proxy, Always On VPN, MDM enrollment, silent managed-app control,
  kiosk/restriction policy, OS update policy, Lost Mode, remote wipe, remote
  restart/shutdown, and Activation Lock management.
- Reading TikTok application storage, passwords, keychain data, or the iOS full
  filesystem.
- Silent Safari or launch-argument fallback when the Agent cannot open a URL.
- Advertising an action as supported before its contract and live evidence pass.

The existing `AdminControl` capability boundary remains reserved for a later MDM
phase. No MDM menu, placeholder command, or dependency is added now.

## 4. Chosen Architecture

Create a dedicated `InteractionCampaignEngine` in `riviu-core`. Do not extend
`NurtureEngine` with a target-link mode, and do not encode campaigns as generic
`ScriptAction` sequences. A target campaign has different persistence, scheduling,
evidence, retry, account, and dependency semantics.

### 4.1 Components

`TikTokLinkParser`
: Parses URLs with `url::Url`, recognizes supported TikTok paths, strips irrelevant
  tracking data after identity is known, and returns typed per-line errors.

`TikTokLinkResolver`
: Resolves short URLs through an injectable HTTP transport. It validates every
  redirect hop and never follows a redirect to a non-TikTok host.

`InteractionPlanner`
: Pure logic invoked at execution time after actor availability is resolved. It
  snapshots the eligible actors and targets, expands the selected distribution,
  merges defaults with per-target overrides, samples probabilities once, and emits
  immutable assignments. `interaction_preview` uses the same logic without
  persisting or reserving its provisional result.

`DeviceWorkCoordinator`
: Owns cross-engine reservations for Nurture, Interaction, Script, and lifecycle
  operations. A supervisor lock around one driver call is not sufficient because
  two engines could otherwise alternate gestures on the same screen.

`InteractionScheduler`
: Starts immediate or one-time campaigns, groups runnable work by device, enforces
  device reservations, and limits USB-heavy work through one global semaphore.

`TikTokActionExecutor`
: A narrow facade for verified TikTok actions. It initially delegates to the
  live-tested nurture action helpers. Extraction must be incremental and protected
  by parity tests; the nurture behavior must not be rewritten as a prerequisite.

`InteractionStore`
: Persists campaign configuration, immutable snapshots, assignments, action runs,
  artifacts, schedules, and state transitions in SQLite.

### 4.2 Why Existing JobQueue Is Not Reused

`JobQueue` owns an internal per-UDID semaphore and stores generic script steps. Its
lock is not shared with Nurture, and its data model cannot represent account-target
assignments, per-action evidence, ambiguous side effects, or future dependencies.
The new coordinator is shared infrastructure; Interaction records remain separate.

### 4.3 USB Concurrency

The current machine has live-qualified only one or two concurrent MJPEG/control
pairs. The scheduler therefore uses:

- one device reservation per UDID;
- a global USB-heavy semaphore defaulting to `1`;
- a hard configurable maximum of `2` until a new fleet test raises the limit.

`Tất cả` means every selected actor eventually runs; it does not mean that all
devices stream simultaneously. Devices reserved by a campaign may wait for a USB
slot without being classified as busy.

## 5. Domain Model

The exact implementation names may follow Rust naming conventions, but these
fields and semantics are required.

```rust
struct AccountBinding {
    id: AccountId,
    platform: Platform,          // TikTok in this project
    device_udid: String,
    slot_key: String,            // "default" today
    username: Option<String>,
    proxy_id: Option<String>,
    state: AccountState,
}

struct TikTokTarget {
    original_url: String,
    normalized_url: String,
    resolved_url: String,
    target_key: String,
    content_id: Option<String>,
    author: Option<String>,
    kind: TikTokTargetKind,       // Video, Photo, ShortUnknown
    overrides: InteractionOverrides,
}

enum ActionPolicy {
    Off,
    Required,
    Probability { percent: u8 },
}

enum ShareMethod {
    CopyLink,
    Repost,
    DirectMessage(RecipientPolicy),
}

enum DistributionMode {
    All,
    RoundRobin,
}
```

Each current device produces one stable actor with an identity equivalent to
`device:<udid>:default`. Assignment history stores both `account_id` and a UDID
snapshot so a future account rebind does not rewrite history.

The only dependency field required now is a nullable `parent_assignment_id` plus
typed output artifacts. A successful comment may emit:

```rust
struct CommentArtifact {
    target_key: String,
    account_id: AccountId,
    author: Option<String>,
    normalized_text: String,
    screenshot_path: String,
}
```

This is sufficient for a later reply workflow without implementing a generic DAG.

## 6. Link Parsing And Resolution

### 6.1 Accepted Inputs

Only HTTPS URLs whose host is exactly `tiktok.com` or ends in `.tiktok.com` are
accepted. Userinfo and custom ports are rejected. Supported forms include:

- `/@<handle>/video/<content-id>`
- `/@<handle>/photo/<content-id>`
- `vt.tiktok.com/<code>`
- `vm.tiktok.com/<code>`
- `/t/<code>`

Blank lines are ignored. Syntax errors, unsupported hosts, unsupported paths, and
resolution failures are reported against their original line. A campaign can use
the valid lines; it cannot start when no valid target remains.

### 6.2 Redirect Rules

The resolver disables automatic redirect following and handles each hop itself:

- validate scheme, host, userinfo, and port on every hop;
- cap redirect count and total deadline;
- reject loops;
- retain no response body beyond what resolution requires;
- strip fragment and known tracking query parameters after the direct target is
  identified.

Targets are deduplicated by `content:<content-id>`. Until a content ID is resolved,
the normalized short URL is the preview-only fallback key. Starting or scheduling
a campaign requires every accepted target to resolve to a content ID; unresolved
short URLs remain visible as per-line errors and are excluded from the runnable set.

Both frontend preview and backend start validation use the same backend parser.
The backend revalidates all inputs when starting or scheduling a campaign.

## 7. Actor Selection And Distribution

The operator selects one target mode:

`Tất cả máy online`
: Resolve all eligible default actors at the actual execution time, then snapshot
  them. A device joining after execution starts is not added.

`Chỉ định`
: Snapshot the explicitly selected actor IDs when the immediate run or schedule is
  created.

At execution time, the coordinator attempts to reserve every selected actor before
the planner expands targets. An actor already reserved by Nurture, Script, Repair,
or another campaign is recorded as `SkippedUnavailable`. An offline actor is handled
the same way. The scheduler does not wait ten minutes and does not retry availability.
Round-robin planning uses only the actors successfully reserved at this point, so a
target is not knowingly assigned to an actor that was already unavailable. If a
reserved actor disconnects later, its assignments are skipped and are not silently
redistributed because another actor may already have started related work.

Distribution then works as follows:

- `All`: deterministic Cartesian product of actor order and target order.
- `RoundRobin`: target at index `i` is assigned to actor
  `i % actor_count`, preserving request order.

If no eligible actor remains at execution time, the campaign becomes
`FailedNoEligibleActors`. Other per-actor skips make an otherwise runnable campaign
`Partial`.

## 8. Configuration And Probability

Campaign defaults contain:

- minimum and maximum watch duration;
- Watch, Like, Follow, Comment, Save, Copy Link, Repost, and Direct Message policies;
- AI comment instruction;
- fixed comment fallback pool;
- direct-share recipient mode and allowlist;
- pacing and jitter within bounded product defaults.

Each target stores an override patch. An absent field means `Inherit`, not `Off`.
The effective target configuration is computed once during planning and stored in
the assignment snapshot.

For every `Probability` policy, the planner samples the decision once and persists
the result. A retry, process restart, or UI reopen must not roll the probability
again. Required actions are always selected; Off actions are never scheduled.

The planner also persists a seed and final action plan for auditability. Retry only
reconsiders actions that were selected in the original plan.

## 9. Execution Lifecycle

The logical lifecycle for each actor-target assignment is:

```text
device reservation
-> Agent/account preflight
-> foreground TikTok
-> create the required UI session
-> start MJPEG and receive the first frame
-> open the target URL
-> verify the target identity
-> watch for the planned duration
-> execute planned actions
-> persist evidence and final state
-> close transient UI
-> release resources when the device batch is complete
```

The session must always exist before MJPEG starts. If any assignment in a device
batch requires Comment or recipient search text, use the fresh text-session path
after TikTok is foreground. Otherwise use an ordinary session. A healthy session
and stream may be reused for consecutive targets on the same actor, but no invariant
or evidence requirement may be skipped.

The popup watcher remains active. While the executor owns a comment or share drawer,
it uses `run_suppressible()` so the watcher continues classifying without tapping
the controlled UI.

## 10. Open URL Capability And Target Identity

Opening a link is the first delivery blocker. The production RT-MMO inventory has
an oracle-only `/url` candidate, while baseline WDA exposes a session route. Static
inventory does not prove method, authentication, body, session, or runtime behavior.

Add a typed `UiSession::open_url` boundary with a profile-specific adapter. Do not
silently fall back to Safari UI or launch arguments. `openUrl` is advertised only
after a live contract probe proves:

- protected authentication is enforced;
- the expected endpoint and request schema work;
- TikTok becomes foreground;
- a direct link, short video link, and photo link open successfully;
- the session and stream remain healthy;
- the opened content can be identified.

Project 2 candidate protocol v2 and the current production IPA/manifest remain
unchanged until their existing Mac gates pass. Interaction work must not weaken or
rewrite those attestations.

Before the live gate, the driver reports `openUrl=unsupported`. After approval, a
versioned driver capability entry is keyed to the exact Agent artifact SHA-256 and
the probe evidence. A different artifact hash does not inherit that capability.
A future Agent protocol may publish `openUrl` in protected health, but this project
does not edit the current production manifest to manufacture that claim.

### 10.1 Identity Gate

A changed frame and visible TikTok rail prove only that TikTok is displaying some
content. They do not prove it is the requested content. No Like, Follow, Comment,
Save, Repost, or Direct Message may run before `targetIdentityConfirmed`.

The primary identity proof is:

```text
open target
-> open Share
-> Copy link
-> read clipboard through the Agent
-> normalize copied URL
-> compare content ID with planned target
-> close Share
```

The identity probe is mandatory even when the operator sets the reporting action
`Copy Link` to Off; it changes the clipboard but creates no TikTok engagement. If
Copy Link was selected, the same verified operation also satisfies that action and
is not repeated. Clipboard/open-link behavior while TikTok is foreground is part of
the live gate. A mismatch or missing identity is `TargetUnverified`, and the
remaining side effects for that assignment are skipped.

## 11. Action Semantics And Evidence

An HTTP success response or gesture acknowledgement is never sufficient evidence.

`Watch`
: Requires the target-ready state to remain observable and elapsed time to meet the
  planned duration. Frame digests and timestamps are stored; continuous screenshots
  are not required.

`Like`
: Uses the existing action-rail detector. Redness must cross the verified threshold.
  A target already liked is `AlreadySatisfied`.

`Follow`
: Requires the Follow badge to disappear. An already-followed author is
  `AlreadySatisfied`.

`Comment`
: Generate one comment from the stable target frame plus campaign and per-target
  instructions. Persist the generated text before typing. If generation fails, pick
  one deterministic entry from the fallback pool. The Send control must arm before
  tap and disarm back to the open drawer afterward.

`Save`
: Requires a fixture-backed bookmark-state transition. Already saved is
  `AlreadySatisfied`.

`CopyLink`
: Requires clipboard read-back containing the same normalized content ID.

`Repost`
: Must distinguish `Repost` from `Remove repost`. If the post-tap state is ambiguous,
  record `Uncertain` and do not tap again. Seeing the verified `Remove repost` state
  before tapping means `AlreadySatisfied`.

`DirectMessage`
: Supports two selectable modes. Allowlist mode stores a normalized handle plus an
  optional display label and locates that recipient. Random-visible mode selects a
  currently visible eligible recipient. Sending requires a post-send state or toast
  and captures the resolved recipient evidence. Text search requires a fresh text
  session.

The default action order is target verification, Watch, Like, Follow, Comment, Save,
and then the remaining Share methods. Share UI is closed between operations unless
the current live contract proves that reuse is stable.

## 12. Retry, Cancellation, And Recovery

Action states are:

```text
Pending -> Running -> Succeeded | AlreadySatisfied | NotConfirmed |
                      Uncertain | Failed | Skipped | Interrupted
```

Assignment states include:

```text
Queued -> Preparing -> Session -> Stream -> Opening -> Verifying -> Acting
       -> Succeeded | Partial | Failed | SkippedUnavailable |
          SkippedUnsupported | Cancelled | Interrupted
```

Campaign states are:

```text
Scheduled | Queued | Running | Succeeded | Partial | Failed |
Cancelled | Missed
```

Retry rules:

- Opening, Like, Follow, and Save may re-read desired state and retry within a
  bounded budget.
- Comment, Repost, and Direct Message are never blindly repeated after their side
  effect may have been sent.
- `Uncertain` remains visible for operator review.
- A target identity failure blocks all remaining side effects for that assignment.
- A non-blocking action failure does not prevent later independent actions.
- Missing `openUrl`, stream, or identity-proof capability skips the assignment.
  Missing capability for one optional action skips that action, runs independent
  supported actions, and makes the assignment `Partial`.
- Assignment failures do not stop other actors or targets; the campaign becomes
  `Partial`.

Campaign aggregation is deterministic:

- `Succeeded`: every runnable assignment succeeded and there were no skipped,
  uncertain, interrupted, or failed assignments;
- `Partial`: at least one assignment succeeded or was already satisfied, and at
  least one other assignment/action was skipped, uncertain, interrupted, or failed;
- `Failed`: no assignment succeeded and at least one attempted assignment failed;
- `Cancelled`: cancellation completed before any assignment succeeded;
- a cancellation after prior success aggregates to `Partial`;
- `Missed`: a one-time schedule was never started because the desktop was not
  running when it became due.

Cancellation is cooperative. The executor finishes the current atomic gesture or
verification step, persists its state, closes transient UI on a best-effort basis,
and stops before the next action.

After a desktop crash, in-flight work becomes `Interrupted`; ambiguous side effects
become `Uncertain`. Recovery is manual through `Retry failed`. The engine rechecks
idempotent desired states before acting and preserves the original sampled plan.

## 13. Scheduling

The only scheduling modes are `RunNow` and `Once`. Store scheduled time in UTC and
display it in the desktop's local timezone.

The desktop process must be running at the scheduled time. On startup, any due
one-time schedule that never entered `Running` is changed to `Missed`; it is not run
late. A scheduled campaign resolves `Tất cả máy online` at its actual start time.
Explicit actors remain the snapshot captured when the schedule was created.

## 14. Persistence

Additive SQLite migrations create:

- `tiktok_accounts`
- `interaction_campaigns`
- `interaction_targets`
- `interaction_assignments`
- `interaction_action_runs`
- `interaction_artifacts`
- `interaction_schedules`

Campaign rows contain an idempotency `request_id`. Starting the same request twice
returns the existing campaign instead of creating duplicate side effects.

Assignments store immutable actor, device, target, effective settings, sampled
actions, and planner seed. Action runs store attempts, typed outcome, timing, and
evidence references. Artifacts store metadata and paths rather than frame bytes in
SQLite.

Proxy secrets are not copied into campaign snapshots or evidence. Store only the
proxy identifier and non-secret status required to explain what the operator chose.

## 15. Tauri API And Events

Commands:

```text
interaction_parse_links(raw_text)
interaction_preview(request)
interaction_start(request)
interaction_schedule(request)
interaction_get(campaign_id)
interaction_list(limit)
interaction_cancel(campaign_id)
interaction_retry(campaign_id, assignment_ids)
interaction_get_defaults()
interaction_save_defaults(settings)
interaction_list_accounts(udids)
```

`interaction_start` and `interaction_schedule` validate the entire request and
persist it transactionally before any background task starts. Request-level errors
such as zero valid targets or invalid percentages reject the request. Actor-specific
readiness errors become typed skipped assignments so eligible actors still run.

The backend emits `InteractionUpdated` with campaign summary and changed assignment
identifiers. The frontend fetches current detail after reconnect instead of assuming
that no missed event means no state change.

## 16. Desktop UX

Add `Tương tác` immediately after `Nuôi TT` in `ProfileToolbar`. Only one tool panel
is open at a time. Use a wide panel, approximately 720-780 px within responsive
desktop constraints, so the device grid remains visible.

### 16.1 Setup Tab

- segmented target selector: `Tất cả máy online | Chỉ định`;
- segmented distribution selector: `Tất cả | Phân bổ`;
- multiline TikTok URL input with inline per-line validation;
- compact target table showing normalized target, expected actors, action summary,
  state, and an edit icon;
- inline expanded target override row using `Inherit` for absent overrides;
- default watch duration and tri-state action controls;
- AI instruction and fallback pool shown only when Comment is enabled;
- allowlist/random-visible recipient controls shown only when Direct Message is
  enabled;
- `Chạy ngay | Hẹn một lần` control;
- disabled Start until a valid target and valid target-selection mode exist.

For `Chỉ định`, at least one actor must be selected. `Tất cả máy online` does not
require manual selection, but it still requires at least one valid URL.

### 16.2 Monitor Tab

- summary counts for waiting, running, succeeded, partial, failed, and skipped;
- target rows expandable to actor and action detail;
- inline typed errors and evidence links;
- Stop, Retry Failed, and Open on Device commands;
- no `window.alert` for runtime errors.

Closing the panel never cancels a campaign. Reopening it restores the active or most
recent campaign from backend state.

### 16.3 Proxy Page

Restore `Proxy` under the system navigation. Keep CRUD/export and add actor/device
assignment. Display two independent facts:

- proxy endpoint check from the desktop: unchecked, reachable, or unreachable;
- iPhone application state: `manual_required` or manually confirmed.

Do not label desktop reachability as device egress verification. Do not display
`applied` or `device IP verified` on the current unsupervised fleet.

## 17. Test Strategy

### 17.1 Pure Unit Tests

- direct video, photo, short, tracking, malformed, duplicate, userinfo, custom port,
  and unsupported-host link cases;
- redirect loops, off-domain redirect, timeout, and hop limit through a mock
  resolver transport;
- deterministic Cartesian and round-robin planning;
- default/override merge with explicit Inherit semantics;
- probability boundary validation and stable persisted sampling;
- state-machine transition and retry classification tests.

### 17.2 Persistence And Runtime Tests

- additive migration on an existing database;
- request idempotency;
- actor and target snapshot immutability;
- schedule start and missed schedule handling;
- crash conversion to Interrupted/Uncertain;
- cancellation and Partial aggregation;
- offline/busy actor skip behavior.

### 17.3 Driver And Coordination Tests

- fake driver call log proving `session -> stream -> openUrl`;
- profile-specific URL endpoint, authentication, schema, and request deadline tests;
- shared device reservation tests across Script, Nurture, Interaction, and Repair;
- global USB semaphore tests;
- generation-safe stream reuse and cleanup;
- no Safari or launch-argument fallback.

### 17.4 Frame And UI Tests

- retain all existing Like, Follow, Comment, popup, and system-alert fixtures;
- add real-device fixtures for Save, share drawer, Copy Link, Repost/Remove Repost,
  recipient selection, and send confirmation;
- test URL draft parsing, overrides, actor selection, distribution preview, action
  policy controls, scheduled-time validation, status rendering, and panel restore;
- verify compact layout and text containment at supported desktop sizes.

### 17.5 Live Gate

Before enabling `openUrl`, run direct-video, photo, and short-link cases and prove
target identity without polling WDA screenshots. Then gate every new action using
real frames and observable results:

- Like and Follow desired-state checks;
- Unicode AI comment with armed-send and sent confirmation;
- Save state transition;
- Copy Link clipboard identity;
- Repost state and ambiguous-result handling;
- allowlisted and random-visible direct share;
- popup suppression, cancellation, partial failure, and clean transport shutdown;
- `All` and `RoundRobin` using two devices under the qualified USB limit.

An action remains capability-gated until its own live gate passes. Existing nurture
unit tests and the live nurture harness are regression gates for the shared facade.

## 18. Rollout And Rollback

Implement the schema additively so older desktop versions ignore the new tables.
Keep Interaction behind runtime capabilities until `openUrl` and target identity
pass. Enable each side-effect action independently after its detector and live gate
pass.

Do not replace or modify the production IPA/manifest as part of the desktop feature.
Project 2 candidate B0/Gate B/Gate C and its later text/comment gate remain separate
Mac work. A failed Interaction gate disables that capability and leaves Nurture and
the current production Agent path unchanged.

Rollback consists of disabling Interaction capability exposure and reverting the
desktop feature code. Additive tables and history may remain; no destructive schema
rollback is required.

## 19. Delivery Slices

The implementation is divided into independently verifiable slices. A later slice
may depend on an earlier one, but each slice must leave the workspace passing its
own tests.

1. **Gate 0 and coordination:** shared device reservations, URL parser/resolver,
   typed `open_url`, artifact-bound capability, target identity proof, and live
   contract probe.
2. **Campaign core:** domain types, planner, probability snapshot, additive schema,
   store, scheduler, states, events, and Tauri commands using fake actions first.
3. **Existing verified actions:** action facade plus Watch, Like, Follow, Comment,
   Copy Link, and nurture parity regression.
4. **Desktop workflow:** Interaction panel, target overrides, monitoring, scheduling,
   restored Proxy page, actor assignment, and `manual_required` state.
5. **New action gates:** Save, Repost, allowlisted/random Direct Message detectors,
   fixtures, implementation, and live evidence.
6. **Fleet acceptance:** two-device All/RoundRobin run, cancellation, partial failure,
   transport cleanup, documentation, and capability enablement.

The desktop UI may be developed against mocks before Gate 0 passes, but production
execution stays disabled until `openUrl` and target identity are artifact-bound.

## 20. Final Decisions

- Architecture: dedicated Interaction engine with a shared verified-action facade.
- Devices: choose all online or explicit actors.
- Distribution: both All and RoundRobin.
- Configuration: campaign defaults plus per-target overrides.
- Actions: Off, Required, or Probability.
- Comments: contextual AI, per-target instruction, deterministic fallback pool.
- Direct share: allowlisted or random visible recipients.
- Failure: record Partial and continue.
- Scheduling: immediate or once; missed runs do not execute late.
- Availability: offline or externally busy actors are skipped immediately.
- Accounts: one default actor per iPhone now; extension fields only for multi-account.
- Proxy: manage, assign, and test endpoint; current iPhone application remains manual.
- MDM/supervision: entirely deferred from this project.
