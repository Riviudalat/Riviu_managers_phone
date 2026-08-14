import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  collectStalledViews,
  nextViewReconnectDelay,
  PAINT_RECOVERY_COOLDOWN_MS,
  OBSERVED_RESTART_MS,
  PAINT_RECOVERY_MAX_MS,
  PAINT_STALL_MS,
  shouldAttemptViewRecovery,
  viewRecoveryDelayMs,
  startViewClient,
  VIEW_RECONNECT_MAX_MS,
  VIEW_RECONNECT_MIN_MS,
} from "./viewStore";

vi.mock("./api", () => ({
  viewEndpoint: vi.fn(async () => null),
  viewEnsure: vi.fn(async () => undefined),
}));

describe("view WebSocket reconnect", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("backs off from 200 ms to about 2 s", () => {
    expect(nextViewReconnectDelay(VIEW_RECONNECT_MIN_MS)).toBe(400);
    expect(nextViewReconnectDelay(400)).toBe(800);
    expect(nextViewReconnectDelay(800)).toBe(1600);
    expect(nextViewReconnectDelay(1600)).toBe(VIEW_RECONNECT_MAX_MS);
    expect(nextViewReconnectDelay(VIEW_RECONNECT_MAX_MS)).toBe(VIEW_RECONNECT_MAX_MS);
  });

  it("tries the endpoint once in test mode and does not reconnect", async () => {
    const api = await import("./api");
    startViewClient();
    await vi.waitFor(() => expect(api.viewEndpoint).toHaveBeenCalledTimes(1));
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(api.viewEndpoint).toHaveBeenCalledTimes(1);
  });
});

describe("stalled view detection", () => {
  // The defect being guarded, measured on hardware: a producer restarted at a new
  // resolution, decoded exactly one keyframe, and then painted nothing for 8 minutes while
  // the Rust watchdog stayed silent -- it counts bytes arriving from the phone, not frames
  // drawn on screen, so a stream that arrives and cannot be decoded looks perfectly healthy
  // to it. The canvas held a stale frame the whole time, which reads as a live phone that
  // simply is not changing.
  const now = 1_000_000;

  const beat = (at: number, receivedCount: number, frames: number) => ({
    at,
    received: receivedCount,
    frames,
  });

  it("flags a view whose packets kept arriving while it drew nothing", () => {
    const painted = new Map([["stale", beat(now - PAINT_STALL_MS - 1, 100, 50)]]);
    const latest = new Map([["stale", beat(now, 340, 50)]]);
    expect(collectStalledViews(now, ["stale"], painted, latest)).toEqual(["stale"]);
  });

  it("leaves a static screen alone even though it has drawn nothing for ages", () => {
    // The mistake the first version of this rule made, and the expensive one: scrcpy only
    // encodes when the screen changes, so a phone parked on a lock screen sends nothing and
    // paints nothing and is entirely healthy. Restarting it cost ~45s of real downtime.
    const painted = new Map([["idle", beat(now - 10 * PAINT_STALL_MS, 100, 50)]]);
    const latest = new Map([["idle", beat(now, 100, 50)]]);
    expect(collectStalledViews(now, ["idle"], painted, latest)).toEqual([]);
  });

  it("leaves a view alone while it is still painting", () => {
    const painted = new Map([["fresh", beat(now - 1000, 100, 50)]]);
    const latest = new Map([["fresh", beat(now, 124, 74)]]);
    expect(collectStalledViews(now, ["fresh"], painted, latest)).toEqual([]);
  });

  it("leaves a view exactly at the boundary alone", () => {
    const painted = new Map([["edge", beat(now - PAINT_STALL_MS, 100, 50)]]);
    const latest = new Map([["edge", beat(now, 300, 50)]]);
    expect(collectStalledViews(now, ["edge"], painted, latest)).toEqual([]);
  });

  it("does not flag a view that has never painted", () => {
    const latest = new Map([["starting", beat(now, 5, 0)]]);
    expect(collectStalledViews(now, ["starting"], new Map(), latest)).toEqual([]);
  });

  it("ignores a stalled view that is not live", () => {
    const painted = new Map([["gone", beat(now - 10 * PAINT_STALL_MS, 100, 50)]]);
    const latest = new Map([["gone", beat(now, 900, 50)]]);
    expect(collectStalledViews(now, [], painted, latest)).toEqual([]);
  });

  it("rate limits recovery so an undecodable stream cannot be restarted every tick", () => {
    const recovered = new Map<string, number>();
    const attempts = new Map<string, number>();
    expect(shouldAttemptViewRecovery("a", now, recovered, attempts)).toBe(true);
    recovered.set("a", now);
    attempts.set("a", 1);
    expect(shouldAttemptViewRecovery("a", now, recovered, attempts)).toBe(false);
    expect(
      shouldAttemptViewRecovery("a", now + PAINT_RECOVERY_COOLDOWN_MS - 1, recovered, attempts),
    ).toBe(false);
    expect(
      shouldAttemptViewRecovery("a", now + PAINT_RECOVERY_COOLDOWN_MS, recovered, attempts),
    ).toBe(true);
    // Per udid, not global: one wedged phone must not block another's recovery.
    expect(shouldAttemptViewRecovery("b", now, recovered, attempts)).toBe(true);
  });

  it("backs off doubling so a restart slower than the cooldown cannot loop", () => {
    // The regression this encodes, measured live: a producer restart takes ~44 s end to
    // end, longer than the flat 20 s floor, so every stall re-armed before the previous
    // restart had finished and the phone was torn down roughly once a minute forever.
    expect(viewRecoveryDelayMs(1)).toBe(PAINT_RECOVERY_COOLDOWN_MS);
    expect(viewRecoveryDelayMs(2)).toBe(PAINT_RECOVERY_COOLDOWN_MS * 2);
    expect(viewRecoveryDelayMs(3)).toBe(PAINT_RECOVERY_COOLDOWN_MS * 4);
    expect(viewRecoveryDelayMs(99)).toBe(PAINT_RECOVERY_MAX_MS);
    // By the second attempt the wait already exceeds how long a restart takes, which is
    // the property that breaks the loop.
    expect(viewRecoveryDelayMs(2)).toBeGreaterThan(OBSERVED_RESTART_MS);
    // And the FIRST restart is still immediate, so a one-off transient costs no delay.
    expect(shouldAttemptViewRecovery("never-tried", now, new Map(), new Map())).toBe(true);
  });

  it("keeps the stall window longer than the Rust watchdog's byte window", () => {
    // scrcpy only encodes when the screen changes, so a phone on a static screen paints
    // nothing for a while quite legitimately. A window at or below the 5s byte threshold
    // would restart healthy streams.
    expect(PAINT_STALL_MS).toBeGreaterThan(5000);
    expect(PAINT_RECOVERY_COOLDOWN_MS).toBeGreaterThan(PAINT_STALL_MS);
  });
});
