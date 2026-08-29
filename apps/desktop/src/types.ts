export type ConnectionKind = "usb" | "wifi" | "mock";
export type DeviceStatus =
  | "disconnected"
  | "pairing"
  | "connected"
  | "preparing"
  | "ready"
  | "busy"
  | "error";

export type TileStreamState = "live" | "sampling" | "parked" | "stale" | "error";

const TILE_STREAM_LABELS: Record<TileStreamState, string> = {
  live: "Live",
  sampling: "Sampling",
  parked: "Parked",
  stale: "Stale",
  error: "Error",
};

export function tileStreamStateView(
  state: TileStreamState | undefined,
  hasFrame: boolean,
  hasError: boolean,
): { state: TileStreamState; label: string } {
  const resolved = state ?? (hasError ? "error" : hasFrame ? "live" : "parked");
  return { state: resolved, label: TILE_STREAM_LABELS[resolved] };
}

export type DevicePlatform = "ios" | "android";

export type HardwareKey =
  | "home"
  | "back"
  | "recents"
  | "volumeUp"
  | "volumeDown"
  | "power"
  | "notification";

const PLATFORM_OS_NAMES: Record<DevicePlatform, string> = {
  ios: "iOS",
  android: "Android",
};

/** "iOS 16.7.15" / "Android 15".
 *
 * Never guesses: an unrecognised platform yields the bare version, because
 * labelling an Android phone "iOS" is the exact bug this replaces. */
export function deviceOsLabel(device: Pick<DeviceInfo, "platform" | "osVersion">): string {
  const name: string | undefined = PLATFORM_OS_NAMES[device.platform];
  const version = device.osVersion ?? "";
  if (!name) return version;
  return version ? `${name} ${version}` : name;
}

/** The one label shared by the tile footer, the device row and the focus dock.
 *
 * One function because those three had already drifted into three different
 * strings — `iOS {v}`, `{model} · {v}`, `{model} · {v}` — and only one of them
 * said which OS. */
export function deviceModelOsLabel(
  device: Pick<DeviceInfo, "model" | "platform" | "osVersion">,
): string {
  const os = deviceOsLabel(device);
  return os ? `${device.model} · ${os}` : device.model;
}

/**
 * The line under a tile's bold name. When a phone reports no friendly name its `name` is
 * its model — an Android serial like "23021RAAEG" — so the full "model · OS" caption would
 * print that serial a *second* time right beneath the name and the tile reads as a cluttered
 * duplicate. In that case show only the OS ("Android 15"); otherwise the model still adds
 * information (an iPhone named "iPhone 8 (Global)" over model "iPhone10,1"), so keep it.
 */
export function deviceTileSubtitle(
  device: Pick<DeviceInfo, "name" | "model" | "platform" | "osVersion">,
): string {
  return device.name === device.model
    ? deviceOsLabel(device)
    : deviceModelOsLabel(device);
}

/// One phone a group action did not reach, and why.
export interface GroupInputSkip {
  udid: string;
  /// `DeviceBusy` when something else holds the phone, `ActionFailed` when the action itself
  /// did not work.
  code: string;
  /// Set for `DeviceBusy`: who is holding it. This is the field the operator can act on.
  currentOwner?: string | null;
  /// Set for `ActionFailed`.
  message?: string | null;
}

export interface GroupInputReport {
  completedUdids: string[];
  skipped: GroupInputSkip[];
}

/// Group-sync timing/offset policy (A1). Mirrors `riviu_core::group_sync`. All fields
/// optional — an absent policy (or `{}`) is the old lockstep behaviour.
export type DelayPolicy =
  | { mode: "none" }
  | { mode: "random"; minMs: number; maxMs: number }
  | { mode: "staggered"; stepMs: number };

export interface OffsetPolicy {
  /// Max absolute pixel jitter applied independently to x and y. 0 disables offset.
  maxPx: number;
}

export interface GroupSyncPolicy {
  delay?: DelayPolicy;
  offset?: OffsetPolicy;
}

export interface DeviceInfo {
  udid: string;
  name: string;
  model: string;
  platform: DevicePlatform;
  osVersion: string;
  connection: ConnectionKind;
  status: DeviceStatus;
  battery?: number | null;
  wdaReady: boolean;
  wdaExpiresAt?: string | null;
  streamUrl?: string | null;
  tileStreamState?: TileStreamState;
  lastError?: string | null;
}

export type StreamQuality = "low" | "medium" | "high" | "extra";

export interface StreamSettings {
  fps: number;
  gridQuality: StreamQuality;
  focusQuality: StreamQuality;
}

export type JobStatus = "queued" | "running" | "succeeded" | "failed" | "cancelled";
export type StepStatus = "pending" | "running" | "succeeded" | "failed" | "skipped";

export interface JobStepRecord {
  index: number;
  action: string;
  status: StepStatus;
  error?: string | null;
  artifactPath?: string | null;
}

export interface JobRecord {
  id: string;
  scriptName: string;
  udids: string[];
  status: JobStatus;
  createdAt: string;
  updatedAt: string;
  steps: JobStepRecord[];
  error?: string | null;
}

export interface AppleIdConfig {
  email: string;
  hasPassword: boolean;
}

export type AgentState =
  | "unknown"
  | "missing"
  | "repairRequired"
  | "starting"
  | "ready"
  | "error";

export interface AgentSettings {
  autoRepair: boolean;
}

export interface AgentStatus {
  udid: string;
  state: AgentState;
  artifactId: string;
  artifactVersion: string;
  bundleId: string;
  protocolVersion: number;
  features: string[];
  installedVersion: string | null;
  installedBuild: string | null;
  authReady: boolean;
  mjpegReady: boolean;
  sessionReady: boolean;
  message: string | null;
}

export interface AgentRuntimeView {
  settings: AgentSettings;
  tokenConfigured: boolean;
  activeArtifactId: string;
  activeArtifactVersion: string;
}

export type PageId =
  | "control"
  | "material"
  | "apps"
  | "scripts"
  | "jobs"
  | "publish"
  | "data"
  | "api"
  | "settings";

