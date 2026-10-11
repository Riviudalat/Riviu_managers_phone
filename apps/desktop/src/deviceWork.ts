import type { AgentStatus, DeviceInfo, DeviceLoadState, DeviceWorkOwner, DeviceActivity } from "./types";
import { describeError, helperRecoveryMessage } from "./describeError";

/**
 * The three status axes (PLAN §4). Each axis has its own "Chưa rõ": missing data is unknown,
 * never Sẵn sàng and never Mất kết nối.
 * - Kết nối: does the phone still answer over USB/Wi-Fi.
 * - Điều khiển: is there a still-valid proof that Riviu can drive it. Chưa kiểm (nothing has
 *   proved it yet) is not Không đạt (a check failed).
 * - Công việc: does a job hold it, or does it need the operator.
 */
export type ConnectionAxisState = "connected" | "disconnected" | "unknown";
export type ControlAxisState = "ready" | "unchecked" | "failed" | "unknown";
export type WorkAxisState = "idle" | "running" | "attention" | "unknown";

export interface DeviceStatusAxis<S extends string> {
  state: S;
  /** The axis value in operator words: "Đã kết nối", "Chưa kiểm", "Đang chạy"… */
  label: string;
  /** The job ("Nuôi TikTok") or the short reason ("Máy quá tải") after the label. */
  detail?: string;
  /** The full explanation; tooltip and details drawer. */
  reason?: string;
}

export interface DeviceStatusAxes {
  connection: DeviceStatusAxis<ConnectionAxisState>;
  control: DeviceStatusAxis<ControlAxisState>;
  work: DeviceStatusAxis<WorkAxisState>;
}

/**
 * The condensed one-line status, decided from the axes in this order: Mất kết nối, Đang chạy,
 * Cần xử lý, Chưa rõ, Chưa kiểm, Sẵn sàng. `busy` is never assignable to new work, and `ready`
 * needs both a still-valid control proof and no job holding the phone.
 */
export type DeviceOperationalStatus = "ready" | "unchecked" | "busy" | "warning" | "offline" | "unknown";
export type DeviceOperationalFilter = DeviceOperationalStatus | "all";
export type DeviceWorkOwnerReadState = "known" | "loading" | "error";

export interface DeviceOperationalView {
  kind: DeviceOperationalStatus;
  /** "Sẵn sàng", "Chưa kiểm", "Đang chạy", "Cần xử lý", "Mất kết nối" or "Chưa rõ". */
  label: string;
  /** The job or the short reason after the label: "Đang chạy: Nuôi TikTok". */
  detail?: string;
  /** The full explanation (tooltip, accessible name, attention line). */
  reason?: string;
  /** The running job's current step. */
  step?: string;
  helperRecovery?: boolean;
  axes: DeviceStatusAxes;
}

/** Operator words for each condensed state; the counters and the filter select use the same. */
export const DEVICE_STATUS_LABELS: Record<DeviceOperationalStatus, string> = {
  ready: "Sẵn sàng",
  unchecked: "Chưa kiểm",
  busy: "Đang chạy",
  warning: "Cần xử lý",
  offline: "Mất kết nối",
  unknown: "Chưa rõ",
};

/** The wall filter's states, in counter order. */
export const DEVICE_STATUS_FILTERS: readonly DeviceOperationalStatus[] = ["ready", "unchecked", "busy", "warning", "offline", "unknown"];

const OWNER_LABELS: Record<DeviceWorkOwner, string> = {
  nurture: "Nuôi TikTok",
  interaction: "Tương tác",
  script: "Tác vụ tự động",
  repair: "Sửa chữa",
  manualControl: "Điều khiển trực tiếp",
  groupSync: "Đồng bộ nhóm",
  idleSweep: "Tự khôi phục nền",
};

export function deviceWorkOwnerLabel(owner: DeviceWorkOwner | string): string {
  return OWNER_LABELS[owner as DeviceWorkOwner] ?? "Tác vụ chưa nhận diện";
}

type StatusDevice = Pick<DeviceInfo, "platform" | "status" | "wdaReady"> & Partial<Pick<DeviceInfo, "lastError">>;

const CONNECTED_STATUSES: ReadonlySet<string> = new Set(["pairing", "connected", "preparing", "ready", "busy", "error"]);
const UNCHECKED_REASON = "Điều khiển chưa được chứng minh: được kiểm khi mở máy hoặc chạy tác vụ; hiện không có kiểm tra đang chạy.";
const CONTROL_UNREAD_REASON = "Chưa đọc được bằng chứng điều khiển của máy này (chỉ đọc trong Trung tâm điều khiển), nên không coi là Sẵn sàng.";

