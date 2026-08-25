import { describe, expect, it, vi } from "vitest";

import {
  NurtureFailureWatch,
  failureReason,
  summariseFailures,
  type FailureReport,
} from "./nurtureFailureWatch";
import type { NurtureSessionStatus } from "./types";

function row(over: Partial<NurtureSessionStatus>): NurtureSessionStatus {
  return {
    udid: "ce0717171c2a64d50d",
    running: false,
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
    runId: "run-1",
    runSize: 14,
    phase: "finished",
    outcome: "failed",
    videoTarget: 120,
    startedAt: null,
    deadlineAt: null,
    ...over,
  };
}

describe("the reason a failure is grouped by", () => {
  it("drops the `failed —` prefix and the trailing detail", () => {
    // The detail is the part that differs between two phones that failed the same way — a
    // udid, a nested driver error — so grouping on the whole string reports one problem as
    // two.
    expect(
      failureReason(
        "failed — không mở được phiên điều khiển: startInteractionSession failed for device ce07…",
      ),
    ).toBe("không mở được phiên điều khiển");
  });

  it("keeps a message that has no detail whole", () => {
    expect(failureReason("failed — stream không có frame")).toBe("stream không có frame");
  });

  it("never returns an empty reason", () => {
    expect(failureReason("")).toBe("không rõ lý do");
    expect(failureReason("failed — ")).toBe("không rõ lý do");
  });
});

describe("the summary line", () => {
  it("is nothing for an empty batch", () => {
    expect(summariseFailures([])).toBeNull();
  });

  it("groups phones by reason, biggest group first", () => {
    const report = summariseFailures([
      row({ udid: "aaaaaa111111", lastMessage: "failed — máy đang ở màn hình khoá: StatusBar" }),
      row({ udid: "bbbbbb222222", lastMessage: "failed — máy đang ở màn hình khoá: StatusBar" }),
      row({ udid: "cccccc333333", lastMessage: "failed — stream không có frame" }),
    ]) as FailureReport;
    expect(report.title).toBe("3 máy nuôi TT bị lỗi");
    // Biggest cause first: on a fleet run it is the one worth reading.
    expect(report.detail).toBe(
      "máy đang ở màn hình khoá (2): 111111, 222222 · stream không có frame (1): 333333",
    );
  });

  it("names one phone in the singular", () => {
    const report = summariseFailures([row({ lastMessage: "failed — x" })]) as FailureReport;
    expect(report.title).toBe("1 máy nuôi TT bị lỗi");
  });
});

describe("the watch", () => {
  /**
   * The property that keeps the toast readable: `toastStore` shows four at a time, so
   * fourteen per-phone toasts would evict each other and the run's own summary.
   */
  it("reports a burst of failures as one line", () => {
    const said: FailureReport[] = [];
    const watch = new NurtureFailureWatch((report) => said.push(report));
    for (let i = 0; i < 14; i += 1) {
      watch.observe(row({ udid: `phone-${i}`, lastMessage: "failed — màn hình khoá" }));
    }
    watch.flush();
    expect(said).toHaveLength(1);
    expect(said[0].title).toBe("14 máy nuôi TT bị lỗi");
  });

  it("says nothing about a run that is merely in progress", () => {
    const said: FailureReport[] = [];
    const watch = new NurtureFailureWatch((report) => said.push(report));
    watch.observe(row({ phase: "watching", outcome: null, running: true }));
    watch.observe(row({ phase: "awaitingFeed", outcome: null, running: true }));
    watch.flush();
    expect(said).toEqual([]);
  });

  it("says nothing about a run that finished cleanly or was stopped", () => {
    const said: FailureReport[] = [];
    const watch = new NurtureFailureWatch((report) => said.push(report));
    watch.observe(row({ outcome: "done" }));
    watch.observe(row({ outcome: "partial" }));
    watch.observe(row({ outcome: "stopped" }));
    watch.flush();
    expect(said).toEqual([]);
  });

  /** The engine pushes a terminal status twice on some paths — the summary, then the row. */
  it("reports one phone once even when its terminal status arrives twice", () => {
    const said: FailureReport[] = [];
    const watch = new NurtureFailureWatch((report) => said.push(report));
    watch.observe(row({ udid: "same", lastMessage: "failed — x" }));
    watch.observe(row({ udid: "same", lastMessage: "failed — x" }));
    watch.flush();
    expect(said).toHaveLength(1);
    expect(said[0].title).toBe("1 máy nuôi TT bị lỗi");
  });

  /** A phone restarted after a failure is a new session, and may fail again out loud. */
  it("reports the same phone again after it starts a new session", () => {
    const said: FailureReport[] = [];
    const watch = new NurtureFailureWatch((report) => said.push(report));
    watch.observe(row({ udid: "same", lastMessage: "failed — x" }));
    watch.flush();
    watch.observe(row({ udid: "same", phase: "watching", outcome: null, running: true }));
    watch.observe(row({ udid: "same", lastMessage: "failed — x" }));
    watch.flush();
    expect(said).toHaveLength(2);
  });

  it("batches on a timer without a manual flush", () => {
    vi.useFakeTimers();
    try {
      const said: FailureReport[] = [];
      const watch = new NurtureFailureWatch((report) => said.push(report));
      watch.observe(row({ udid: "a", lastMessage: "failed — x" }));
      watch.observe(row({ udid: "b", lastMessage: "failed — x" }));
      expect(said).toEqual([]);
      vi.advanceTimersByTime(3_000);
      expect(said).toHaveLength(1);
      expect(said[0].title).toBe("2 máy nuôi TT bị lỗi");
    } finally {
      vi.useRealTimers();
    }
  });
});