export interface DeviceMeta {
  udid: string;
  notes: string;
  tags: string[];
  groupId?: string | null;
  /** TikTok @handle this phone is logged into, without the leading `@`. Empty if unknown. */
  handle?: string;
  /**
   * What the operator calls this phone (xiaowei "Change Name"). Empty means "use the name
   * the phone reports" — this is a label in this app's records, never written to the device.
   */
  alias?: string;
  /**
   * The number written on the phone and on the shelf (xiaowei "Change Number"). `null` means
   * unnumbered, and the tile then shows its position in the grid instead — which is the very
   * thing a number replaces, since a position moves when the fleet list changes.
   */
  number?: number | null;
}

export interface DeviceGroup {
  id: string;
  name: string;
  color: string;
  udids: string[];
  createdAt: string;
}

export interface GroupInstallResult {
  udid: string;
  ok: boolean;
  error?: string;
}

export interface MaterialItem {
  id: string;
  name: string;
  path: string;
  kind: string;
  size: number;
  createdAt: string;
}

export interface AppLibraryItem {
  id: string;
  name: string;
  path: string;
  bundleId: string;
  version: string;
  createdAt: string;
}

export interface ScheduleItem {
  id: string;
  name: string;
  scriptName: string;
  udids: string[];
  everyMinutes: number;
  enabled: boolean;
  lastRunAt?: string | null;
  nextRunAt?: string | null;
  /// Why the last due tick enqueued nothing, or absent if it enqueued something.
  lastError?: string | null;
}

export type PublishCampaignState =
  | "queued"
  | "scheduled"
  | "preparing"
  | "ready"
  | "transferring"
  | "imported"
  | "posting"
  | "verifying"
  | "succeeded"
  | "failedBeforeDispatch"
  | "uncertain"
  | "cancelled"
  | "missed";

export interface PublishImage {
  path: string;
  fileName: string;
  order: number;
  sha256: string;
  byteLen: number;
  width: number;
  height: number;
}

export interface PublishBundle {
  id: string;
  sourcePath: string;
  name: string;
  mediaKind: "image";
  images: PublishImage[];
  captionPath: string;
  caption: string;
  captionSha256: string;
  totalBytes: number;
}

export interface PublishScanNotice {
  severity: "warning" | "error";
  path: string;
  message: string;
}

export interface PublishFolderManifest {
  sourceRoot: string;
  scannedAt: string;
  bundles: PublishBundle[];
  notices: PublishScanNotice[];
  ignoredPartnerFiles: number;
  ignoredHiddenFiles: number;
}

export interface PublishAssignmentPlan {
  bundleId: string;
  udid: string;
  ordinal: number;
}

export interface PublishCampaignRecord {
  id: string;
  requestId: string;
  sourceRoot: string;
  state: PublishCampaignState;
  runAt?: string | null;
  visibility: "public";
  cleanupPolicy: "deleteImportedAssetsAfterVerified";
  assignments: PublishAssignmentPlan[];
  createdAt: string;
  updatedAt: string;
  errorCode?: string | null;
}

export interface PublishAssignmentRecord {
  id: string;
  campaignId: string;
  bundleId: string;
  ordinal: number;
  udid: string;
  state: PublishCampaignState;
  effectIntent?: string | null;
  evidenceJson?: string | null;
  errorCode?: string | null;
}

export interface PublishEventRecord {
  revision: number;
  kind: string;
  payloadJson: string;
  createdAt: string;
}

export interface PublishCampaignDetail {
  campaign: PublishCampaignRecord;
  bundles: PublishBundle[];
  assignments: PublishAssignmentRecord[];
  events: PublishEventRecord[];
}

export interface OpLog {
  id: string;
  action: string;
  detail: string;
  createdAt: string;
}

export interface AnalyticsSummary {
  deviceTotal: number;
  deviceReady: number;
  jobsTotal: number;
  jobsSucceeded: number;
  jobsFailed: number;
  jobsRunning: number;
  scriptsTotal: number;
  materialsTotal: number;
  appsTotal: number;
  schedulesEnabled: number;
  recentLogs: OpLog[];
}

/**
 * The part of a window that overrides how a session behaves, when it overrides it at all.
 *
 * All five or none: the three rates share the panel's one 100% budget, and a budget assembled
 * from two sources is one nobody can read off the screen.
 */
export interface NurtureWindowBehaviour {
  numVideos: number;
  numRounds: number;
  likeProb: number;
  commentProb: number;
  followProb: number;
}

/**
 * One stretch of the local day the schedule may run in.
 *
 * Times are minutes from local midnight — the number the operator is thinking in when they
 * type `08:00`. `endMinute <= startMinute` wraps past midnight, which is how `22:00 - 02:00`
 * is written. `udids` empty means every connected phone, and the editor says so in words.
 */
export interface NurtureWindow {
  id: string;
  startMinute: number;
  endMinute: number;
  everyMinutes: number;
  durationMinutes: number;
  udids: string[];
  /** `null` means "behave like the panel above". */
  behaviour?: NurtureWindowBehaviour | null;
}

export interface NurtureSettings {
  baseUrl: string;
  model: string;
  /**
   * Never the real key on the way *out* of the backend.
   *
   * The key lives in the OS credential store, not in the settings row, and it is not handed to
   * this page: a load returns the sentinel `__riviu_keep_stored_key__` when one is configured,
   * and sending that same value back means "leave it alone". Anything else — including an
   * empty string — is taken literally, so the key can still be replaced or cleared.
   */
  apiKey: string;
  /** Whether a key is stored. The only thing the form can honestly show about it. */
  hasApiKey?: boolean;
  bundleId: string;
  numVideos: number;
  numRounds: number;
  likeProb: number;
  commentProb: number;
  followProb: number;
  frenzyProb: number;
  watchMin: number;
  watchMax: number;
  persona: string;
  fatigue: boolean;
  timeOfDay: boolean;
  pauseSwipe: boolean;
  nightStart: number;
  nightEnd: number;
  recoverDelayMin: number;
  recoverDelayMax: number;
  staggerDelayMin: number;
  staggerDelayMax: number;
  commentLang: string;
  aiDirections: string;
  maxCommentWords: number;
  scheduleEnabled: boolean;
  scheduleEveryMinutes: number;
  scheduleDurationMinutes: number;
  scheduleUdids: string[];
  /**
   * Stretches of the local day the schedule may run in, each with its own cadence.
   *
   * Optional because a settings row written before windows existed has no key for it, and
   * because **empty means the old all-day cadence** rather than "never runs" — see
   * `decide_single_cadence` in `nurture_schedule.rs`. Anything reading this has to treat
   * absent and empty the same way.
   */
  scheduleWindows?: NurtureWindow[];
  steadyMood?: string;

