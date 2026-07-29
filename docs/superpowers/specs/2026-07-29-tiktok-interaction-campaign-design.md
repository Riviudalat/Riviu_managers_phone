# TikTok Interaction Campaign Design

**Status:** Revised after conflict audit on 29/07/2026; pending final user review

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
- Give every optional action an `Off`, `Required`, or `Probability(percent)` policy.
- Verify every opened target through a disclosed, mandatory Copy Link identity gate.
- Support optional watch, like, follow, comment, save, repost, and direct share.
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
- Profile, LIVE, music, shop, search, and other non-post TikTok destinations. This
  phase accepts only video and photo posts, including short URLs that resolve to one
  of those two kinds.

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
: Is the only public allocator for device-driving work. It owns Nurture,
  Interaction, Script, Repair/lifecycle, manual device commands, Group Sync input,
  and any monitor command that changes the device screen. A supervisor lock around
  one driver call is not sufficient because two owners could otherwise alternate
  gestures on the same screen. Callers request one typed lease rather than acquiring
  nested locks themselves, so auto-repair can remain inside the same ownership
  context without recursively acquiring the high-level lease.

`StreamBudgetManager`
: Is owned by the coordinator and accounts for every live MJPEG producer, including
  device-grid background tiles. A `BackgroundStreamLease` holds capacity for the
  producer lifetime. Interaction work first owns a `DeviceExclusive` lease, then
  atomically upgrades it to `UiWithStream` after metadata/install work. The upgrade
  either consumes free capacity or revokes and transfers one eligible background
  lease; it never waits for a permit that the caller itself must park.

`InteractionScheduler`
: Starts immediate or one-time campaigns, groups runnable work by device, enforces
  durable dispatch, and requests atomic coordinator leases. It snapshots actor
  availability without reserving the entire fleet; each device obtains only its own
  non-blocking `DeviceExclusive` lease when the batch begins and upgrades to stream
  capacity immediately before session/MJPEG setup.

`DurableInteractionDispatcher`
: Treats SQLite as the only source of runnable work. In-memory channels and Tauri
  events only wake it. Claims use a persisted owner, timestamp, state, and revision;
  a worker may execute only after its claim transaction commits.

`TikTokActionExecutor`
: A narrow facade for verified TikTok actions. Like, Follow, Watch, and compatible
  frame checks may initially delegate to live-tested nurture helpers. Comment cannot
  delegate to the existing monolithic helper because campaign recovery requires the
  chosen text to be committed before typing. Extraction remains incremental and is
  protected by nurture parity tests.

`CommentPreparationService`
: Generates one contextual comment, chooses a deterministic fallback when needed,
  and commits the exact normalized text before `send_prepared_comment` may focus or
  type into TikTok.

`RecipientLocator`
: Locates share recipients from pixels through an injectable local text-recognition
  boundary. Allowlist matching uses an exact normalized ASCII `@handle`; a display
  label is presentation-only. Ambiguous or low-confidence results produce
  `NotConfirmed` without tapping. Direct Message remains capability-disabled until
  the recognizer, model artifacts, fixtures, and live gate are pinned and pass.

`InteractionStore`
: Persists campaign configuration, immutable snapshots, assignments, action runs,
  artifacts, schedules, and state transitions in SQLite.

### 4.2 Why Existing JobQueue Is Not Reused

`JobQueue` owns an internal per-UDID semaphore and stores generic script steps. Its
lock is not shared with Nurture, and its data model cannot represent account-target
assignments, per-action evidence, ambiguous side effects, or future dependencies.
Interaction records therefore remain separate, but `JobQueue` must stop owning an
independent device semaphore and receive the shared coordinator before Interaction
can be enabled.

### 4.3 USB Concurrency

The current machine has live-qualified only one or two concurrent MJPEG/control
pairs. The coordinator therefore grants at most one `UiWithStream` lease per UDID,
uses a global stream budget defaulting to `1`, and enforces a hard configurable
maximum of `2` until a new fleet test raises it. The limit counts producers, not
campaign tasks: background tiles, Nurture, Interaction, and Script streams all hold
the same permit for the stream lifetime.

Background streams outside the budget are parked rather than left producing bytes.
Their tiles retain the last frame with a visible stale/parked state. Foreground demand
preempts the low-priority sampler. Taking the Interaction `DeviceExclusive` lease
atomically reserves the target UDID and revokes its own background lease, if present;
stop confirmation frees that background capacity before metadata/install work begins.
Other typed owners may adopt or retain a read-only tile stream when their lifecycle
does not require a fresh session, but screen-changing ownership remains exclusive.

During the later upgrade, `acquire_foreground_or_transfer_background` verifies the
target owner token and atomically marks one eligible background lease `Revoking` so
no other claimant can use its capacity. Producer cancellation, generation advance,
and bounded stop confirmation occur outside the coordinator state mutex. A final
atomic transition retags that same capacity as the foreground reservation. Only then
may the caller start a new producer. Failure rolls back the upgrade and never creates
a second producer.

When no foreground workflow needs capacity, a low-priority background sampler
rotates permits across online tiles so every tile can obtain an initial and periodic
frame. A tile keeps its turn until one fresh frame arrives or 5 seconds elapse, then
yields; a failed tile backs off for 30 seconds. Rotation still obeys
session-before-stream, generation invalidation, and the same hard producer limit; it
never reopens a stream owned by foreground work.

Callers never acquire a device lock and stream semaphore independently. Interaction
first takes one non-blocking device lease, performs bounded install-only work without
holding stream capacity, then requests the atomic upgrade. While capacity is
unavailable the batch is `WaitingCapacity`; it holds only its own device lease, not
leases for the rest of the fleet. The ios-driver supervisor lock remains an internal
lower-level lock. No SQLite transaction is held while waiting for a lease, transfer,
or device operation.