function connectionAxis(device: StatusDevice): DeviceStatusAxis<ConnectionAxisState> {
  if (device.status === "disconnected") {
    return { state: "disconnected", label: "Mất kết nối", reason: "Máy không còn trả lời qua USB/Wi-Fi." };
  }
  if (CONNECTED_STATUSES.has(device.status)) return { state: "connected", label: "Đã kết nối" };
  return { state: "unknown", label: "Chưa rõ", detail: "kết nối", reason: "Danh sách máy báo một trạng thái kết nối chưa nhận diện." };
}

function controlAxis(device: StatusDevice, agent: AgentStatus | undefined): DeviceStatusAxis<ControlAxisState> & { helperRecovery?: true } {
  if (device.status === "disconnected") {
    return { state: "unknown", label: "Chưa rõ", reason: "Máy mất kết nối nên bằng chứng điều khiển cũ không còn hiệu lực." };
  }
  if (device.platform === "android") {
    const recovery = helperRecoveryMessage(agent?.message);
    if (recovery) return { state: "failed", label: "Không đạt", detail: "Cần khôi phục helper", reason: recovery, helperRecovery: true };
    if (!agent) return { state: "unknown", label: "Chưa rõ", detail: "chưa đọc điều khiển", reason: CONTROL_UNREAD_REASON };
    if (agent.state === "error" || agent.state === "repairRequired" || agent.state === "missing") {
      return {
        state: "failed", label: "Không đạt", detail: "Chưa điều khiển được",
        reason: agent.message ? describeError(agent.message) : "Mở Chẩn đoán để kiểm tra Agent trên máy này.",
      };
    }
    // UiAutomator preflight and USB discovery do not prove helper attachment. The cached status
    // is re-read whenever the roster row changes, so a proof never outlives its connection.
    if (agent.state === "ready" && agent.authReady && agent.sessionReady && agent.features.includes("helperReady")) {
      return { state: "ready", label: "Sẵn sàng", reason: "Helper, phiên và xác thực đã sẵn sàng trong lần kiểm của kết nối hiện tại." };
    }
    if (agent.state === "starting") {
      return { state: "unchecked", label: "Chưa kiểm", detail: "đang kiểm", reason: "Agent đang khởi động; chưa có bằng chứng điều khiển." };
    }
    return { state: "unchecked", label: "Chưa kiểm", reason: UNCHECKED_REASON };
  }
  if (device.status === "ready" || device.wdaReady) {
    return { state: "ready", label: "Sẵn sàng", reason: "WDA trên iPhone đã sẵn sàng." };
  }
  switch (device.status) {
    case "error":
      return { state: "failed", label: "Không đạt", detail: "Chưa điều khiển được", reason: "Máy báo lỗi; mở Chẩn đoán để kiểm tra WDA." };
    case "pairing":
      return { state: "unchecked", label: "Chưa kiểm", detail: "đang ghép nối", reason: "iPhone đang ghép nối; chưa có bằng chứng điều khiển." };
    case "preparing":
      return { state: "unchecked", label: "Chưa kiểm", detail: "đang chuẩn bị", reason: "WDA đang chuẩn bị; chưa có bằng chứng điều khiển." };
    case "busy":
      return { state: "unknown", label: "Chưa rõ", reason: "Máy đang bận nên chưa đọc được trạng thái WDA." };
    default:
      return { state: "unchecked", label: "Chưa kiểm", reason: "WDA chưa sẵn sàng; được chuẩn bị khi mở máy hoặc chạy tác vụ." };
  }
}