  // Per-feature switches. Separate from the probabilities so pausing a feature does not
  // destroy the tuned number — see `NurtureSettings` in `crates/core/src/types.rs`.
  // Optional here because a profile stored before they existed has no key for them, and
  // the Rust side defaults every one to `true`.
  likeEnabled?: boolean;
  commentEnabled?: boolean;
  followEnabled?: boolean;
  frenzyEnabled?: boolean;
  carouselEnabled?: boolean;
  /// Ceiling on slides paged through in one photo carousel.
  carouselMaxSlides?: number;
  /// How much of that ceiling to traverse, as a percentage. 100 = to the end.
  carouselPortionPercent?: number;
  /**
   * Whether the engine's own pacing may override the numbers above.
   *
   * Optional and read as `false` when absent, matching the Rust `#[serde(default)]`: a row
   * stored before this existed means the same thing as off. Off is the shipped default —
   * the ceilings it holds back are listed on the switch's own explanation.
   */
  humanLimits?: boolean;
}

/// Which settings a **running** session picks up on its next post.
///
/// The same split as `NurtureSettings::absorb_live_changes` on the Rust side, and it has to stay
/// the same split: this list is the declaration, that one is the behaviour.
///
/// **Read by two tests and by no component, and that is worth stating plainly** — an earlier
/// version of this comment said the badges read it, and they do not:
/// `NurturePopup` renders from `RESTART_REQUIRED_REASONS`, which is the other half of the same
/// split, written independently. So this constant looks unused from inside TypeScript while
/// `crates/core/src/types.rs` asserts on it via `include_str!` — delete it and `cargo test`
/// goes red from a change made entirely here. `nurtureLiveFields.test.ts` reads it from this
/// side too, both to make that visible and to keep the two lists from ever naming the same
/// field.
export const LIVE_TUNABLE_FIELDS = new Set<keyof NurtureSettings>([
  "likeProb",
  "commentProb",
  "followProb",
  "frenzyProb",
  "likeEnabled",
  "commentEnabled",
  "followEnabled",
  "frenzyEnabled",
  "watchMin",
  "watchMax",
  "fatigue",
  "timeOfDay",
  "pauseSwipe",
  "humanLimits",
  "nightStart",
  "nightEnd",
  "recoverDelayMin",
  "recoverDelayMax",
  "carouselEnabled",
  "carouselMaxSlides",
  "carouselPortionPercent",
  "baseUrl",
  "model",
  "apiKey",
  "commentLang",
  "aiDirections",
  "maxCommentWords",
]);

/// Fields a running session will **not** pick up, with the reason to show the operator.
///
/// Each reason is a fact about the session, not a policy: it built something out of the
/// value and cannot rebuild it mid-run.
export const RESTART_REQUIRED_REASONS = {
  numVideos: "Mục tiêu của phiên được tính lúc bắt đầu",
  numRounds: "Mục tiêu của phiên được tính lúc bắt đầu",
  persona: "Mô hình hành vi được dựng một lần từ persona",
  steadyMood: "Chu kỳ mood đã dựng xong",
  bundleId: "App đã mở rồi; trên Android package được resolve theo từng máy",
  staggerDelayMin: "Chỉ có tác dụng giữa các phiên",
  staggerDelayMax: "Chỉ có tác dụng giữa các phiên",
  scheduleEnabled: "Lịch tác động giữa các phiên",
  scheduleEveryMinutes: "Lịch tác động giữa các phiên",
  scheduleDurationMinutes: "Lịch tác động giữa các phiên",
  scheduleUdids: "Lịch tác động giữa các phiên",
} satisfies Partial<Record<keyof NurtureSettings, string>>;

/**
 * A field that needs a restart, i.e. one this map has a reason for.
 *
 * `satisfies` rather than a type annotation keeps the literal keys, so `RestartBadge` can
 * only be pointed at a field there is actually a sentence for — a badge on anything else is
 * a compile error instead of the word "undefined" in a tooltip.
 */
export type RestartRequiredField = keyof typeof RESTART_REQUIRED_REASONS;

export interface NurtureApiTestResult {
  udid: string;
  comment: string;
  caption: string | null;
  contextConfidence: number;
  relevance: number;
  evidenceSupport: number;
  frameSha256: string;
  model: string;
  baseUrlHost: string;
  evidenceMode: string;
  /** How many *different* frames the picture carried — `1` on a still card, `0` on OCR. */
  distinctFrames: number;
  promptTokens: number;
  completionTokens: number;
}

export interface NurtureCommentAttempt {
  id: string;
  udid: string;
  outcome: string;
  source: string;
  model: string;
  baseUrlHost: string;
  promptTokens: number;
  completionTokens: number;
  /**
   * What the gateway said this attempt cost, in USD. `null` means it did not say, which
   * is not the same as free -- the column is nullable so the two can never be confused.
   */
  costUsd?: number;
  preview: string;
  captionPreview: string;
  frameSha256: string;
  contextConfidence?: number;
  relevance?: number;
  evidenceSupport?: number;
  /**
   * How many *different* frames the model was shown: `1` on a photo post, where the three
   * samples were one byte-identical picture, `0` on the caption-only path, which sends no
   * picture, and `null` on rows written before this was recorded.
   *
   * Read `evidenceSupport` next to this. Low with `1` means there was one frame of evidence;
   * low with `3` means the model saw three and still could not ground the comment.
   */
  distinctFrames?: number;
  /**
   * Slides the traversal paged before the comment was written, duplicates included. `0` on a
   * post that was never paged; `undefined` on rows from before it was recorded.
   *
   * The pair is what says anything: `7` slides with `distinctFrames: 1` means the pager turned
   * seven times and the stream handed back one picture every time.
   */
  carouselSlides?: number;
  createdAt: string;
}

