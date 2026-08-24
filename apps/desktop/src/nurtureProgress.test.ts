import { describe, expect, it } from "vitest";

import {
  currentRun,
  deviceProgress,
  deviceProgressLabel,
  governingBound,
  minutesLeft,
} from "./nurtureProgress";
import type { NurtureSessionStatus } from "./types";

/**
 * These mirror `progress_tests` in `crates/core/src/types.rs` case for case.
 *
 * The rules are implemented twice — once in Rust as the reference, once here because the
 * clock bound means the bar has to advance between status pushes — so they are pinned twice.
 * A rule changed on one side and not the other is a bar that disagrees with the engine about
 * whether a run is finished.
 */

const T0 = Date.UTC(2026, 7, 23, 10, 0, 0);
const at = (seconds: number) => T0 + seconds * 1000;

function row(over: Partial<NurtureSessionStatus> = {}): NurtureSessionStatus {
  return {
    udid: "fixture",
    running: true,
    videosDone: 0,
    swipeAttempts: 0,
    likeAttempts: 0,
    commentAttempts: 0,
    followAttempts: 0,
    likes: 0,
    comments: 0,
    follows: 0,
    lastMessage: "",
    sessionPromptTokens: 0,
    sessionCompletionTokens: 0,
    runId: null,
    runSize: 0,
    phase: "watching",
    outcome: null,
    videoTarget: 0,
    startedAt: null,
    deadlineAt: null,
    ...over,
  };
}

/** Mid-run: 120 posts wanted, a three-hour horizon. */
const running = (videosDone: number) =>
  row({
    videosDone,
    videoTarget: 120,
    startedAt: new Date(at(0)).toISOString(),
    deadlineAt: new Date(at(3 * 3600)).toISOString(),
  });

describe("one device's progress", () => {
  it("is unknown, not zero, when there is nothing to divide by", () => {
    // An empty track reads as a stall, and a phone that has not started is not stalled.
    const queued = row({ phase: "queued" });
    expect(deviceProgress(queued, at(0))).toBeNull();
    expect(governingBound(queued, at(0))).toBeNull();
  });

  it("follows the video count while it is ahead of the clock", () => {
    // 60/120 posts = 50%, against 6 of 180 minutes = 3.3%.
    const status = running(60);
    expect(deviceProgress(status, at(6 * 60))).toBeCloseTo(0.5, 10);
    expect(governingBound(status, at(6 * 60))).toBe("videos");
  });

  it("follows the clock when the clock is ahead", () => {
    // The reading a count-only bar gets wrong: twelve minutes from the end, at 40% of posts.
    const status = running(48);
    const nearlyOver = at(168 * 60);
    expect(deviceProgress(status, nearlyOver)).toBeCloseTo(168 / 180, 10);
    expect(governingBound(status, nearlyOver)).toBe("clock");
    expect(minutesLeft(status, nearlyOver)).toBe(12);
  });

  it("reads full on a terminal row whatever the counters say", () => {
    // A run that stopped at 40 of 120 is finished, not 33% done.
    const status = row({ ...running(40), phase: "finished", outcome: "partial", running: false });
    expect(deviceProgress(status, at(60))).toBe(1);
    expect(governingBound(status, at(60))).toBeNull();
  });

  it("reads full on a failed row too, because the slot is settled", () => {
    // Deliberate: the bar means "settled", the colour carries the verdict. A failed phone
    // frozen at 0% would look like one that never started.
    const status = row({ ...running(0), phase: "finished", outcome: "failed", running: false });
    expect(deviceProgress(status, at(0))).toBe(1);
  });

  it("never exceeds one, even long past the deadline", () => {
    expect(deviceProgress(running(0), at(10 * 3600))).toBe(1);
  });

  it("never goes negative when the clock is before the start", () => {
    // Clocks are not monotone across a machine's time changes, and a negative fraction
    // renders as a bar growing leftwards.
    expect(deviceProgress(running(0), at(-600))).toBe(0);
  });

  it("ignores a deadline at or before the start rather than dividing by zero", () => {
    const status = row({
      videosDone: 30,
      videoTarget: 120,
      startedAt: new Date(at(0)).toISOString(),
      deadlineAt: new Date(at(0)).toISOString(),
    });
    expect(deviceProgress(status, at(60))).toBeCloseTo(0.25, 10);
    expect(governingBound(status, at(60))).toBe("videos");
  });

  it("tracks the video count when there is no deadline", () => {
    const status = row({ videosDone: 15, videoTarget: 60 });
    expect(deviceProgress(status, at(60))).toBeCloseTo(0.25, 10);
    expect(governingBound(status, at(60))).toBe("videos");
  });

  /** The lead threshold: a clock barely ahead of a zero count must not steal the label. */
  it("keeps the video count on the label until the clock is meaningfully ahead", () => {
    const justStarted = running(0);
    // Two minutes into a three-hour horizon: the clock is ahead of 0/120 but only by 1%.
    expect(governingBound(justStarted, at(2 * 60))).toBe("videos");
    expect(deviceProgressLabel(justStarted, at(2 * 60))).toBe("0/120 video");
    // Twenty minutes in, still nothing watched: now the clock genuinely leads.
    expect(governingBound(justStarted, at(20 * 60))).toBe("clock");
  });

  it("still fills the bar from the clock even while the label says videos", () => {
    // The fill takes the plain maximum; only the sentence waits for the lead.
    const justStarted = running(0);
    expect(deviceProgress(justStarted, at(2 * 60))).toBeCloseTo(2 / 180, 10);
    expect(governingBound(justStarted, at(2 * 60))).toBe("videos");
  });

  it("tracks the clock when there is no video target", () => {
    const status = row({
      videoTarget: 0,
      startedAt: new Date(at(0)).toISOString(),
      deadlineAt: new Date(at(100)).toISOString(),
    });
    expect(deviceProgress(status, at(25))).toBeCloseTo(0.25, 10);
    expect(governingBound(status, at(25))).toBe("clock");
  });

  it("never goes backwards as a run proceeds", () => {
    let last = 0;
    for (let minute = 0; minute < 180; minute += 1) {
      const value = deviceProgress(running(Math.floor(minute / 2)), at(minute * 60));
      expect(value).not.toBeNull();
      expect(value as number).toBeGreaterThanOrEqual(last);
      last = value as number;
    }
  });

  it("survives an unparseable timestamp instead of rendering NaN", () => {
    const status = row({ videosDone: 6, videoTarget: 12, startedAt: "not a date" });
    expect(deviceProgress(status, at(0))).toBeCloseTo(0.5, 10);
  });
});