function workAxis(
  device: StatusDevice,
  currentOwner: DeviceWorkOwner | null,
  ownerReadState: DeviceWorkOwnerReadState,
  control: DeviceStatusAxis<ControlAxisState>,
  activity: DeviceActivity | null | undefined,
  load: DeviceLoadState | undefined,
): DeviceStatusAxis<WorkAxisState> & { step?: string } {
  if (currentOwner || device.status === "busy") {
    if (currentOwner && ownerReadState === "known" && activity?.label.trim() && (activity.state === "running" || activity.state === "queued" || activity.state === "uncertain")) {
      return { state: "running", label: "Đang chạy", detail: activity.label.trim(), step: activity.step?.trim() || undefined };
    }
    return { state: "running", label: "Đang chạy", detail: currentOwner ? deviceWorkOwnerLabel(currentOwner) : undefined };
  }
  if (control.state === "failed") return { state: "attention", label: "Cần xử lý", detail: control.detail, reason: control.reason };
  if (device.status === "error") {
    return {
      state: "attention", label: "Cần xử lý", detail: "Máy báo lỗi",
      reason: device.lastError ? describeError(device.lastError) : "Mở Chẩn đoán để xem lỗi của máy này.",
    };
  }
  // An overloaded phone ("máy quá tải") takes no new job until a fresh sample passes.
  if (load && (load.status === "overloaded" || load.status === "awaitingOperator")) {
    return { state: "attention", label: "Cần xử lý", detail: load.reason ?? "Máy quá tải", reason: load.reason ?? "Máy quá tải" };
  }
  if (ownerReadState === "loading") {
    return { state: "unknown", label: "Chưa rõ", detail: "đang đọc tác vụ", reason: "Đang đọc tác vụ đang giữ máy." };
  }
  if (ownerReadState === "error") {
    return { state: "unknown", label: "Chưa rõ", detail: "chưa đọc được tác vụ", reason: "Chưa đọc được tác vụ đang giữ máy." };
  }
  return { state: "idle", label: "Rảnh", reason: "Không có tác vụ nào giữ máy." };
}

/** Operator-facing state shared by the stream grid, table, pickers, drawer and their filter. */
export function deviceOperationalView(
  device: StatusDevice,
  currentOwner: DeviceWorkOwner | null,
  ownerReadState: DeviceWorkOwnerReadState = "known",
  agent?: AgentStatus,
  activity?: DeviceActivity | null,
  load?: DeviceLoadState,
): DeviceOperationalView {
  const connection = connectionAxis(device);
  const { helperRecovery, ...control } = controlAxis(device, agent);
  const { step, ...work } = workAxis(device, currentOwner, ownerReadState, control, activity, load);
  const axes: DeviceStatusAxes = { connection, control, work };
  if (connection.state === "disconnected") {
    // A lease the coordinator still records is said, not hidden: the job may be waiting for it.
    const held = work.state === "running" ? `${work.detail ?? "Một tác vụ"} vẫn đang giữ máy.` : undefined;
    return { kind: "offline", label: "Mất kết nối", reason: [connection.reason, held].filter(Boolean).join(" "), axes };
  }
  if (connection.state === "unknown") return { kind: "unknown", label: "Chưa rõ", detail: connection.detail, reason: connection.reason, axes };
  if (work.state === "running") return { kind: "busy", label: "Đang chạy", detail: work.detail, step, axes };
  if (work.state === "attention") {
    return { kind: "warning", label: "Cần xử lý", detail: work.detail, reason: work.reason, ...(helperRecovery ? { helperRecovery } : {}), axes };
  }
  if (work.state === "unknown") return { kind: "unknown", label: "Chưa rõ", detail: work.detail, reason: work.reason, axes };
  if (control.state === "unknown") return { kind: "unknown", label: "Chưa rõ", detail: control.detail, reason: control.reason, axes };
  if (control.state === "unchecked") return { kind: "unchecked", label: "Chưa kiểm", detail: control.detail, reason: control.reason, axes };
  return { kind: "ready", label: "Sẵn sàng", reason: control.reason, axes };
}

/**
 * A picker's view: the roster row alone, with no work-owner projection and no helper proof.
 * Android control is therefore "Chưa rõ" here (it is checked when a run starts), never Sẵn sàng.
 */
export function deviceRosterView(device: StatusDevice): DeviceOperationalView {
  const view = deviceOperationalView(device, null);
  if (view.kind !== "unknown" || view.axes.control.state !== "unknown") return view;
  const prepared = device.status === "ready" ? "đã chuẩn bị" : "chưa chuẩn bị";
  return {
    ...view,
    detail: prepared,
    reason: `Đã kết nối, ${prepared}. Điều khiển được kiểm khi bắt đầu chạy; bằng chứng helper chỉ đọc trong Trung tâm điều khiển.`,
  };
}

/** "Đang chạy: Nuôi TikTok" — the line every surface shows for a phone. */
export function deviceStatusText(view: Pick<DeviceOperationalView, "label" | "detail">): string {
  return view.detail ? `${view.label}: ${view.detail}` : view.label;
}

/** Every line of the status in full: the label, the step and the reason. */
export function deviceStatusTitle(view: Pick<DeviceOperationalView, "label" | "detail" | "step" | "reason">): string {
  return [deviceStatusText(view), view.step, view.reason].filter(Boolean).join("\n");
}