`Tất cả` means every actor in the execution snapshot is planned; it does not mean all
devices stream or hold reservations simultaneously. An actor that is offline or busy
when its batch reaches the allocator is skipped immediately and is not silently
redistributed.

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
    state: AccountState,
}

struct TikTokTargetDraft {
    original_url: String,
    normalized_url: String,
    content_id: Option<String>,
    author: Option<String>,
    kind: TikTokTargetDraftKind,  // Video, Photo, ShortUnknown
    resolution_error: Option<TargetError>,
}

struct ResolvedTikTokTarget {
    original_url: String,
    normalized_url: String,
    resolved_url: String,
    target_key: String,
    content_id: String,
    author: Option<String>,
    kind: TikTokPostKind,         // Video or Photo only
    overrides: InteractionOverrides,
}

enum ActionPolicy {
    Off,
    Required,
    Probability { percent: u8 },
}

enum ShareMethod {
    Repost,
    DirectMessage(RecipientPolicy),
}

enum DistributionMode {
    All,
    RoundRobin,
}

enum ClipboardAccessMode {
    TargetBackgroundSafe,
    AgentForegroundRequired,
}
```

Each current device produces one stable actor with an identity equivalent to
`device:<udid>:default`. Assignment history stores both `account_id` and a UDID
snapshot so a future account rebind does not rewrite history.

Migration/device discovery idempotently creates this default binding for every known
device without overwriting an existing label or disabled state. A username is
optional, so current devices do not require a fake handle before they can run.

The binding is local operator metadata, not proof of the currently logged-in TikTok
identity. This phase neither reads TikTok credentials nor switches accounts. An
inactive or missing default binding is a typed actor preflight failure; labels in
history must not be described as live account verification.

For the current one-account-per-device phase, `device_meta.proxy_id` is the only
mutable proxy assignment. `AccountBinding` does not duplicate it. An immutable
`effective_proxy_id` may be copied into an assignment snapshot for audit. A future
multi-account migration may add an explicit account override with documented
inheritance, but it must not create a second owner now. Changing a device's proxy
assignment clears any prior manual-confirmation annotation.

The only dependency field required now is a nullable `parent_assignment_id` plus
typed output artifacts. A successful comment may emit:

```rust
struct CommentArtifact {
    target_key: String,
    account_id: AccountId,
    configured_account_handle: Option<String>,
    normalized_text: String,
    text_sha256: String,
    sent_at_utc: String,
    platform_comment_id: Option<String>,
    screenshot_path: String,
}
```

This reserves a typed dependency/output boundary without implementing a generic DAG.
It does not claim the current runtime can find the comment again: a later reply
phase must uniquely prove the source comment by platform ID or by a separately gated
author/text locator. A configured account handle alone is not live identity proof.

## 6. Link Parsing And Resolution

### 6.1 Accepted Inputs

Only HTTPS URLs whose host is exactly `tiktok.com` or ends in `.tiktok.com` are
accepted. Userinfo and custom ports are rejected. Supported forms include:

- `/@<handle>/video/<content-id>`
- `/@<handle>/photo/<content-id>`
- `vt.tiktok.com/<code>`
- `vm.tiktok.com/<code>`
- `/t/<code>`

Every short form must resolve to `/@<handle>/video/<content-id>` or
`/@<handle>/photo/<content-id>`. Profile, LIVE, music, shop, search, and any other
final path are rejected with a typed `UnsupportedTargetKind` error.

Blank lines are ignored. Syntax errors, unsupported hosts, unsupported paths, and
resolution failures are reported against their original line. A campaign can use
the valid lines; it cannot start when no valid target remains.

### 6.2 Redirect Rules

The resolver disables automatic redirect following and handles each hop itself:

- validate scheme, host, userinfo, and port on every hop;
- accept only 301, 302, 303, 307, or 308 redirect responses;
- cap redirects at 5 and the full resolution at 10 seconds;
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

At execution time, the scheduler takes a non-mutating availability snapshot before
the planner expands targets. Actors already known to be offline, unsupported, or
owned by another workflow are excluded. It does not hold leases for actors waiting
behind another device's USB work. Round-robin planning uses this eligible snapshot.
When each device batch reaches execution, the coordinator performs one non-blocking
atomic lease request. A newly busy or disconnected actor becomes
`SkippedUnavailable`; the scheduler does not wait or silently redistribute its
assignments because other actors may already have started related work.

Distribution then works as follows:

- `All`: deterministic Cartesian product of actor order and target order.
- `RoundRobin`: target at index `i` is assigned to actor
  `i % actor_count`, preserving request order.

The execution-time actor snapshot, target revalidation results, planner seed,
assignments, effective settings, and every `NotPlanned`/`Pending` action decision are
committed in one transaction before any device lease or side effect. A crash before
that commit leaves the dispatch safely re-plannable; a crash afterward reuses the
persisted plan and never samples probability again.

If no eligible actor remains at execution time, the campaign becomes `Failed` with
typed reason `NoEligibleActors`. Other per-actor skips make an otherwise runnable
campaign `Partial`.

## 8. Configuration And Probability

Campaign defaults contain:

- minimum and maximum watch duration;
- Watch, Like, Follow, Comment, Save, Repost, and Direct Message policies;
- AI comment instruction;
- fixed comment fallback pool;
- direct-share recipient mode and allowlist;
- pacing and jitter within bounded product defaults.

Initial defaults are Watch Required at 4-12 seconds, 0.6-1.8 seconds between planned
actions, and 1.5-4.0 seconds between targets. Watch bounds are configurable from
1-300 seconds and must satisfy minimum <= maximum. Probability is an integer from
0-100; the backend canonicalizes 0 to Off and 100 to Required before hashing,
previewing, or persisting a request.

Each target stores an override patch. An absent field means `Inherit`, not `Off`.
The effective target configuration is computed once during planning and stored in
the assignment snapshot.

For every `Probability` policy, the planner samples the decision once and persists
the result. A retry, process restart, or UI reopen must not roll the probability
again. Required actions are always selected; Off actions are never scheduled.

The planner also persists a seed and final action plan for auditability. Retry only
reconsiders actions that were selected in the original plan.

Copy Link is not an optional action policy. It is a mandatory, user-visible identity
precondition for every assignment. Probability decisions that evaluate false and
`Off` policies are stored as `NotPlanned`, not `Skipped`, and do not make an
otherwise successful assignment partial. A request with every optional action Off
remains a valid open-and-verify campaign. Comment requires either configured AI
credentials or a non-empty fallback pool. Allowlisted Direct Message requires at
least one exact handle, normalizes/deduplicates the list, and uses the persisted
assignment seed to select exactly one handle for each planned send.

## 9. Execution Lifecycle

The scheduler first acquires one coordinator-owned `DeviceExclusive` lease. Inside
that ownership context it uses new lifecycle primitives rather than the current
`preflight_agent()` or `repair_agent_locked()`, because both generic paths finish by
creating a session and MJPEG stream. The logical lifecycle for a device batch is:

```text
non-blocking DeviceExclusive lease
-> park and invalidate this device's previous/background stream generation
-> inspect Agent/TikTok metadata, protected auth, transport, and geometry
   without creating session or stream