/// Where one device is in its session — the same enum as Rust's `NurturePhase`.
///
/// Exists because a bar drawn from `videosDone` alone reads 0% for the first minute of a
/// perfectly healthy run (up to 40s waiting for TikTok to reach the foreground, then up to
/// 30s waiting for the feed) and reads exactly the same 0% for a phone that never opened the
/// app at all. The two lock-screen phones on 23/08/2026 died inside that window.
export type NurturePhase =
  | "queued"
  | "opening"
  | "awaitingFeed"
  | "watching"
  | "recovering"
  | "finished";

/// How a session ended. Mirrors Rust's `Outcome`.
///
/// This used to be stringified into the first token of a Vietnamese summary sentence and
/// then dropped, so the panel could not tell a phone that finished 47 videos from one that
/// failed to open the app, and rendered both as the same grey row.
export type NurtureOutcome = "done" | "partial" | "failed" | "stopped";

export interface NurtureSessionStatus {
  udid: string;
  running: boolean;
  videosDone: number;
  swipeAttempts: number;
  likeAttempts: number;
  commentAttempts: number;
  followAttempts: number;
  likes: number;
  comments: number;
  follows: number;
  lastMessage: string;
  /// What the comment model reported spending on this device, in tokens.
  ///
  /// **Tokens and not money, because money was fabricated.** This was `sessionUsd`: the
  /// product of two hand-typed per-million prices that were never sent to the API and
  /// existed in three different values at once, with no UI able to edit them. Tokens come
  /// from the API's own `usage`, so they are true of whatever model is configured. Multiply
  /// by the provider's real rate outside the app.
  sessionPromptTokens: number;
  sessionCompletionTokens: number;
  /// Which run this row belongs to. `null` on a row from before run ids existed.
  ///
  /// Load-bearing for any fleet total: the status list is keyed by udid and never pruned,
  /// so it accumulates every phone that has run since the app started. Summing over it
  /// without filtering counts finished phones from earlier runs, and restarting one phone
  /// makes an overall bar go *backwards* — that row's counters reset while the others keep
  /// their finished values.
  runId: string | null;
  /// How many devices were started together in this run — the denominator for an overall
  /// bar. Must be this rather than the number of rows present: a phone that failed before
  /// producing a second status still occupies a slot.
  runSize: number;
  phase: NurturePhase;
  outcome: NurtureOutcome | null;
  /// Posts this session is aiming for, snapshotted at start.
  ///
  /// Never recompute this from the settings form: `numVideos` is a RESTART-required field,
  /// so dividing by the live value rescales the bar under a session that never changed —
  /// lower it from 120 to 15 mid-run and the bar reads 800%.
  videoTarget: number;
  /// ISO timestamp: when this device's session began, after its stagger.
  startedAt: string | null;
  /// ISO timestamp: when the wall clock ends this session regardless of the video count.
  ///
  /// A run ends at **whichever bound arrives first**, and for a manual start this is a
  /// randomised 2–3 hour horizon. A bar drawn from the video count alone stalls at 40% on a
  /// run that is minutes from finishing on time, and reads as hung.
  deadlineAt: string | null;
}

/// One line a phone said, with the moment it first said it.
///
/// `repeats` is why the ring is readable at all: a session polling for the feed emits the
/// same sentence every second, and collapsing those into a count is what keeps the line
/// before them from being pushed out. Render it as `×N`, and prefer `at` over `lastAt` —
/// "stuck here since 14:22" is the reading that helps.
export interface SessionLogEntry {
  at: string;
  lastAt: string;
  text: string;
  repeats: number;
}

/// One phone that has history, for building the row list.
///
/// Needed because the idle sweep writes lines for phones that never ran a session, so
/// "which phones have anything to show" cannot be answered from the status list.
export interface SessionLogSummary {
  udid: string;
  lines: number;
  last?: SessionLogEntry | null;
}

export interface NurtureCostSummary {
  /// Tokens summed over **every** attempt, sent or rejected — a comment the verification
  /// gate threw away still burned up to four API calls, and recording that as free is how
  /// the most expensive failure mode became invisible.
  todayPromptTokens: number;
  todayCompletionTokens: number;
  totalPromptTokens: number;
  totalCompletionTokens: number;
  /// Comments actually sent.
  todayComments: number;
  totalComments: number;
}

export type TikTokPostKind = "video" | "photo";
export type LinkErrorCode =
  | "empty"
  | "invalidUrl"
  | "unsupportedScheme"
  | "unsupportedHost"
  | "userInfoNotAllowed"
  | "customPortNotAllowed"
  | "unsupportedTargetKind"
  | "unresolvedShortLink";

export interface ResolvedTikTokTarget {
  originalUrl: string;
  normalizedUrl: string;
  targetKey: string;
  contentId: string;
  author: string;
  kind: TikTokPostKind;
}

export interface TikTokLinkLine {
  lineNo: number;
  original: string;
  target: ResolvedTikTokTarget | null;
  error: LinkErrorCode | null;
}

/** Reply chain, or independent top-level comments from each account. */
export type ThreadMode = "threaded" | "standalone";

/** Chain: message N answers N-1. Star: every message answers message 0. */
export type ThreadShape = "chain" | "star";

export interface ThreadCampaignRequest {
  requestId: string;
  targets: ResolvedTikTokTarget[];
  actorUdids: string[];
  messageCount: number;
  instruction: string;
  maxWords: number;
  mode: ThreadMode;
  /**
   * Chain or star, and only read in `threaded` mode.
   *
   * Optional so a caller that never sets it keeps the chain, matching the Rust
   * `#[serde(default)]`. Star is the shape that lets a run go parallel: every reply
   * answers message 0, so they no longer have to wait for each other.
   */
  shape?: ThreadShape;
  /**
   * Split the actors into teams of this size, each team taking its own links.
   *
   * Absent means one team holding every actor — the whole selection working the same
   * link, one phone at a time. The remainder is spread rather than left idle, so twenty
   * phones at three become 4,4,3,3,3,3.
   */
  cohortSize?: number;
  /**
   * Comments written by the operator, used instead of the AI when non-empty.
   *
   * Optional so a caller that never sets it keeps the AI behaviour, matching the Rust
   * `#[serde(default)]`. The backend deals them out across (target, ordinal).
   */
  manualComments?: string[];
  /** Also like each target, once per actor that comments on it. */
  likeTarget?: boolean;
  /**
   * @-handles (without the leading `@`) tagged at the front of each thread's opening
   * comment, as plain text. A handle that belongs to a fleet phone is also added to
   * `actorUdids` by the caller so that phone joins the post and replies; a handle matching
   * no phone is tagged in text only. Optional/empty prepends nothing (Rust `#[serde(default)]`).
   */
  mentions?: string[];
  /** Each reply tags the account it answers; ignored for `standalone`. */
  mentionParent?: boolean;
}