/**
 * The toolbar's helper recovery without a selection: only connected Android phones whose
 * cached control status or roster row reports a helper recovery need. Preparing every phone
 * would only refuse healthy ones (live owner, no retained owner) and bury the real failures.
 */
export function helperRecoveryTargets(
  devices: Pick<DeviceInfo, "udid" | "platform" | "status" | "lastError">[],
  agentFor: (udid: string) => AgentStatus | undefined,
): string[] {
  return devices
    .filter(device => device.platform === "android" && device.status !== "disconnected"
      && (helperRecoveryMessage(agentFor(device.udid)?.message) !== null
        || helperRecoveryMessage(device.lastError) !== null))
    .map(device => device.udid);
}

function normalizeSearch(value: string): string {
  return value
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/đ/g, "d")
    .replace(/Đ/g, "D")
    .toLocaleLowerCase("vi")
    .trim();
}

/** Search only the operator-visible identity; model and serial remain details-only. */
export function deviceMatchesFleetFilter(
  device: StatusDevice,
  currentOwner: DeviceWorkOwner | null,
  machineNumber: number,
  displayName: string,
  query: string,
  status: DeviceOperationalFilter,
  ownerReadState: DeviceWorkOwnerReadState = "known",
  agent?: AgentStatus,
  load?: DeviceLoadState,
): boolean {
  const operational = deviceOperationalView(device, currentOwner, ownerReadState, agent, null, load);
  if (!operationalMatchesFilter(operational.kind, status)) return false;
  const needle = normalizeSearch(query);
  if (!needle) return true;
  return normalizeSearch(`Máy ${machineNumber} ${machineNumber} ${displayName}`).includes(needle);
}

export function operationalMatchesFilter(kind: DeviceOperationalStatus, filter: DeviceOperationalFilter): boolean {
  return filter === "all" || kind === filter;
}

/**
 * Header fleet counters: every phone is counted once, under its condensed status, so the six
 * numbers add up to `total`. `null` is unknown ("?"), never 0.
 */
export interface FleetCounts {
  total: number | null;
  ready: number | null;
  unchecked: number | null;
  busy: number | null;
  attention: number | null;
  offline: number | null;
  unknown: number | null;
}

const COUNT_KEY: Record<DeviceOperationalStatus, Exclude<keyof FleetCounts, "total">> = {
  ready: "ready",
  unchecked: "unchecked",
  busy: "busy",
  warning: "attention",
  offline: "offline",
  unknown: "unknown",
};

/**
 * Counts with the tile's own predicate, so a counter and the wall filter it sets always agree.
 * A counter turns unknown when a phone counted as "Chưa rõ" could belong to it: an unread work
 * owner could hide a running, attention, ready or unchecked phone; an unread control proof
 * (only Trung tâm điều khiển reads that cache) a ready, unchecked or failed one.
 */
export function fleetCounts(
  devices: Pick<DeviceInfo, "udid" | "platform" | "status" | "wdaReady">[],
  settled: boolean,
  ownerFor: (udid: string) => DeviceWorkOwner | null,
  ownerReadState: DeviceWorkOwnerReadState,
  agentFor: (udid: string) => AgentStatus | undefined,
  loadFor: (udid: string) => DeviceLoadState | undefined,
): FleetCounts {
  if (!settled) return { total: null, ready: null, unchecked: null, busy: null, attention: null, offline: null, unknown: null };
  const counts = { ready: 0, unchecked: 0, busy: 0, attention: 0, offline: 0, unknown: 0 };
  let connectionUnknown = false;
  let workUnknown = false;
  let controlUnknown = false;
  for (const device of devices) {
    const view = deviceOperationalView(device, ownerFor(device.udid), ownerReadState, agentFor(device.udid), null, loadFor(device.udid));
    counts[COUNT_KEY[view.kind]] += 1;
    if (view.kind !== "unknown") continue;
    if (view.axes.connection.state === "unknown") connectionUnknown = true;
    else if (view.axes.work.state === "unknown") workUnknown = true;
    else controlUnknown = true;
  }
  const idleUnknown = connectionUnknown || workUnknown || controlUnknown;
  return {
    total: devices.length,
    ready: idleUnknown ? null : counts.ready,
    unchecked: idleUnknown ? null : counts.unchecked,
    busy: connectionUnknown || workUnknown ? null : counts.busy,
    attention: idleUnknown ? null : counts.attention,
    offline: connectionUnknown ? null : counts.offline,
    unknown: counts.unknown,
  };
}
