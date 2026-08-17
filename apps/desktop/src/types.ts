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

export function markDeviceFrameLive(devices: DeviceInfo[], udid: string): DeviceInfo[] {
  let changed = false;
  const next = devices.map((device) => {
    if (device.udid !== udid || device.tileStreamState === "live") return device;
    changed = true;
    return { ...device, tileStreamState: "live" as const };
  });
  return changed ? next : devices;
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

/** @deprecated use PageId */
export type TabId = PageId;

export interface DeviceMeta {
  udid: string;
  notes: string;
  tags: string[];
  groupId?: string | null;
  proxyId?: string | null;
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

export interface ProxyConfig {
  id: string;
  name: string;
  proxyType: string;
  host: string;
  port: number;
  username: string;
  password: string;
  notes: string;
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

export interface PublishTask {
  id: string;
  name: string;
  scriptName: string;
  materialIds: string[];
  udids: string[];
  status: string;
  createdAt: string;
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

export interface NurtureSettings {
  baseUrl: string;
  model: string;
  apiKey: string;
  inputPricePer1m: number;
  outputPricePer1m: number;
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
/// The same split as `NurtureSettings::absorb_live_changes` on the Rust side, and it has to
/// stay the same split: this list is what the UI promises, that one is what the loop does.
/// Anything not here needs the session stopped and started again, and the form says so
/// rather than letting the operator wonder why a change did nothing.
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
  "inputPricePer1m",
  "outputPricePer1m",
  "commentLang",
  "aiDirections",
  "maxCommentWords",
]);

/// Fields a running session will **not** pick up, with the reason to show the operator.
///
/// Each reason is a fact about the session, not a policy: it built something out of the
/// value and cannot rebuild it mid-run.
export const RESTART_REQUIRED_REASONS: Partial<Record<keyof NurtureSettings, string>> = {
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
};

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
  promptTokens: number;
  completionTokens: number;
  usd: number;
}

export interface NurtureCommentCost {
  id: string;
  udid: string;
  model: string;
  baseUrlHost: string;
  promptTokens: number;
  completionTokens: number;
  usd: number;
  preview: string;
  createdAt: string;
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
  usd: number;
  preview: string;
  captionPreview: string;
  frameSha256: string;
  contextConfidence?: number;
  relevance?: number;
  evidenceSupport?: number;
  createdAt: string;
}

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
  sessionUsd: number;
}

export interface NurtureCostSummary {
  todayUsd: number;
  totalUsd: number;
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

export interface ThreadCampaignRequest {
  requestId: string;
  targets: ResolvedTikTokTarget[];
  actorUdids: string[];
  messageCount: number;
  instruction: string;
  maxWords: number;
  mode: ThreadMode;
  /**
   * Comments written by the operator, used instead of the AI when non-empty.
   *
   * Optional so a caller that never sets it keeps the AI behaviour, matching the Rust
   * `#[serde(default)]`. The backend deals them out across (target, ordinal).
   */
  manualComments?: string[];
  /** Also like each target, once per actor that comments on it. */
  likeTarget?: boolean;
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
}

export interface InteractionCampaignDetail {
  summary: InteractionCampaignSummary;
  assignments: InteractionAssignmentRecord[];
}

export interface ThreadPlanAssignment {
  targetKey: string;
  ordinal: number;
  actorUdid: string;
  parentOrdinal: number | null;
}

export interface ThreadPlan {
  requestId: string;
  assignments: ThreadPlanAssignment[];
}

export interface ThreadPreview {
  lines: TikTokLinkLine[];
  plan: ThreadPlan | null;
  validTargetCount: number;
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

export type FlowPortDefinition = PortDefinition;

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

export type ContextPlan = FlowContextPlan;

export interface CompiledFlowPlanV2 {
  schemaVersion: 2;
  flowId: string;
  revision: number;
  nodes: Record<string, CompiledFlowNode>;
  executionOrder: string[];
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
  | "groupSync";

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

export interface FlowEventRecord {
  id: number;
  runId: string;
  revision: number;
  kind: string;
  payload: JsonValue;
  createdAt: string;
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
