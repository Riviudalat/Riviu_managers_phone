import type { PublishStartStatus } from "../../types";

const STORAGE_KEY = "riviu.publish.pending-start.v1";
export const PUBLISH_START_ACKNOWLEDGED = "riviu:publish-start-acknowledged";
export const PUBLISH_START_PENDING = "riviu:publish-start-pending";
export interface PendingPublishStart {
  requestId: string;
  inputKey: string;
  requestedAt: string;
  status?: PublishStartStatus;
}

export function readPendingPublishStart(): PendingPublishStart | null {
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return null;
  const value: unknown = JSON.parse(raw);
  if (!value || typeof value !== "object" || !("requestId" in value)
    || typeof value.requestId !== "string" || !("inputKey" in value)
    || typeof value.inputKey !== "string" || !("requestedAt" in value)
    || typeof value.requestedAt !== "string") {
    throw new Error("Không đọc được lượt đăng đang chờ. Giữ bản ghi này và kiểm tra trước khi tạo lượt khác.");
  }
  return value as PendingPublishStart;
}

export function savePendingPublishStart(value: PendingPublishStart) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
}

export function observePendingPublishStart(value: PendingPublishStart) {
  window.dispatchEvent(new CustomEvent<PendingPublishStart>(PUBLISH_START_PENDING, { detail: value }));
}

export function acknowledgePublishStart(pending: PendingPublishStart, status: PublishStartStatus) {
  const next = { ...pending, status };
  // An acknowledged backend request remains observable even if browser storage fails.
  try { savePendingPublishStart(next); } finally {
    window.dispatchEvent(new CustomEvent<PendingPublishStart>(PUBLISH_START_ACKNOWLEDGED, { detail: next }));
  }
}

export const PUBLISH_START_RETIRED = "riviu:publish-start-retired";

/** Only for a request the backend durably refused (failed receipt, no campaign). The event
 * clears monitor memory too, so a retired marker cannot keep blocking from another view. */
export function retirePublishStart(requestId: string) {
  // Broadcast only after the marker is really gone; a failed removal throws and keeps
  // every view blocked on the still-stored marker instead of hiding it.
  if (readPendingPublishStart()?.requestId === requestId) localStorage.removeItem(STORAGE_KEY);
  window.dispatchEvent(new CustomEvent<string>(PUBLISH_START_RETIRED, { detail: requestId }));
}

export function publishStageLabel(stage: string): string {
  return ({ preparingDevices: "Chuẩn bị thiết bị", checkingDevices: "Kiểm tra từng máy",
    checkingSheet: "Kiểm tra Sheet", staging: "Chuẩn bị nội dung", queued: "Chờ lượt chạy",
    running: "Đang chạy" } as Record<string, string>)[stage] ?? stage;
}