export type ThreadMessageState =
  | "queued"
  | "preparing"
  | "ready"
  | "sending"
  | "succeeded"
  | "failed"
  | "uncertain"
  | "skippedParent";
export type ThreadCampaignState =
  | "queued"
  | "running"
  | "succeeded"
  | "partial"
  | "failed"
  | "cancelled";

/**
 * Numbers the operator wants a post to reach. `null` means "leave this one alone".
 *
 * Three different problems, not three of the same one. A like is one per account and an account
 * cannot like twice, so a like target above the number of accounts that have not liked yet is
 * unreachable however long it runs. A comment can be repeated, so its ceiling is taste rather
 * than arithmetic. A view accumulates across passes — measured 24/08/2026 — so a view target is a
 * schedule.
 */
export interface PostTargets {
  views: number | null;
  likes: number | null;
  comments: number | null;
}

/** Where the post is now. `null` for a number this build or this screen could not state. */
export interface PostNow {
  views: number | null;
  likes: number | null;
  comments: number | null;
}

/** One metric's verdict, before anything runs. */
export interface MetricPlan {
  /** How far short the post is; `0` when it is already there. */
  shortfall: number;
  /** The most this fleet could add, or `null` when nothing bounds it but time. */
  ceiling: number | null;
  /** Passes of the whole fleet needed, when that is a meaningful number. */
  passes: number | null;
  /** Why it cannot be reached, in the operator's language. `null` means it can. */
  unreachable: string | null;
}

/** The whole plan, one entry per metric asked for. */
export interface ThresholdPlan {
  views: MetricPlan | null;
  likes: MetricPlan | null;
  comments: MetricPlan | null;
}

/** What one phone read off a post, and what the targets would take. */
export interface InteractionPostReading {
  now: PostNow;
  plan: ThresholdPlan;
  /** Whether the view count was asked for — it is the slow half. */
  viewsRead: boolean;
}

export interface InteractionCampaignSummary {
  id: string;
  requestId: string;
  state: ThreadCampaignState;
  messageCount: number;
  targetCount: number;
  succeededMessages: number;
  failedMessages: number;
  /** Why the campaign ended, when something ended it. Rendered — see InteractionPopup. */
  errorCode: string | null;
  updatedAt: string;
  /**
   * What the campaign was, read back out of its stored request.
   *
   * Null when the stored request will not parse. The list used to name a row with a slice of
   * its UUID, so runs against different posts were indistinguishable.
   */
  brief: InteractionCampaignBrief | null;
}

export interface InteractionCampaignBrief {
  firstAuthor: string | null;
  firstContentId: string | null;
  mode: ThreadMode;
  shape: ThreadShape;
  cohortSize: number | null;
  actorCount: number;
  /** The operator wrote the comments rather than the AI. */
  manual: boolean;
  likeTarget: boolean;
}

export interface InteractionAssignmentRecord {
  id: string;
  targetKey: string;
  ordinal: number;
  actorUdid: string;
  parentAssignmentId: string | null;
  state: ThreadMessageState;
  preparedText: string | null;
  errorCode: string | null;
  /**
   * What happened to the like on this message, when the campaign asked for one.
   *
   * Separate from `errorCode` on purpose: a like that fails must not cost the comment, so
   * a message that posted is `succeeded` and this is a note beside it — not a failure. It
   * used to go only to the log, which meant a refused like was invisible.
   */
  like?: string | null;
  /**
   * What happened to the `@` tags on this message, when the campaign asked for any.
   *
   * A tag only becomes a real mention if the driver could pick it out of TikTok's own
   * suggestion list; one that was merely typed posts as grey text and notifies nobody. The
   * comment itself looks the same either way, which is why this is reported separately.
   */
  mention?: string | null;
}

export interface InteractionCampaignDetail {
  summary: InteractionCampaignSummary;
  assignments: InteractionAssignmentRecord[];
}

/**
 * What the desktop learned about one Interaction target from outside the phones.
 *
 * Read by `InteractionTargetNotesTab`. It exists because of the trap in AGENTS.md 9.103 §4:
 * `nurture_list_comment_attempts` was registered and allowlisted for weeks while nothing here
 * ever called it, so the numbers it produced could not be checked by anyone. The web lookup
 * files its findings against every target; without this they were the same dead end.
 *
 * Mirrored field for field by `interaction::target_note_tests::the_frontend_mirrors_this_note_field_for_field`
 * — the repo's wire-parity test only scans `types.rs`, so this one is pinned by its own test.
 */
export interface InteractionTargetNote {
  targetKey: string;
  lineNo: number;
  normalizedUrl: string;
  kind: TikTokPostKind;
  /** Characters of caption fetched. The measurement: the phone's tree truncates, the web does not. */
  captionChars: number | null;
  captionPreview: string | null;
  durationSecs: number | null;
  /** Slides the post has. `null` for a video, and for a lookup that produced nothing. */
  slideCount: number | null;
  /** `false` is why no transcript was asked for: the post carries music, not speech. */
  hasOriginalAudio: boolean | null;
  subtitleLangs: string[];
  /** `vie-VN/ASR` is the original speech; `eng-US/MT` a machine translation of it. */
  transcriptTrack: string | null;
  /** `ip_blocked` | `post_unavailable` | `transient` | `no_ytdlp`, when the lookup was refused. */
  errorCode: string | null;
  errorDetail: string | null;
}

export interface ThreadPlanAssignment {
  targetKey: string;
  ordinal: number;
  actorUdid: string;
  parentOrdinal: number | null;
  /**
   * Which team runs this message.
   *
   * The planner has emitted it since cohorts landed and this type never declared it, so the
   * one field that says which conversation a row belongs to arrived as `undefined` on the
   * desktop. Pinned on the Rust side by `the_preview_wire_shape_is_what_the_frontend_types_say`.
   */
  cohort: number;
}

export interface ThreadPlan {
  requestId: string;
  assignments: ThreadPlanAssignment[];
}

