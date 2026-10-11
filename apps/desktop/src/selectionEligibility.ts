import { deviceWorkOwnerLabel } from "./deviceWork";
import type { DeviceInfo, DeviceWorkOwner } from "./types";

/**
 * Who may receive new work, and what happens to a chosen phone that stops being able to
 * (PLAN §3). Shared by the Control Center selection and, later, the automation scope pickers.
 */

/**
 * Owners that mean "this phone is running work", so it takes no new work.
 *
 * The others hold a phone for moments, or on the operator's own behalf: direct control takes
 * the lease per command, Sync holds exactly the phones that are selected, a repair is one the
 * operator started on them, and the idle sweep yields to everything. Counting those would
 * drop phones out of the selection every time the operator touched them.
 */
const WORK_OWNERS: ReadonlySet<DeviceWorkOwner> = new Set<DeviceWorkOwner>(["nurture", "interaction", "script"]);

/** Why a phone cannot be added for new work right now, or null when it can. */
export type SelectionBlock = (udid: string) => string | null;

/** A chosen phone that left by itself, and why; kept until the operator clears it. */
export interface ExcludedDevice { udid: string; reason: string; at: number }

/**
 * Why a phone cannot be added to the selection for new work, or null when it can.
 *
 * Only positive evidence blocks: an unread owner is not "busy", because the backend's lease
 * refuses a busy phone anyway and an unknown must not empty the selection. `device` is
 * undefined when the phone is no longer in the roster.
 */
export function newWorkBlock(
  device: Pick<DeviceInfo, "status"> | undefined,
  owner: DeviceWorkOwner | null | undefined,
): string | null {
  if (!device) return "Không còn trong danh sách máy";
  if (device.status === "disconnected") return "Mất kết nối";
  if (owner && WORK_OWNERS.has(owner)) return `Đang chạy: ${deviceWorkOwnerLabel(owner)}`;
  if (device.status === "busy") return "Đang chạy tác vụ";
  return null;
}

/**
 * Splits chosen phones into those still able to take new work and those that must leave,
 * each with its reason. The caller removes `excluded` from its set and never adds them back
 * by itself: a phone that frees up stays out until the operator chooses it again.
 */
export function excludeBlocked(
  chosen: readonly string[],
  block: SelectionBlock,
  at = Date.now(),
): { kept: string[]; excluded: ExcludedDevice[] } {
  const kept: string[] = [];
  const excluded: ExcludedDevice[] = [];
  for (const udid of chosen) {
    const reason = block(udid);
    if (reason) excluded.push({ udid, reason, at });
    else kept.push(udid);
  }
  return { kept, excluded };
}
