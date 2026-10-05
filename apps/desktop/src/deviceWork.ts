import type { AgentStatus, DeviceInfo, DeviceWorkOwner } from "./types";
import { describeError, helperRecoveryMessage } from "./describeError";

export type DeviceOperationalStatus = "ready" | "connected" | "busy" | "warning" | "offline";
export type DeviceOperationalFilter = DeviceOperationalStatus | "all";
export type DeviceWorkOwnerReadState = "known" | "loading" | "error";

export interface DeviceOperationalView {
  kind: DeviceOperationalStatus;
  label: string;
  ownerLabel: string | null;
  tone: "ok" | "warn" | "info";
  reason?: string;
  helperRecovery?: boolean;
}

const OWNER_LABELS: Record<DeviceWorkOwner, string> = {
  nurture: "Nuôi TikTok",
  interaction: "Tương tác",
  script: "Flow",
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
): DeviceOperationalView {
  const ownerLabel = currentOwner ? deviceWorkOwnerLabel(currentOwner) : null;
  if (device.status === "disconnected") {
    return { kind: "offline", label: "Ngoại tuyến", ownerLabel, tone: "info" };
  }
  if (currentOwner || device.status === "busy") {
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
    // UiAutomator preflight and USB discovery do not prove helper attachment.
    if (agent?.state === "ready" && agent.authReady && agent.sessionReady && agent.features.includes("helperReady")) {
      return { kind: "ready", label: "Sẵn sàng", ownerLabel: null, tone: "ok" };
    }
    return { kind: "connected", label: "Đã kết nối · Chưa kiểm tra điều khiển", ownerLabel: null, tone: "info" };
  }
  if (device.status === "ready" || device.wdaReady) {
    return { kind: "ready", label: "Sẵn sàng", ownerLabel: null, tone: "ok" };
  }
  return { kind: "warning", label: "Cần xem", ownerLabel: null, tone: "warn" };
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
): boolean {
  const operational = deviceOperationalView(device, currentOwner, ownerReadState, agent);
  if (status !== "all" && operational.kind !== status) return false;
  const needle = normalizeSearch(query);
  if (!needle) return true;
  return normalizeSearch(`Máy ${machineNumber} ${machineNumber} ${displayName}`).includes(needle);
}