export interface ThreadPreview {
  lines: TikTokLinkLine[];
  plan: ThreadPlan | null;
  validTargetCount: number;
  /** Teams this plan would run at once — the backend's own `partition_actors`, not a copy. */
  cohortCount: number;
  /**
   * Device streams the app can hold open at once.
   *
   * Worth warning about before starting, because exceeding it is a refusal and not a queue:
   * the cohorts past the limit fail rather than wait.
   */
  streamCapacity: number;
}

export type JsonValue = string | number | boolean | null | JsonObject | JsonValue[];

export interface JsonObject {
  [key: string]: JsonValue;
}

export type ScreenOrientation =
  | "portrait"
  | "portraitUpsideDown"
  | "landscapeLeft"
  | "landscapeRight";

export type ActionKind =
  | "start"
  | "end"
  | "launchApp"
  | "terminateApp"
  | "wait"
  | "tap"
  | "swipe"
  | "typeText"
  | "screenshot"
  | "home"
  | "assertVisible"
  | "tapVision"
  | "ifVision"
  | "rawHttp"
  | "rawWda"
  | "shell";

export type ActionCategory = "control" | "app" | "input" | "timing" | "evidence";
export type ResourceClass = "pureDesktop" | "bridge" | "uiSession" | "uiWithStream";
export type SideEffectClass = "none" | "idempotentSet" | "ambiguousUi" | "artifactWrite";
export type EvidenceRequirement =
  | "none"
  | "activeApp"
  | "process"
  | "frame"
  | "textOrQualifiedFrame"
  | "artifact";
export type EvidenceKind =
  | "activeAppEquals"
  | "processAbsent"
  | "frameDigestChanged"
  | "frameRegionChanged"
  | "qualifiedFramePredicate"
  | "accessibilityVisible"
  | "textReadBackEquals"
  | "artifactDecodedAndHashed";
export type ReconciliationPolicy =
  | "none"
  | "readActiveApp"
  | "readProcess"
  | "readFrame"
  | "readText"
  | "readArtifact";
export type RetryPolicy = "never" | "beforeDispatchOnly" | "idempotentAfterRead";

export interface PortDefinition {
  name: string;
  valueType: string;
  required: boolean;
}

export interface ActionDefinition {
  kind: ActionKind;
  schemaVersion: number;
  label: string;
  disabledReason: string | null;
  category: ActionCategory;
  configSchema: JsonValue;
  inputPorts: PortDefinition[];
  outputPorts: PortDefinition[];
  requiredCapabilities: string[];
  resourceClass: ResourceClass;
  sideEffectClass: SideEffectClass;
  evidenceRequirement: EvidenceRequirement;
  allowedEvidence: EvidenceKind[];
  qualifiedDetectorIds: string[];
  reconciliationPolicy: ReconciliationPolicy;
  defaultTimeoutMs: number;
  retryPolicy: RetryPolicy;
}

export interface CanvasPoint {
  x: number;
  y: number;
}

export interface FlowViewport {
  x: number;
  y: number;
  zoom: number;
}

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

export type EvidenceSpec =
  | { kind: "activeAppEquals"; bundleId: string }
  | { kind: "processAbsent"; bundleId: string }
  | { kind: "frameDigestChanged"; minimumDistance: number }
  | {
      kind: "frameRegionChanged";
      x: number;
      y: number;
      width: number;
      height: number;
      minimumDistance: number;
    }
  | { kind: "qualifiedFramePredicate"; detectorId: string }
  | { kind: "accessibilityVisible"; accessibilityId: string }
  | { kind: "textReadBackEquals"; locator: QualifiedElementLocator; value: string }
  | { kind: "artifactDecodedAndHashed" };

export interface FlowNode {
  id: string;
  kind: ActionKind;
  position: CanvasPoint;
  config: JsonObject;
  postcondition?: EvidenceSpec | null;
}

export interface FlowEdge {
  id: string;
  sourceNodeId: string;
  sourcePort: string;
  targetNodeId: string;
  targetPort: string;
}

export interface FlowDocumentV2 {
  schemaVersion: 2;
  id: string;
  name: string;
  revision: number;
  entryNodeId: string;
  nodes: FlowNode[];
  edges: FlowEdge[];
  viewport: FlowViewport;
}

export type CompiledTapTarget =
  | { mode: "point"; target: ImageCoordinateTarget }
  | { mode: "accessibilityId"; value: string };

export type CompiledActionConfig =
  | { kind: "empty" }
  | { kind: "launchApp"; bundleId: string }
  | { kind: "terminateApp"; bundleId: string }
  | { kind: "wait"; durationMs: number }
  | { kind: "tap"; target: CompiledTapTarget }
  | { kind: "swipe"; from: ImageCoordinateTarget; to: ImageCoordinateTarget; durationMs: number }
  | { kind: "typeText"; text: string; readBackLocator: QualifiedElementLocator }
  | { kind: "screenshot"; label: string; format: string }
  | { kind: "assertVisible"; accessibilityId: string }
  | {
      kind: "tapVision";
      templatePngBase64: string;
      threshold: number;
      region: VisionRegion | null;
    }
  | {
      kind: "ifVision";
      templatePngBase64: string;
      threshold: number;
      region: VisionRegion | null;
    };

