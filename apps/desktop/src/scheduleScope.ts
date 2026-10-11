import { resolveAutomationTarget } from "./automationTargets";
import type { DeviceGroup, DeviceInfo, DeviceMeta, TargetRef } from "./types";

/**
 * What a schedule's approval covers, and how far its target has drifted from it.
 *
 * The backend refuses a due run on the same rule (`approved_scope_difference` in
 * `crates/core/src/automation.rs`); this copy only explains the refusal before it happens.
 * `added` are phones the target would now reach without approval. `removed` are approved
 * phones a group or explicit list no longer names. An approved phone that is merely offline
 * is neither: the run skips it and records why.
 */
export type ScheduleScope =
  | { kind: "unknown" }
  | { kind: "missingGroup" }
  | { kind: "unapproved"; designated: string[] }
  | { kind: "changed"; designated: string[]; added: string[]; removed: string[] }
  | { kind: "approved"; designated: string[] };

/** The phones a target names right now, offline group members included. `null`: the group is gone. */
export function designatedUdids(target: TargetRef, devices: DeviceInfo[], groups: DeviceGroup[]): string[] | null {
  if (target.type === "all") return devices.map((device) => device.udid);
  if (target.type === "group") {
    const group = groups.find((candidate) => candidate.id === target.groupId);
    return group ? [...new Set(group.udids)] : null;
  }
  return [...new Set(target.udids)];
}

export function scheduleScope(
  approved: string[] | null,
  target: TargetRef | null | undefined,
  devices: DeviceInfo[],
  groups: DeviceGroup[] | null,
): ScheduleScope {
  if (!target || (target.type === "group" && !groups)) return { kind: "unknown" };
  const designated = designatedUdids(target, devices, groups ?? []);
  if (!designated) return { kind: "missingGroup" };
  if (!approved) return { kind: "unapproved", designated };
  const approvedSet = new Set(approved);
  const added = resolveAutomationTarget(target, devices, groups ?? []).filter((udid) => !approvedSet.has(udid));
  const named = new Set(designated);
  const removed = target.type === "all" ? [] : [...new Set(approved)].filter((udid) => !named.has(udid));
  return added.length || removed.length
    ? { kind: "changed", designated, added, removed }
    : { kind: "approved", designated };
}

export const SCHEDULE_PREVIEW_COUNT = 5;

/**
 * The next runs as the backend will claim them: the stored due slot, then whole intervals
 * after it. An overdue slot runs at the next scan, and the slots after it skip what was missed.
 */
export function upcomingRuns(
  nextDueAt: string | null,
  everyMinutes: number,
  now: number,
  count = SCHEDULE_PREVIEW_COUNT,
): { overdue: boolean; times: Date[] } {
  const interval = everyMinutes * 60_000;
  const first = nextDueAt ? Date.parse(nextDueAt) : Number.NaN;
  if (!Number.isFinite(first) || !(interval > 0)) return { overdue: false, times: [] };
  if (first >= now) {
    return { overdue: false, times: Array.from({ length: count }, (_, index) => new Date(first + index * interval)) };
  }
  const after = first + (Math.floor((now - first) / interval) + 1) * interval;
  return {
    overdue: true,
    times: [new Date(first), ...Array.from({ length: count - 1 }, (_, index) => new Date(after + index * interval))],
  };
}

/** Runs of a schedule saved or switched on at `now`: the backend arms the first slot one interval later. */
export function runsIfEnabledAt(now: number, everyMinutes: number, count = SCHEDULE_PREVIEW_COUNT): Date[] {
  return upcomingRuns(new Date(now + everyMinutes * 60_000).toISOString(), everyMinutes, now, count).times;
}

/** The zone every schedule time is shown in, stated rather than assumed. */
export function localTimeZoneLabel(at = new Date()): string {
  const offset = -at.getTimezoneOffset();
  const sign = offset >= 0 ? "+" : "-";
  const hours = String(Math.floor(Math.abs(offset) / 60)).padStart(2, "0");
  const minutes = String(Math.abs(offset) % 60).padStart(2, "0");
  let zone = "";
  try {
    zone = Intl.DateTimeFormat().resolvedOptions().timeZone ?? "";
  } catch {
    // An engine without zone names still has the offset.
  }
  return `Giờ máy tính${zone ? ` ${zone}` : ""} (UTC${sign}${hours}:${minutes})`;
}

export function formatScheduleTime(time: Date): string {
  return time.toLocaleString("vi-VN", {
    weekday: "short",
    day: "2-digit",
    month: "2-digit",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** "Máy N · alias", falling back to the phone's name, then its ID for a phone no longer listed. */
export function scheduleDeviceLabel(udid: string, devices: DeviceInfo[], metas?: Map<string, DeviceMeta>): string {
  const meta = metas?.get(udid);
  const alias = meta?.alias?.trim();
  if (meta?.number != null && meta.number > 0) return alias ? `Máy ${meta.number} · ${alias}` : `Máy ${meta.number}`;
  if (alias) return alias;
  return devices.find((device) => device.udid === udid)?.name ?? `Thiết bị ${udid}`;
}

/** Operator text for the reason a due run was refused or failed before any effect. */
export function scheduleErrorLabel(code: string | null): string | null {
  if (!code) return null;
  switch (code) {
    case "scope_not_approved":
      return "Chưa duyệt danh sách máy — lần chạy bị từ chối trước khi gửi lệnh";
    case "approved_scope_changed":
      return "Phạm vi khác danh sách đã duyệt — lần chạy bị từ chối trước khi gửi lệnh";
    case "target_resolution_failed":
      return "Không có máy nào trong phạm vi đang kết nối, hoặc nhóm không còn";
    case "profile_archived":
      return "Cấu hình đã lưu trữ";
    case "unsupported_schedule_schema":
      return "Định dạng lịch chưa được hỗ trợ";
    default:
      return code;
  }
}