-> when app is missing or attested metadata mismatches, run repair_install_only_locked
-> atomically upgrade ownership to UiWithStream capacity
-> foreground TikTok
-> create or attach the profile-approved UI session
-> start MJPEG, bind it to the new generation, and receive the first frame
-> open the target URL
-> verify the target identity
-> watch for the planned duration
-> execute planned actions
-> persist evidence and final state
-> close transient UI
-> release the batch context and restore eligible background streaming within budget
```

`repair_install_only_locked` verifies the production artifact checksum, stops only
matching old processes/transports, installs the missing or mismatched app, and proves
installed metadata plus protected auth. It never creates an automation session or
opens MJPEG. Auth, session, or MJPEG failures do not trigger reinstall. The generic
health/repair commands remain unchanged for their existing callers.

The session must always exist before MJPEG starts. Fresh-text eligibility is computed
from the persisted sampled action plan, not merely from a `Required` policy. Any
selected Comment or allowlisted Direct Message batch conservatively requires text
capability. If Gate 0 qualifies production RT-MMO as `TargetBackgroundSafe`, its
atomic transition follows the live-confirmed sequence exactly: bootstrap a fresh
Agent process, foreground TikTok, create a new automation session, then start MJPEG.
Other modes use only their profile-approved pre-identity path and apply the final
session transition in Section 10.2. The background poller observes the same lease and
cannot interleave a session or stream. A healthy owned context may be reused for
consecutive targets on the same actor, but no identity or evidence requirement may
be skipped.

The popup watcher remains active. While the executor owns a comment or share drawer,
it uses `run_suppressible()` so the watcher continues classifying without tapping
the controlled UI.

## 10. Open URL Capability And Target Identity

Opening a link is the first delivery blocker. The production RT-MMO inventory has
an oracle-only `/url` candidate, while baseline WDA exposes a session route. Static
inventory does not prove method, authentication, body, session, or runtime behavior.

Add a typed `UiSession::open_url` boundary with a profile-specific adapter. The WDA
request body is exactly `{ url, bundleId: <configured TikTok bundle>,
idleTimeoutMs: 0 }`; omitting the bundle risks Safari and a positive idle timeout
enters the TikTok quiescence path. The adapter pins whether the proven profile uses
a session or sessionless route, applies the protected auth header, and puts the
10-second deadline on the request itself rather than wrapping it in
`tokio::time::timeout`.
Do not silently fall back to Safari UI or launch arguments. `openUrl` is exposed
only after a live contract probe proves:

- protected authentication is enforced;
- the expected endpoint and request schema work;
- TikTok becomes foreground;
- a direct link, short video link, and photo link open successfully;
- the session and stream remain healthy;
- the opened content can be identified.

Project 2 candidate protocol v2 and the current production IPA/manifest remain
unchanged until their existing Mac gates pass. Interaction work must not weaken or
rewrite those attestations.

Before the live gate, the driver reports `openUrl=unsupported`. After approval,
`DeviceControllerCapabilities.ui.open_url` is keyed to the exact Agent artifact
SHA-256, agent/protocol and driver-adapter versions, active transport, WDA profile
route contract, qualified iOS range, installed TikTok version/build, and probe
evidence. A change to any keyed dimension does not inherit support. This
driver-adapter capability remains separate from `AgentStatus.features`, which is
derived from the signed manifest.
A future Agent protocol may publish `openUrl` in protected health, but this project
does not edit the current production manifest to manufacture that claim. Interaction
continues using the production RT-MMO artifact until a Riviu-built candidate passes
its separate text, clipboard, open-URL, and TikTok evidence gates.

### 10.1 Capability Sources And Geometry

The mandatory identity path has its own typed capability:
`DeviceControllerCapabilities.ui.target_identity_copy_link`. It is not inferred from
`AgentStatus.features=clipboard`. Its qualification key includes:

- Agent artifact SHA-256, agent/protocol version, and driver-adapter version;
- active transport adapter (`LegacyUsbmuxTransport` or `RsdTransport`);
- exact open-URL and clipboard route/auth/body contracts;
- `ClipboardAccessMode`;
- qualified iOS range and installed TikTok version/build;
- Share/Copy Link layout and detector-set versions;
- point-space logical bounds, scale evidence, and portrait/orientation proof.

Every optional action capability extends the same base qualification key with its
route, detector/model version, and live evidence. Manifest feature names alone never
enable Like, Follow, Comment, Save, Repost, or Direct Message for Interaction.

The non-mutating inspect stage obtains installed TikTok metadata and active transport
from the Device Bridge, not from the Agent manifest. Geometry must come from a
protected runtime bounds/orientation contract or a separately live-qualified fixed
geometry profile that also matches observed frame dimensions and device capability
metadata. The production manifest's fixed `375x667` fields alone are not runtime
proof. Until another profile passes, Interaction fails closed outside the exact
qualified 375x667 portrait profile; newer devices gain support by adding evidence to
capability negotiation, not by inheriting iPhone 8 coordinates.
Before every coordinate action, the current frame/orientation evidence must still
match the qualified geometry profile. Rotation or bounds drift produces
`UnsupportedGeometry` before any tap.

### 10.2 Identity Gate

A changed frame and visible TikTok rail prove only that TikTok is displaying some
content. They do not prove it is the requested content. No Like, Follow, Comment,
Save, Repost, or Direct Message may run before `targetIdentityConfirmed`.

Add typed, profile-specific clipboard set/get operations to the driver boundary.
`TargetBackgroundSafe` performs them while TikTok remains foreground.
`AgentForegroundRequired` uses a guarded transition:

1. pause/clear the current stream generation;
2. foreground the Agent and verify its stable PID before the clipboard operation;
3. foreground TikTok again and verify its bundle plus a fresh target frame;
4. recreate session-before-stream as required by the profile.

For `AgentForegroundRequired`, sentinel setup performs this transition before Share,
and clipboard read-back performs it again after Copy Link. The final return to TikTok
creates the action session: fresh text session when the persisted plan needs text,
ordinary otherwise, followed by a new MJPEG generation and target-frame confirmation.
No pre-identity text session is trusted across an Agent foreground switch.

The primary identity proof is:

```text
open target
-> read at most 64 KiB of prior clipboard data into bounded process memory
-> store only prior type, byte length, and SHA-256 in evidence
-> write a namespaced per-attempt random sentinel and read it back
-> open Share
-> commit identity_copy_intent=issued
-> Copy link
-> read clipboard through the Agent
-> require that the sentinel was replaced by a new HTTPS TikTok URL
-> resolve a copied short URL through the same bounded resolver
-> normalize the final post URL
-> compare content ID and post kind with planned target
-> close Share
```

The identity probe is mandatory and visibly disclosed in Setup; Copy Link is not an
`Off/Required/Probability` control. The target URL remains in the clipboard after
successful verification. Clipboard/open-link behavior is qualified according to the
declared `ClipboardAccessMode`, including verified return to TikTok when required,
as part of Gate 0. A completed read that yields a stale/unchanged sentinel,
unsupported target, exhausted resolution failure, or content mismatch is
`TargetUnverified`. If the engine cannot determine whether the committed Copy Link
tap or its read-back completed, the assignment is `Uncertain`. Both outcomes skip all
remaining side effects. The design makes no claim about whether TikTok counts Copy
Link in its own analytics.

Raw prior clipboard bytes never enter SQLite, artifacts, logs, or events. If the
prior value cannot be read and retained within 64 KiB, verification stops before
writing the sentinel. Every unsuccessful, cancelled, or timed-out identity attempt
best-effort restores the in-memory prior value and verifies its hash. A restore
failure is reported as `ClipboardRestoreFailed` alongside the primary outcome. The
sentinel includes the campaign/attempt ID; startup cleanup may clear a leftover owned
sentinel under a coordinator lease before dispatching work after a process crash, but
it does not claim to reconstruct prior bytes.

The Copy Link tap is not automatically repeated after
`identity_copy_intent=issued`. A crash or ambiguous read-back after that commit makes
the assignment `Uncertain`; a deterministic mismatch becomes `TargetUnverified`.
Only read-back, bounded URL resolution, cleanup, and drawer-close steps may retry
without another Copy Link tap. If the intent commit fails, the executor restores the
clipboard/closes Share and does not tap Copy Link.
An assignment with issued identity intent but no confirmed identity is ineligible for
same-assignment Retry Failed; the operator must create a new explicit campaign.

## 11. Action Semantics And Evidence

An HTTP success response or gesture acknowledgement is never sufficient evidence.

`Watch`
: Requires the target-ready state to remain observable and elapsed time to meet the
  planned duration. Frame digests and timestamps are stored; continuous screenshots
  are not required.

`Like`
: Uses a new fixture-backed `ActionRailLocator` that returns coordinates rather than
  only a boolean. It must locate both known TikTok rail layouts from either the red
  Follow badge or the white icon chain when the author is already followed. Redness
  must cross the verified threshold. A target already liked is `AlreadySatisfied`.

`Follow`
: Requires the Follow badge to disappear. Absence is `AlreadySatisfied` only after
  the rail itself was positively located; an unknown or ambiguous rail is
  `NotConfirmed` and is never tapped.

`Comment`
: Generate one comment from the stable target frame plus campaign and per-target
  instructions. `prepare_and_persist_comment` commits the generated text before any
  drawer gesture. If generation fails and the fallback pool is non-empty, it commits
  one deterministic entry; otherwise the action fails before opening the drawer.
  `send_prepared_comment` accepts only stored text. The Send control must arm before
  tap and disarm back to the open drawer afterward.

`Save`
: Requires a fixture-backed bookmark-state transition. Already saved is
  `AlreadySatisfied`.

`TargetIdentityCopyLink`
: Is the mandatory precondition described in Section 10.2, not a configurable
  action. It requires a fresh clipboard read-back containing the same normalized
  content ID and supported post kind.

`Repost`
: Must distinguish `Repost` from `Remove repost`. If the post-tap state is ambiguous,
  record `Uncertain` and do not tap again. Seeing the verified `Remove repost` state
  before tapping means `AlreadySatisfied`.

`DirectMessage`
: Supports two selectable modes. Allowlist mode stores a normalized handle plus an
  optional display label, but locator matching uses only the exact handle.
  Random-visible mode selects exactly one currently visible recipient only when the
  locator returns an unambiguous eligible tile. Every locator result records crop,
  recognized handle, confidence, and detector/model version. Sending requires a
  post-send state or toast and captures the resolved recipient evidence. Text search
  requires a fresh text session. Missing OCR/model capability, multiple matches, or
  low confidence disables the tap and returns `NotConfirmed`.

For random-visible mode, the locator sorts eligible unambiguous results by normalized
handle and screen position, then uses the persisted assignment seed to select one.
The resolved handle, crop, and coordinate are committed before tapping. A pre-send
retry must re-locate that same handle or stop as `NotConfirmed`; it never chooses a
different recipient for the same assignment.

All rail/share coordinates are derived from the current frame and the qualified
point-space geometry capability. Manifest dimensions alone are insufficient. The
iPhone 8 measurements are fixtures, not coordinates inherited by newer devices.

The default action order is target verification, Watch, Like, Follow, Comment, Save,
and then the remaining Share methods. Share UI is closed between operations unless
the current live contract proves that reuse is stable.

## 12. Retry, Cancellation, And Recovery

Action states are:

```text
NotPlanned
Pending -> Running -> Succeeded | AlreadySatisfied | NotConfirmed |
                      Uncertain | Failed | Skipped | Interrupted
