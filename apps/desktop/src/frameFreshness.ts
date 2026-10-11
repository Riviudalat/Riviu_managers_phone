import { useEffect, useState } from "react";
import { viewLastFrameAt } from "./viewStore";

/**
 * How old a tile's picture may get before the tile says so. UI thresholds chosen for the
 * Tổng quan wall (PLAN §2 "Độ mới"), not measured on a device: past the first the tile prints
 * "hình 25 giây trước"; past the second it lays "Hình cũ" over the picture.
 *
 * Frame age only. It is not the age of the last status check and not the connection: a phone
 * whose screen does not change sends no new frame, so a still, connected phone ages too, and
 * the words say "hình", never "mất kết nối". Làm mới never resets it — only a drawn frame does.
 */
export const FRAME_AGE_SHOWN_MS = 20_000;
export const FRAME_STALE_MS = 60_000;
/** How often a tile re-reads its frame age. Coarser than the thresholds on purpose. */
const FRAME_AGE_TICK_MS = 5_000;

export type FrameFreshness =
  | { state: "fresh" | "unknown" }
  | { state: "aging" | "stale"; ageText: string; shortText: string };

export function frameFreshness(lastFrameAt: number | undefined, nowMs: number): FrameFreshness {
  if (lastFrameAt === undefined) return { state: "unknown" };
  const age = Math.max(0, nowMs - lastFrameAt);
  if (age < FRAME_AGE_SHOWN_MS) return { state: "fresh" };
  const seconds = Math.floor(age / 1000);
  const shortText = seconds < 120 ? `${seconds} giây` : `${Math.floor(seconds / 60)} phút`;
  // The short form fits the Tổng quan header beside Máy N; the full one is its tooltip and name.
  return { state: age >= FRAME_STALE_MS ? "stale" : "aging", ageText: `hình ${shortText} trước`, shortText };
}

/** The tile's own frame-age clock; idle while the tile has no live picture. */
export function useFrameFreshness(udid: string, active: boolean): FrameFreshness {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), FRAME_AGE_TICK_MS);
    return () => window.clearInterval(timer);
  }, [active]);
  return active ? frameFreshness(viewLastFrameAt(udid), now) : { state: "unknown" };
}
