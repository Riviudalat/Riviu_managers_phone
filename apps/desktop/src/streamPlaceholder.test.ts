import { describe, expect, it } from "vitest";

import { streamPlaceholder } from "./streamPlaceholder";

const live = {
  hasView: true,
  hasGeometry: true,
  decodeFailed: false,
  tileStreamState: "live" as const,
  lastError: null,
};

describe("stream placeholder", () => {
  it("draws nothing over a picture that is running", () => {
    const { view, blocksInput } = streamPlaceholder(live);
    expect(view.kind).toBe("none");
    expect(blocksInput).toBe(false);
  });

  it.each(["parked", "sampling", "stale", undefined] as const)(
    "waits rather than accusing the operator when the state is %s",
    (tileStreamState) => {
      // `parked` is the DEFAULT a device is listed with, not a decision to leave it stopped.
      // Offering a Start button here told the operator their phone was idle and needed a
      // nudge, which was never true -- the keeper is already bringing it up.
      const { view } = streamPlaceholder({ ...live, hasView: false, tileStreamState });
      expect(view.kind).toBe("loading");
    },
  );

  it("never covers a running picture, however stale the recorded error is", () => {
    // `lastError` is sticky: the 3 s device merge re-applies the previous error whenever the
    // fresh scan has none, so a phone that failed once and recovered carries it forever.
    // Judging the error before the picture painted a full-bleed failure panel over live
    // video, permanently. The picture is the evidence; the record is a memory.
    const { view } = streamPlaceholder({ ...live, lastError: "adb: device offline" });
    expect(view.kind).toBe("none");

    // Same for a state field that has not caught up with a stream that is plainly working.
    expect(streamPlaceholder({ ...live, tileStreamState: "error" }).view.kind).toBe("none");

    // But a codec refusal still wins, because then the "picture" is a canvas that has
    // stopped being updated -- `live` lags it by design.
    expect(streamPlaceholder({ ...live, decodeFailed: true }).view.kind).toBe("failed");
  });

  it("offers a retry for a failure that a retry could plausibly clear", () => {
    const { view } = streamPlaceholder({
      ...live,
      hasView: false,
      lastError: "scrcpy-server exited before it accepted a connection",
    });
    expect(view).toMatchObject({ kind: "failed", canRetry: true });
  });

  it("refuses to offer a retry when the codec has already refused the stream", () => {
    // Every candidate was tried and rejected, so the same stream will be rejected again.
    // A button that cannot succeed is worse than no button -- AGENTS.md 9.58.
    const { view } = streamPlaceholder({ ...live, hasView: false, decodeFailed: true });
    expect(view).toMatchObject({ kind: "failed", canRetry: false });
  });

  it("treats a reported error as a failure even when the state has not caught up", () => {
    // `lastError` is set by whatever failed; `tileStreamState` by the next device poll. In
    // between, a spinner would promise a recovery nobody is attempting.
    const { view } = streamPlaceholder({
      ...live,
      hasView: false,
      tileStreamState: "sampling",
      lastError: "adb: device unauthorized",
    });
    expect(view).toMatchObject({ kind: "failed", canRetry: true });
  });

  it("blocks input on geometry, not on liveness", () => {
    // The pointer handlers map screen coordinates through the encoded frame size. A stalled
    // stream still HAS that size, so it must keep accepting gestures -- refusing them was
    // the bug. A device that never painted has no size and genuinely cannot be touched.
    expect(streamPlaceholder({ ...live, hasView: false }).blocksInput).toBe(false);
    expect(streamPlaceholder({ ...live, hasGeometry: false }).blocksInput).toBe(true);
  });
});