```

Assignment states include:

```text
Queued -> WaitingCapacity -> Preparing -> Session -> Stream -> Opening
       -> Verifying -> Acting
       -> Succeeded | Partial | Failed | SkippedUnavailable |
          SkippedUnsupported | Cancelled | Interrupted | Uncertain
```

Campaign states are:

```text
Scheduled | Queued | Running | Succeeded | Partial | Failed |
Cancelled | Interrupted | Missed
```

Typed result codes do not create hidden states: `TargetChanged` and deterministic
`TargetUnverified` terminate the assignment as `Failed`; ambiguous identity is
`Uncertain`; `UnsupportedGeometry` or missing Required capability is
`SkippedUnsupported`; offline/owned devices are `SkippedUnavailable`.

Retry rules:

- Opening, Like, Follow, and Save may re-read desired state and retry within a
  bounded budget.
- Comment, Repost, and Direct Message are never blindly repeated after their side
  effect may have been sent.
- `Uncertain` remains visible for operator review and is not eligible for automatic
  Retry Failed. The operator may create a new, explicit campaign after review; the
  original action is never silently repeated.
- A target identity failure blocks all remaining side effects for that assignment.
- A non-blocking action failure does not prevent later independent actions.
- Missing `openUrl`, stream, or identity-proof capability skips the assignment.
  Missing capability for a `Required` action also skips the assignment before any
  optional side effect. Missing capability for a Probability-selected action skips
  only that action, runs independent supported actions, and makes the assignment
  `Partial`.
- Assignment failures do not stop other actors or targets; final campaign state is
  derived only after all children are terminal and may be `Partial` or `Failed`.

Campaign aggregation is a pure function of persisted campaign-actor, assignment, and
action states. Child transition, counters, campaign state, result code, and revision
change in one transaction. `NotPlanned` actions are ignored, while actor-level skips
participate. A positive outcome is an assignment with at least one planned action in
`Succeeded` or `AlreadySatisfied`, or an open-and-verify assignment with no planned
optional action whose mandatory identity gate succeeded. Identity success alone does
not make an assignment positive when all requested actions failed.

- while any child is nonterminal, an executing campaign remains `Running`;
- all terminal with only positive outcomes becomes `Succeeded`;
- at least one positive and one negative outcome becomes `Partial`;
- zero positive outcomes plus any `Interrupted` or `Uncertain` child becomes
  `Interrupted`;
- zero positive outcomes with failures, not-confirmed results, or skips becomes
  `Failed`, with a typed result such as `NoEligibleActors`,
  `NoRunnableAssignments`, or `NoSupportedAssignments`;
- cancellation with zero positive outcomes becomes `Cancelled`; cancellation after
  a positive outcome becomes `Partial`;
- a one-time schedule beyond its lateness tolerance without starting becomes
  `Missed`.

Cancellation is cooperative and durable. The command persists
`cancel_requested_at`; an in-memory token is only a latency optimization. The
executor checks persisted cancellation between atomic steps, finishes the current
gesture or verification, persists its state, closes transient UI on a best-effort
basis, and stops before the next action.

For Comment, Repost, and Direct Message, the store commits
`effect_intent=issued` immediately before the final side-effect gesture. Completion
and evidence are committed afterward. A crash with issued intent becomes
`Uncertain`; no issued intent becomes `Interrupted`. `Retry failed` accepts only
eligible Failed/Interrupted work without ambiguous intent. Idempotent actions re-read
desired state, and every retry preserves the original sampled plan and prepared
comment.
The executor never performs the final tap unless the intent commit succeeds; a
database error aborts before the side effect.

Interaction preserves the established text recovery invariant. Two consecutive
`TextNotArmed` outcomes before any send intent trigger one coordinator-owned recovery
inside the existing device lease: stop the current stream, advance its generation,
foreground TikTok, create a fresh text session, start MJPEG and await its first frame,
then atomically replace the executor/feed and `ScreenWatcher` session handles. The
recovery never reacquires `DeviceWorkCoordinator`. The same persisted comment may be
retried within its bounded budget. `TextNotSent` means the send gesture may have
occurred, so it is `Uncertain` and is never retried.

Run-now dispatch is durable. SQLite is the only source of runnable work; channels and
events only wake the dispatcher. The same transaction that creates a `Queued`
campaign also inserts a pending dispatch row. One dispatcher atomically claims it
with owner, timestamp, and revision before starting workers.

On startup, claims from older process instances are audited transactionally. A
campaign that remained entirely `Queued`, or a stale `WaitingCapacity` claim, may
be requeued because device preparation never began. Once any assignment entered
`Preparing`, unfinished work in that campaign is frozen for manual retry: issued
effects become `Uncertain`, all other unfinished work becomes `Interrupted`.
Automatic dispatch never continues the remainder of a partially executed campaign
after process loss.

## 13. Scheduling

The only scheduling modes are `RunNow` and `Once`. Store scheduled time in UTC and
display it in the desktop's local timezone. The API accepts an RFC 3339 instant with
an explicit offset and rejects an already-past value at creation. The fixed lateness
tolerance is 60 seconds: a due schedule discovered within that window is dispatched
once; beyond it, an unstarted schedule becomes `Missed`. This covers process restart
and machine sleep without an unbounded late run.

At actual execution, every stored target is resolved again through the same redirect
policy and must produce the originally snapshotted content ID and post kind. A
different ID/kind is `TargetChanged`; a resolution failure is `TargetUnverified`.
That target's assignments terminate `Failed` before opening TikTok. A scheduled
campaign resolves `Tất cả máy online` at its actual start time. Explicit actors
remain the snapshot captured when the schedule was created.

## 14. Persistence

Additive SQLite migrations create:

- `tiktok_accounts`
- `interaction_campaigns`
- `interaction_campaign_actors`
- `interaction_targets`
- `interaction_assignments`
- `interaction_action_runs`
- `interaction_artifacts`
- `interaction_schedules`
- `interaction_dispatch`
- `interaction_retry_requests`

Campaign rows contain an idempotency `request_id` and hash of the normalized
request. Repeating the same pair returns the existing campaign; reusing the ID with
different input returns `IdempotencyConflict`. Retry has its own stable
`retry_request_id`. Campaign-actor rows preserve the selection snapshot, eligibility
decision, and terminal actor outcome even when no assignment was planned.

Required constraints include:

- `UNIQUE(interaction_campaigns.request_id)`;
- `UNIQUE(interaction_retry_requests.request_id)`;
- `UNIQUE(interaction_targets.campaign_id, target_key)`;
- `UNIQUE(interaction_assignments.campaign_id, account_id, target_id)`;
- `UNIQUE(interaction_action_runs.assignment_id, action_ordinal, attempt_no)`.

Assignment lookup, dispatch state, campaign state/revision, actor/UDID, target,
artifact retention, and schedule due-time columns have explicit query indexes.
Campaign summaries use a monotonically increasing revision; claims and transitions
use a state-plus-revision compare-and-swap so a zero-row update means ownership was
lost.

Assignments store immutable actor, device, target, effective settings, sampled
actions, planner seed, identity state, and `identity_copy_intent`. Action runs store
attempts, `effect_intent`, typed outcome, timing, and evidence references. Artifacts
store metadata and paths rather than frame bytes in SQLite.

Proxy secrets are not copied into campaign snapshots or evidence. Store only the
proxy identifier and non-secret status required to explain what the operator chose.
Endpoint-test errors and logs redact usernames, passwords, and URL userinfo.

Numbered migrations are transactional. Database initialization enables WAL once.
Every opened connection enables foreign keys, uses `synchronous=FULL`, and has a
5-second busy timeout; durability is required because losing a committed side-effect
intent can duplicate a send. Interaction mutations go through one serialized
blocking writer with short transactions. No SQLite transaction or `rusqlite` call
crosses an `.await` or runs on an async runtime worker. `InteractionUpdated` is
emitted only after commit and includes the committed revision. Tests still exercise
concurrent assignment workers because other application modules share the database.

One campaign accepts at most 500 resolved targets and 10,000 assignments after
distribution. The backend rejects excess targets before campaign persistence. It
checks assignment count again after the execution-time actor snapshot; this is
mandatory for scheduled `Tất cả máy online`, whose future fleet size is unknown. An
excess becomes `Failed/TooManyAssignments` before assignments or device actions are
created. Frontend preview is provisional, not the authority. Assignment and artifact
reads are cursor-paginated.

Evidence is bounded. Successful actions keep cropped before/after proof plus hashes
and timing; only failed, uncertain, or not-confirmed outcomes keep a full frame when
needed. Every artifact records a relative managed path, SHA-256, MIME type, byte
size, evidence kind, creation/retention/purge times, and pin state. Files are written
to a temp path and atomically renamed before the metadata transaction; failed
transactions remove the orphan, and startup reconciliation marks missing files.
Startup also removes unreferenced temp/final files left by a crash. Cleanup
canonicalizes every path under the Interaction artifact root.

Active or explicitly pinned artifacts are never purged. Successful completed
evidence expires after 14 days; failed, uncertain, and not-confirmed evidence expires
after 30 days. A 5 GiB global cap evicts the oldest unpinned completed files first.
Database metadata and cryptographic hashes remain after file eviction.

## 15. Tauri API And Events

Commands:

```text
interaction_parse_links(raw_text)
interaction_preview(request)
interaction_start(request)
interaction_schedule(request)
interaction_get(campaign_id)
interaction_list(cursor, limit, status_filter)
interaction_list_targets(campaign_id, cursor, limit)
interaction_list_assignments(campaign_id, cursor, limit, filters)
interaction_get_assignment(assignment_id)
interaction_list_action_runs(assignment_id, cursor, limit)
interaction_cancel(campaign_id)
interaction_retry(campaign_id, assignment_ids, retry_request_id)
interaction_open_on_device(assignment_id)
interaction_get_defaults()
interaction_save_defaults(settings)
interaction_list_accounts(udids)
```

`interaction_start` and `interaction_schedule` validate the entire request and
persist it transactionally before any background task starts. Start requires every
runnable target to be a `ResolvedTikTokTarget`, validates percentages, comment and
recipient prerequisites, and enforces the target plus currently determinable
assignment caps. The execution planner rechecks the final assignment cap after the
actor snapshot. Actor-specific readiness errors become typed skipped assignments so
eligible actors still run.
Run-now persistence includes its dispatch row in the same transaction.

The backend emits `InteractionUpdated` with campaign summary and changed assignment
identifiers. The frontend fetches current detail after reconnect instead of assuming
that no missed event means no state change.
`interaction_get` returns summary/configuration and target counts, not an unbounded
assignment graph. List APIs use opaque keyset cursors, default limit 50 and maximum
200; they never use offset pagination. `interaction_open_on_device` and every other
command that can navigate or gesture must acquire the same `ManualControl` lease;
it returns typed `DeviceBusy` while another owner holds the device.

## 16. Desktop UX

Add `Tương tác` immediately after `Nuôi TT` in `ProfileToolbar`. Only one tool panel
is open at a time. Use a wide panel, approximately 720-780 px within responsive
desktop constraints, so the device grid remains visible.

Because stream capacity is shared, a parked device tile shows its last frame,
`Đã tạm dừng`, and the last-frame time. A tile with no sampled frame shows an explicit
waiting state; it must not display a stale image as live or a spinner that implies an
active producer.

### 16.1 Setup Tab

- segmented target selector: `Tất cả máy online | Chỉ định`;
- segmented distribution selector: `Tất cả | Phân bổ`;
- multiline TikTok URL input with inline per-line validation;
- compact target table showing normalized target, expected actors, action summary,
  state, and an edit icon;
- inline expanded target override row using `Inherit` for absent overrides;
- a fixed `Xác minh bài bằng Copy Link` precondition row with no disable control;
- default watch duration and tri-state controls for optional actions only;
- AI instruction and fallback pool shown only when Comment is enabled;
- allowlist/random-visible recipient controls shown only when Direct Message is
  enabled;
- `Chạy ngay | Hẹn một lần` control;
- disabled Start until a valid target and valid target-selection mode exist.

For `Chỉ định`, at least one actor must be selected. `Tất cả máy online` does not
require manual selection, but it still requires at least one valid URL.
An action whose runtime capability has not passed is disabled with its typed status;
the UI does not let a draft imply that Direct Message/OCR or another gated action is
available.

### 16.2 Monitor Tab

- summary counts for waiting, running, succeeded, partial, failed, and skipped;
- target rows expandable to actor and action detail;
- inline typed errors and evidence links;
- Stop, Retry Failed, and Open on Device commands;
- cursor-backed virtualized rows for large campaigns;
- no `window.alert` for runtime errors.

Retry Failed is enabled only for backend-reported eligible assignments. Uncertain
effects and an issued-but-unconfirmed identity Copy Link remain read-only with their
typed reason.

Closing the panel never cancels a campaign. Reopening it restores the active or most
recent campaign from backend state.
Open on Device reports `DeviceBusy` instead of navigating a device owned by a running
workflow.

### 16.3 Proxy Page

Restore `Proxy` under the system navigation. Keep CRUD/export and add device
assignment. Current actor rows display their inherited device default but do not own
an independent proxy edit. Display two independent facts:

- proxy endpoint check from the desktop: unchecked, reachable, or unreachable;
- iPhone application state: `manual_required` or manually confirmed.

Do not label desktop reachability as device egress verification. Do not display
`applied` or `device IP verified` on the current unsupervised fleet.

The `proxies` table is the catalog and `device_meta.proxy_id` is the current device
default. The planner snapshots only effective proxy ID, configuration revision,
source (`device_default`), and non-secret status. Endpoint checks and manual
confirmation are annotations keyed to that revision; editing host, port, credentials,
or device assignment invalidates both. Manual confirmation never changes the typed
device capability `unsupported_unsupervised/manual_required` into `applied`.

This phase preserves the existing proxy-secret storage format so restoration of the
page does not become a credential migration project. Passwords and authenticated
URLs must never enter Interaction snapshots, frontend events, evidence, traces, or
unredacted errors. Moving legacy proxy passwords to the OS credential store is a
separate, explicit migration with rollback and export compatibility.

## 17. Test Strategy

### 17.1 Pure Unit Tests

- direct video, photo, short, tracking, malformed, duplicate, userinfo, custom port,
  unsupported-host, and unsupported profile/LIVE/music/shop link cases;
- redirect loops, off-domain redirect, timeout, and hop limit through a mock
  resolver transport;
- deterministic Cartesian and round-robin planning;
- default/override merge with explicit Inherit semantics;
- probability boundary validation and stable persisted sampling;
- `NotPlanned` exclusion and exhaustive actor/assignment/action aggregation;
- state-machine transition and retry classification tests.

### 17.2 Persistence And Runtime Tests

- additive migration on an existing database;
- request/retry idempotency and conflicting request-hash rejection;
- actor and target snapshot immutability;
- schedule start, 60-second lateness, target re-resolution, and target-change handling;
- durable dispatch recovery for Queued and WaitingCapacity;
- partial-execution crash freeze plus intent-based Interrupted/Uncertain conversion;
- cancellation and Partial aggregation;
- offline/busy actor skip behavior without fleet-wide held leases;
- two concurrent assignment workers under WAL/busy-timeout/write serialization;
- unique/CAS constraints, keyset pagination, caps, artifact retention/path safety,
  and event-after-commit ordering.

### 17.3 Driver And Coordination Tests

- fake driver call log proving non-mutating inspect, foreground, session, stream,
  first frame, and exact `openUrl` body in that order;
- profile-specific URL endpoint, auth, session scope, schema, and request deadline;
- shared device ownership tests across Script, Nurture, Interaction, Repair, manual
  commands, Group Sync, and Open on Device;
- stream-budget tests that count and park background producers, retain stale tiles,
  prevent poller reopen, rotate idle background tiles, and restore only within the
  configured budget;
- budget-1 transfer test proving foreground revokes/retags a background permit
  without deadlock or two simultaneous producers;
- install-only repair tests proving no session/MJPEG creation and no reinstall for
  auth/session/stream failures;
- fresh-text transition tests proving no old preflight session/stream and no nested
  coordinator reacquisition;
- generation-safe stream reuse and cleanup;
- artifact/driver/transport/iOS/TikTok/layout/geometry-bound capability negotiation,
  including fail-closed non-qualified dimensions/orientation;
- both ClipboardAccessMode transitions, bounded prior-value handling, sentinel
  restore/cleanup, committed Copy Link intent/no-repeat behavior, short-link
  resolution, and exact content-ID proof;
- two-strike TextNotArmed recovery with atomic executor/watcher handle swap, and no
  retry after TextNotSent;
- no Safari or launch-argument fallback.

### 17.4 Frame And UI Tests

- retain all existing Like, Follow, Comment, popup, and system-alert fixtures;
- add real-device fixtures for Save, share drawer, Copy Link, Repost/Remove Repost,
  recipient selection, and send confirmation;
- add both action-rail layouts with followed/unfollowed and liked/unliked states;
- test exact-handle recipient matching, ambiguous/low-confidence OCR rejection, and
  runtime-coordinate derivation;
- test URL draft parsing, overrides, actor selection, distribution preview, action
  policy controls, scheduled-time validation, status rendering, and panel restore;
- verify compact layout and text containment at supported desktop sizes.

### 17.5 Live Gate

Before enabling `openUrl`, run direct-video, photo, and short-link cases and prove
the complete mandatory Copy Link identity gate without polling WDA screenshots.
Record the exact artifact/protocol/driver/transport/iOS/TikTok/layout/
clipboard-mode/geometry/orientation qualification key. Each ClipboardAccessMode must
pass its own foreground-transition contract before exposure. Then gate every new
action using real frames and observable results:

- Like and Follow desired-state checks;
- Unicode AI comment with armed-send and sent confirmation;
- two-strike TextNotArmed recovery and TextNotSent no-retry classification;
- Save state transition;
- clipboard sentinel replacement, copied short-link resolution, and content identity;
- Repost state and ambiguous-result handling;
- allowlisted and random-visible direct share;
- popup suppression, cancellation, partial failure, and clean transport shutdown;
- `All` and `RoundRobin` using two devices under the qualified USB limit.

An action remains capability-gated until its own live gate passes. Existing nurture
unit tests and the live nurture harness are regression gates for the shared facade.

## 18. Rollout And Rollback

Implement the schema additively so older desktop versions ignore the new tables.
Keep Interaction behind runtime capabilities until `openUrl`, clipboard operations,
Share/Copy Link location, and target identity pass for the complete capability tuple.
Enable each side-effect action independently after its detector and live gate pass.
The current production runtime remains RT-MMO; the Riviu-built candidate is not
selected merely because its unrelated Project 2 gates pass.

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

1. **Gate 0 and coordination:** two-stage device/stream ownership, atomic background
   permit transfer, install-only repair, URL parser/resolver, typed `open_url` and
   clipboard-mode boundaries, qualified geometry, Share/Copy Link locator, complete
   capability-key binding, target identity proof, and live contract probe.
2. **Campaign core:** domain types, planner, probability snapshot, additive schema,
   store, scheduler, states, events, and Tauri commands using fake actions first.
3. **Existing verified actions:** action facade plus Watch, the new rail locator,
   Like, Follow, prepared/persisted Comment, and nurture parity regression.
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
- Identity: mandatory disclosed Copy Link proof before every optional side effect.
- Actions: optional actions use Off, Required, or Probability.
- Comments: contextual AI, per-target instruction, deterministic fallback pool.
- Direct share: allowlisted or random visible recipients.
- Failure: record Partial and continue.
- Scheduling: immediate or once; missed runs do not execute late.
- Availability: offline or externally busy actors are skipped immediately.
- Accounts: one default actor per iPhone now; extension fields only for multi-account.
- Targets: video/photo posts only; short links must resolve to one of those kinds.
- Coordination: every device-driving command and MJPEG producer shares one allocator.
- Compatibility: device/transport/TikTok/geometry capabilities fail closed and gain
  newer-iPhone support only through versioned live qualification.
- Proxy: `device_meta.proxy_id` is canonical now; manage, assign, and test endpoint;
  current iPhone application remains manual.
- MDM/supervision: entirely deferred from this project.