describe("the label beside a device's bar", () => {
  it("counts videos while the count governs", () => {
    expect(deviceProgressLabel(running(42), at(60))).toBe("42/120 video");
  });

  it("says the time left when the clock governs, and does not print the count", () => {
    // Printing "48/120 video" here would name the bound that is *not* going to end the run.
    expect(deviceProgressLabel(running(48), at(168 * 60))).toBe("còn ~12 phút");
  });

  it("names the verdict and keeps the counters on a terminal row", () => {
    // "lỗi · 0/120 video" and "lỗi · 96/120 video" are very different facts.
    const failed = row({ ...running(0), phase: "finished", outcome: "failed", running: false });
    expect(deviceProgressLabel(failed, at(0))).toBe("lỗi · 0/120 video");
    const done = row({ ...running(120), phase: "finished", outcome: "done", running: false });
    expect(deviceProgressLabel(done, at(0))).toBe("xong · 120/120 video");
  });
});

describe("a whole run", () => {
  const inRun = (over: Partial<NurtureSessionStatus>) =>
    row({
      runId: "run-1",
      runSize: 4,
      videoTarget: 100,
      startedAt: new Date(at(0)).toISOString(),
      ...over,
    });

  it("is null when no row carries a run id", () => {
    // Every row from before run ids existed, and every row the idle sweep wrote.
    expect(currentRun([row({}), row({})], at(0))).toBeNull();
  });

  it("divides by the run's own size, not by the rows present", () => {
    // Two of four phones reported; the other two never produced a row. A denominator of 2
    // would read 100% on a run that is half missing.
    const progress = currentRun(
      [
        inRun({ udid: "a", videosDone: 100, phase: "finished", outcome: "done", running: false }),
        inRun({ udid: "b", videosDone: 100, phase: "finished", outcome: "done", running: false }),
      ],
      at(60),
    );
    expect(progress?.size).toBe(4);
    expect(progress?.fraction).toBeCloseTo(0.5, 10);
    expect(progress?.done).toBe(2);
  });

  it("counts a failed phone as settled but reports it separately", () => {
    // The property that keeps a full bar honest: 2 done + 2 failed is 100% *settled*, and
    // the failure count and the red tail are what stop it reading as success.
    const progress = currentRun(
      [
        inRun({ udid: "a", phase: "finished", outcome: "done", running: false }),
        inRun({ udid: "b", phase: "finished", outcome: "done", running: false }),
        inRun({ udid: "c", phase: "finished", outcome: "failed", running: false }),
        inRun({ udid: "d", phase: "finished", outcome: "failed", running: false }),
      ],
      at(60),
    );
    expect(progress?.fraction).toBe(1);
    expect(progress?.failed).toBe(2);
    expect(progress?.failedFraction).toBeCloseTo(0.5, 10);
    expect(progress?.running).toBe(0);
  });

  it("ignores rows from an earlier run", () => {
    // The bug this filter exists for: the status list is never pruned, so an unfiltered sum
    // counts finished phones from previous runs and jumps backwards when one restarts.
    const progress = currentRun(
      [
        row({ udid: "old", runId: "run-0", runSize: 1, videosDone: 100, videoTarget: 100,
              phase: "finished", outcome: "done", running: false,
              startedAt: new Date(at(-10_000)).toISOString() }),
        inRun({ udid: "new", videosDone: 0, running: true }),
      ],
      at(60),
    );
    expect(progress?.runId).toBe("run-1");
    expect(progress?.rows).toHaveLength(1);
  });

  it("prefers the run that still has a live session", () => {
    const progress = currentRun(
      [
        row({ udid: "later-but-done", runId: "run-2", runSize: 1, phase: "finished",
              outcome: "done", running: false,
              startedAt: new Date(at(9_000)).toISOString() }),
        inRun({ udid: "live", running: true, phase: "watching" }),
      ],
      at(9_100),
    );
    expect(progress?.runId).toBe("run-1");
  });

  it("counts a queued phone as occupying its slot at zero", () => {
    const progress = currentRun(
      [
        inRun({ udid: "a", phase: "queued", videoTarget: 0, startedAt: null }),
        inRun({ udid: "b", videosDone: 100, phase: "finished", outcome: "done", running: false }),
      ],
      at(60),
    );
    expect(progress?.fraction).toBeCloseTo(0.25, 10);
    expect(progress?.running).toBe(1);
  });
});
