import type { AgentStatus, DeviceInfo, DeviceLoadState, DeviceWorkOwner, DeviceActivity } from "./types";
import { describeError, helperRecoveryMessage } from "./describeError";

export type DeviceOperationalStatus = "ready" | "connected" | "busy" | "warning" | "offline";
/** `idle` ("Rảnh") is `ready` or `connected`: nothing holds the phone and nothing needs the operator. */
export type DeviceOperationalFilter = DeviceOperationalStatus | "all" | "idle";
export type DeviceWorkOwnerReadState = "known" | "loading" | "error";

export interface DeviceOperationalView {
  kind: DeviceOperationalStatus;
  label: string;
  ownerLabel: string | null;
  tone: "ok" | "warn" | "info";
  reason?: string;
  step?: string;
  helperRecovery?: boolean;
}

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

/** Operator-facing state shared by the stream grid, table and their filter. */
export function deviceOperationalView(
  device: Pick<DeviceInfo, "platform" | "status" | "wdaReady">,
  currentOwner: DeviceWorkOwner | null,
  ownerReadState: DeviceWorkOwnerReadState = "known",
  agent?: AgentStatus,
  activity?: DeviceActivity | null,
  load?: DeviceLoadState,
): DeviceOperationalView {
  const ownerLabel = currentOwner ? deviceWorkOwnerLabel(currentOwner) : null;
  if (device.status === "disconnected") {
    return { kind: "offline", label: "Ngoại tuyến", ownerLabel, tone: "info" };
  }
  if (currentOwner || device.status === "busy") {
    if (currentOwner && ownerReadState === "known" && activity?.label.trim() && (activity.state === "running" || activity.state === "queued" || activity.state === "uncertain")) {
      return { kind: "busy", label: activity.label, ownerLabel: null, tone: "warn", step: activity.step?.trim() || undefined };
    }
    return { kind: "busy", label: "Bận", ownerLabel, tone: "warn" };
  }
  if (ownerReadState === "loading") {
    return { kind: "warning", label: "Đang đọc tác vụ", ownerLabel: null, tone: "warn" };
  }
  if (ownerReadState === "error") {
    return {
      kind: "warning",
      label: "Chưa đọc được tác vụ",
      ownerLabel: null,
      tone: "warn",
    };
  }
  if (device.platform === "android") {
    const recovery = helperRecoveryMessage(agent?.message);
    if (recovery) return { kind: "warning", label: "Cần khôi phục helper", ownerLabel: null, tone: "warn", reason: recovery, helperRecovery: true };
    if (agent?.state === "error" || agent?.state === "repairRequired" || agent?.state === "missing") {
      return { kind: "warning", label: "Chưa điều khiển được", ownerLabel: null, tone: "warn", reason: agent.message ? describeError(agent.message) : "Mở Chẩn đoán để kiểm tra Agent trên máy này." };
    }
    if (device.status === "error") return { kind: "warning", label: "Cần xem", ownerLabel: null, tone: "warn" };
    // An overloaded phone ("máy quá tải") takes no new job until a fresh sample passes, so it is
    // never shown as ready. Existing vocabulary: the reason carries the overload sentence.
    if (load && (load.status === "overloaded" || load.status === "awaitingOperator")) {
      return { kind: "warning", label: "Cần xem", ownerLabel: null, tone: "warn", reason: load.reason ?? "Máy quá tải" };
    }
    // UiAutomator preflight and USB discovery do not prove helper attachment.
    if (agent?.state === "ready" && agent.authReady && agent.sessionReady && agent.features.includes("helperReady")) {
      return { kind: "ready", label: "Sẵn sàng", ownerLabel: null, tone: "ok" };
    }
    return { kind: "connected", label: "Đã kết nối", ownerLabel: null, tone: "info", reason: "Điều khiển được kiểm tra khi mở máy hoặc chạy tác vụ; hiện không có kiểm tra đang chạy." };
  }
  if (device.status === "ready" || device.wdaReady) {
    return { kind: "ready", label: "Sẵn sàng", ownerLabel: null, tone: "ok" };
  }
  return { kind: "warning", label: "Cần xem", ownerLabel: null, tone: "warn" };
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
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/đ/g, "d")
    .replace(/Đ/g, "D")
    .toLocaleLowerCase("vi")
    .trim();
}

/** Search only the operator-visible identity; model and serial remain details-only. */
export function deviceMatchesFleetFilter(
  device: Pick<DeviceInfo, "platform" | "status" | "wdaReady">,
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
  if (filter === "all") return true;
  if (filter === "idle") return kind === "ready" || kind === "connected";
  return kind === filter;
}

/** Header fleet counters. `null` is unknown ("Chưa rõ"), never 0. */
export interface FleetCounts {
  idle: number | null;
  busy: number | null;
  attention: number | null;
  offline: number | null;
}

/**
 * Counts with the tile's own predicate, so a counter and the wall filter it sets always agree.
 * Unknown follows the header's "Sẵn sàng" rule: owners unread make busy/idle/attention unknown,
 * and a connected idle Android phone without cached helper proof could be idle or need attention.
 */
export function fleetCounts(
  devices: Pick<DeviceInfo, "udid" | "platform" | "status" | "wdaReady">[],
  settled: boolean,
  ownerFor: (udid: string) => DeviceWorkOwner | null,
  ownerReadState: DeviceWorkOwnerReadState,
  agentFor: (udid: string) => AgentStatus | undefined,
  loadFor: (udid: string) => DeviceLoadState | undefined,
): FleetCounts {
  if (!settled) return { idle: null, busy: null, attention: null, offline: null };
  const counts = { idle: 0, busy: 0, attention: 0, offline: 0 };
  const ownersKnown = ownerReadState === "known";
  let idleKnown = ownersKnown;
  for (const device of devices) {
    const owner = ownerFor(device.udid);
    const agent = agentFor(device.udid);
    if (device.platform === "android" && device.status !== "disconnected" && device.status !== "busy" && !owner && !agent) idleKnown = false;
    const kind = deviceOperationalView(device, owner, ownerReadState, agent, null, loadFor(device.udid)).kind;
    if (kind === "offline") counts.offline += 1;
    else if (kind === "busy") counts.busy += 1;
    else if (kind === "warning") counts.attention += 1;
    else counts.idle += 1;
  }
  return {
    idle: idleKnown ? counts.idle : null,
    busy: ownersKnown ? counts.busy : null,
    attention: idleKnown ? counts.attention : null,
    offline: counts.offline,
  };
}
