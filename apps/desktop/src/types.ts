export type ConnectionKind = "usb" | "wifi" | "mock";
export type DeviceStatus =
  | "disconnected"
  | "pairing"
  | "connected"
  | "preparing"
  | "ready"
  | "busy"
  | "error";

export interface DeviceInfo {
  udid: string;
  name: string;
  model: string;
  iosVersion: string;
  connection: ConnectionKind;
  status: DeviceStatus;
  battery?: number | null;
  wdaReady: boolean;
  wdaExpiresAt?: string | null;
  streamUrl?: string | null;
  lastError?: string | null;
}

export type TileSize = "thumbnail" | "medium" | "large" | "extraLarge";
export type StreamQuality = "low" | "medium" | "high" | "extra";

export interface StreamSettings {
  fps: number;
  tileSize: TileSize;
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

export type PageId =
  | "control"
  | "groups"
  | "proxy"
  | "material"
  | "apps"
  | "scripts"
  | "jobs"
  | "sync"
  | "publish"
  | "data"
  | "team"
  | "logs"
  | "account"
  | "api"
  | "settings"
  | "nurture"
  | "login"
  | "register";

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

export interface OpLog {
  id: string;
  action: string;
  detail: string;
  createdAt: string;
}

export interface LocalUser {
  id: string;
  email: string;
  role: string;
  createdAt: string;
}

export interface AuthSession {
  showAuthUi: boolean;
  bypassed: boolean;
  user?: LocalUser | null;
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

export interface NurtureSessionStatus {
  udid: string;
  running: boolean;
  videosDone: number;
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
