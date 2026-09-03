import type { DeviceInfo, DeviceWorkOwner } from "./types";

export type DeviceOperationalStatus = "ready" | "busy" | "warning" | "offline";
export type DeviceOperationalFilter = DeviceOperationalStatus | "all";
export type DeviceWorkOwnerReadState = "known" | "loading" | "error";

export interface DeviceOperationalView {
  kind: DeviceOperationalStatus;
  label: string;
  ownerLabel: string | null;
  tone: "ok" | "warn" | "info";
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
  device: Pick<DeviceInfo, "status" | "wdaReady">,
  currentOwner: DeviceWorkOwner | null,
  ownerReadState: DeviceWorkOwnerReadState = "known",
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
  device: Pick<DeviceInfo, "status" | "wdaReady">,
  currentOwner: DeviceWorkOwner | null,
  machineNumber: number,
  displayName: string,
  query: string,
  status: DeviceOperationalFilter,
  ownerReadState: DeviceWorkOwnerReadState = "known",
): boolean {
  const operational = deviceOperationalView(device, currentOwner, ownerReadState);
  if (status !== "all" && operational.kind !== status) return false;
  const needle = normalizeSearch(query);
  if (!needle) return true;
  return normalizeSearch(`Máy ${machineNumber} ${machineNumber} ${displayName}`).includes(needle);
}
