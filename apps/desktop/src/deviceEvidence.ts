import type { DeviceActivity, DeviceLoadState } from "./types";

/**
 * Older than this, the newest record the backend holds about a phone is shown as stale. The
 * load sampler visits a phone holding work about every 60 s (`device_load.rs`), so three
 * missed visits is no longer "what the phone is doing now".
 */
export const DEVICE_EVIDENCE_STALE_MS = 3 * 60_000;

export type DeviceEvidence =
  | { state: "missing" }
  | { state: "fresh" | "stale"; at: string; source: string };

/**
 * The newest thing the backend recorded about this phone: the current run's own update, or
 * the last load sample read from the phone. Only backend timestamps count — never when the UI
 * polled, nor a toast — so a phone nobody has observed reads "Chưa có", not "vừa xong".
 */
export function deviceEvidence(
  activity: DeviceActivity | null | undefined,
  load: DeviceLoadState | undefined,
  nowMs: number,
): DeviceEvidence {
  const candidates: { at: number; source: string }[] = [];
  const activityAt = activity?.updatedAt ? Date.parse(activity.updatedAt) : Number.NaN;
  if (Number.isFinite(activityAt)) candidates.push({ at: activityAt, source: "bản ghi lượt chạy" });
  const sampledAt = load?.lastSample?.sampledAtMs;
  if (typeof sampledAt === "number" && Number.isFinite(sampledAt)) candidates.push({ at: sampledAt, source: "mẫu đo tải máy" });
  const newest = candidates.sort((left, right) => right.at - left.at)[0];
  if (!newest) return { state: "missing" };
  return {
    state: nowMs - newest.at > DEVICE_EVIDENCE_STALE_MS ? "stale" : "fresh",
    at: new Date(newest.at).toISOString(),
    source: newest.source,
  };
}