export interface VisionRegion {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export interface CompiledFlowNode {
  id: string;
  kind: ActionKind;
  config: CompiledActionConfig;
  postcondition: EvidenceSpec | null;
}

export interface FlowContextPlan {
  requiresExclusive: boolean;
  requiresUiSession: boolean;
  requiresStream: boolean;
  requiresFreshTextSession: boolean;
  initialBundleId: string | null;
}

export interface CompiledFlowPlanV2 {
  schemaVersion: 2;
  flowId: string;
  revision: number;
  nodes: Record<string, CompiledFlowNode>;
  executionOrder: string[];
  /**
   * Per-node adjacency keyed by output port (`flow`, or `matched`/`notMatched` for `ifVision`).
   * Absent for legacy plans compiled before branching existed, which is why Rust marks it
   * `skip_serializing_if` — their canonical JSON, and so their frozen plan hash, must not change.
   */
  successors?: Record<string, Record<string, string>>;
  contextPlan: FlowContextPlan;
  actionDefinitionVersions: Partial<Record<ActionKind, number>>;
  requiredCapabilities: string[];
}

export interface CompiledRevision {
  plan: CompiledFlowPlanV2;
  canonicalJson: string;
  sha256: string;
}

export interface FlowSummary {
  id: string;
  name: string;
  latestRevision: number;
  archived: boolean;
  updatedAt: string;
}

export interface FlowRevisionRecord {
  document: FlowDocumentV2;
  compiledPlan: CompiledFlowPlanV2;
  planHash: string;
  createdAt: string;
}

export type DeviceWorkOwner =
  | "nurture"
  | "interaction"
  | "script"
  | "repair"
  | "manualControl"
  | "groupSync"
  | "idleSweep";

export interface CommandError {
  code: string;
  message: string;
  nodeId?: string;
  field?: string;
  udid?: string;
  attemptId?: string;
  requestedOwner?: DeviceWorkOwner;
  currentOwner?: DeviceWorkOwner;
}

export type FlowValidationIssue = CommandError;

export interface LegacyImportDiagnostic {
  stepIndex: number;
  code: string;
  message: string;
  field: string | null;
}

export interface LegacyImportResult {
  document: FlowDocumentV2 | null;
  diagnostics: LegacyImportDiagnostic[];
}

export type FlowTargetSelection =
  | { mode: "one"; udid: string }
  | { mode: "selected"; udids: string[] }
  | { mode: "allEligible" };

export interface FlowSelectionSnapshot {
  requested: FlowTargetSelection;
  targetUdids: string[];
}

export type FlowAttemptState =
  | "queued"
  | "intentCommitted"
  | "effectDispatched"
  | "verifying"
  | "succeeded"
  | "failedBeforeDispatch"
  | "failedVerified"
  | "uncertain"
  | "cancelled"
  | "interrupted";

export type FlowAggregateState =
  | "queued"
  | "running"
  | "succeeded"
  | "partial"
  | "failed"
  | "cancelled";

export type FlowDeviceRunState =
  | "queued"
  | "preflight"
  | "running"
  | "succeeded"
  | "failed"
  | "skipped"
  | "cancelled";

export interface FlowErrorRecord {
  code: string;
  message: string;
  nodeId: string | null;
  field: string | null;
  udid: string | null;
  attemptId: string | null;
}

export type ActiveTransport = "legacyUsbmuxTransport" | "rsdTransport" | "mock";

export interface QualifiedGeometry {
  logicalWidth: number;
  logicalHeight: number;
  pixelWidth: number;
  pixelHeight: number;
  scaleX: number;
  scaleY: number;
  orientation: ScreenOrientation;
}

export interface InstalledAgentIdentity {
  bundleId: string;
  version: string;
  build: string;
  executableName: string;
  signerIdentitySha256: string;
}

export interface InstalledTargetIdentity {
  bundleId: string;
  version: string;
  build: string;
}

export interface DeviceCapabilitySnapshot {
  installedAgent: InstalledAgentIdentity;
  selectedArtifactSha256: string;
  agentVersion: string;
  protocolVersion: number;
  driverAdapterVersion: string;
  transport: ActiveTransport;
  productType: string;
  /** Stays `iosVersion` on purpose, unlike `DeviceInfo.osVersion`.
   *
   * This mirror follows the **wire**, not the Rust field name. The Rust side is
   * `DeviceCapabilitySnapshot::os_version` with `#[serde(rename = "iosVersion")]`,
   * frozen because the key is persisted under `deny_unknown_fields` and is hash
   * material for a stored `profile_id`. See `crates/core/src/device_capabilities.rs`. */
  iosVersion: string;
  targetApp: InstalledTargetIdentity;
  protectedAuthReady: boolean;
  geometry: QualifiedGeometry | null;
}

export type FlowPreflightScope =
  | { kind: "targetFree" }
  | { kind: "targetQualified"; bundleId: string };

export interface FlowCapabilitySnapshot {
  scope: FlowPreflightScope;
  device: DeviceCapabilitySnapshot | null;
  agentStatus: AgentStatus | null;
  capabilityIds: string[];
}

export interface FlowContextReleaseProof {
  udid: string;
  owner: DeviceWorkOwner;
  hadSession: boolean;
  hadStream: boolean;
}

export interface FlowRunRecord {
  id: string;
  flowId: string;
  flowRevision: number;
  planSha256: string;
  selection: FlowSelectionSnapshot;
  state: FlowAggregateState;
  eventRevision: number;
  error: FlowErrorRecord | null;
  createdAt: string;
  updatedAt: string;
}

export interface FlowDeviceRunRecord {
  id: string;
  runId: string;
  udid: string;
  state: FlowDeviceRunState;
  capabilitySnapshot: FlowCapabilitySnapshot | null;
  releaseProof: FlowContextReleaseProof | null;
  error: FlowErrorRecord | null;
  startedAt: string | null;
  finishedAt: string | null;
}

export interface FlowNodeAttemptRecord {
  id: string;
  deviceRunId: string;
  nodeId: string;
  actionKind: ActionKind;
  attemptNo: number;
  sideEffectClass: SideEffectClass;
  state: FlowAttemptState;
  canonicalInput: JsonValue | null;
  evidenceBaseline: JsonValue | null;
  evidenceResult: JsonValue | null;
  /**
   * For an `ifVision` node, the output port the runtime match selected. Absent for every other
   * kind and for attempts recorded before branching existed.
   */
  chosenPort?: string;
  retryAllowed: boolean;
  error: FlowErrorRecord | null;
  startedAt: string | null;
  updatedAt: string;
  finishedAt: string | null;
}

export interface FlowArtifactRecord {
  id: string;
  attemptId: string;
  relativePath: string;
  label: string;
  kind: string;
  size: number;
  sha256: string;
  createdAt: string;
}

export interface FlowArtifactPayload {
  artifactId: string;
  label: string;
  kind: string;
  size: number;
  sha256: string;
  base64: string;
}

export interface FlowRunDetail {
  run: FlowRunRecord;
  deviceRuns: FlowDeviceRunRecord[];
  attempts: FlowNodeAttemptRecord[];
  artifacts: FlowArtifactRecord[];
}

export interface RevisionConflict {
  expected: number;
  actual: number;
}

/**
 * What an update check found, plus whether acting on it now would cut work off.
 *
 * `busyReason` is a sentence rather than a boolean so the operator is told *what* is
 * running. The backend re-reads it at install time regardless of what this said.
 */
export interface UpdateStatus {
  available: boolean;
  version: string | null;
  current: string;
  busyReason: string | null;
}

/** Whether an app came with the phone or was installed onto it. */
export type InstalledAppKind = "user" | "system";

/**
 * One app present on a phone, as the phone itself reports it.
 *
 * `label` is null on Android and always will be over adb: the phone returns the name as
 * a resource id needing the APK's resource table plus the device locale, neither phone
 * has aapt, and pulling a 261 MB APK to read one string is not a trade worth making. So
 * null means "this phone cannot tell us", never "unnamed" — render the bundle id.
 */
export interface InstalledApp {
  bundleId: string;
  kind: InstalledAppKind;
  label: string | null;
  /**
   * The app's icon as a base64 PNG (48 px edge), from the on-device helper.
   *
   * Absent for a phone with no helper, for the system partition (which the driver does not
   * pay to describe — see `name_apps_with_helper`), and for the handful of packages that
   * genuinely have no icon. Never a placeholder: the UI draws its own neutral square, so
   * "no icon" cannot be mistaken for "this is what the app looks like".
   */
  iconPngBase64?: string | null;
}

/**
 * What one operator-typed shell command produced.
 *
 * All three fields, because a non-zero exit is a normal answer here: `ls` on a missing
 * path, `grep` with no match and `dumpsys` on an unknown service all exit non-zero and
 * put the useful text on stderr. Showing only stdout would hide the answer.
 */
export interface ShellOutcome {
  exitCode: number;
  stdout: string;
  stderr: string;
}

/**
 * What one row of a phone's directory listing is.
 *
 * `other` is a real answer — a socket, a fifo, a block device — and not a parse failure.
 * A browser that dropped rows it did not recognise would show a folder as emptier than it
 * is, which is the worst kind of wrong for a file manager.
 */
export type DeviceFileKind = "file" | "directory" | "symlink" | "other";

/**
 * One entry in a phone's own directory listing (xiaowei "Preview Mobile Files").
 *
 * `modified` is the phone's own `YYYY-MM-DD HH:MM` text, not a parsed date: `ls` prints in
 * the *device's* timezone with no offset, so turning it into a Date here would invent a
 * precision the source does not have. `null` means the phone printed `?` — it could not
 * stat the row, which happens on dangling symlinks.
 *
 * `size` is meaningful for files. For a directory it is the inode size (3452 on this
 * fleet's sdcard), which says nothing about what is inside, so the UI does not show it.
 */
export interface DeviceFileEntry {
  name: string;
  kind: DeviceFileKind;
  size: number;
  modified: string | null;
  linkTarget: string | null;
}

/**
 * One directory as the phone described it, including what it would not describe.
 *
 * `incomplete` is non-null when `ls -la` printed some rows and complained about the rest, or
 * when a row was in a shape the parser could not read. The list is then **short**, and drawing
 * it as complete is the defect this field exists to stop: an operator deletes from a folder,
 * exports from it, and concludes things about it.
 *
 * A refusal never arrives here — the command rejects instead, because there is no listing to
 * draw. So `entries: []` with `incomplete: null` means the directory really is empty.
 */
/**
 * The two separate answers to "is this phone rooted".
 *
 * Nine of this fleet's twenty phones run their adb shell as uid 0 with **no `su` binary**, so
 * `shellIsRoot` is true and `hasSu` is false — and `factory_reset` is gated on `hasSu`. A
 * single "rooted" flag had to pick one meaning and mislead about the other, which is why the
 * root panel shows both.
 */
export interface DeviceRootStatus {
  hasSu: boolean;
  shellIsRoot: boolean;
}

export interface DeviceDirListing {
  entries: DeviceFileEntry[];
  incomplete: string | null;
}

/** What the phone had on its clipboard (xiaowei "Export Clipboard"). */
export interface ClipboardRead {
  /** The phone's own MIME description, e.g. `text/plain`. */
  contentType: string;
  /** Decoded as text. Empty for non-text content, where `bytes` still says how much. */
  text: string;
  bytes: number;
}

/**
 * Everything that arrives on `riviu://event`.
 *
 * Mirrors `AppEvent` in `crates/core/src/events.rs`, and the mirror is checked:
 * `the_event_union_matches_the_variants_this_enum_sends` reads the `type` literals out of
 * this file and compares them to the enum's variants, so renaming one half fails the build
 * on the other. Before the union existed, six subscribers each narrowed `unknown` with their
 * own `as` cast, and three of them were narrowing to field names the wire never sent.
 */
export type AppEvent =
  | { type: "devicesUpdated"; devices: DeviceInfo[] }
  | { type: "deviceUpdated"; device: DeviceInfo }
  | { type: "jobUpdated"; job: JobRecord }
  | { type: "flowUpdated"; flowId: string; revision: number }
  | { type: "flowRunUpdated"; runId: string; revision: number }
  | { type: "interactionUpdated"; campaignId: string; revision: number }
  | { type: "publishUpdated"; campaignId: string; revision: number }
  | { type: "wdaExpiryWarning"; udid: string; daysRemaining: number }
  | { type: "nurtureStatus"; status: NurtureSessionStatus };

/**
 * Narrow an untyped payload off the Tauri channel.
 *
 * The channel itself is untyped, so something has to make the first assertion. Doing it once
 * here — where the `type` tag is actually checked against the known set — is the difference
 * between one unchecked cast and one per subscriber.
 */
export function asAppEvent(payload: unknown): AppEvent | null {
  if (typeof payload !== "object" || payload === null) return null;
  const tag = (payload as { type?: unknown }).type;
  return typeof tag === "string" && (APP_EVENT_TYPES as readonly string[]).includes(tag)
    ? (payload as AppEvent)
    : null;
}

/** The tag of every `AppEvent`. Exported so the Rust-side pin can read it. */
export const APP_EVENT_TYPES = [
  "devicesUpdated",
  "deviceUpdated",
  "jobUpdated",
  "flowUpdated",
  "flowRunUpdated",
  "interactionUpdated",
  "publishUpdated",
  "wdaExpiryWarning",
  "nurtureStatus",
] as const;
