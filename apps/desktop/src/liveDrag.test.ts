import { describe, expect, it } from "vitest";

import {
  createLiveDrag,
  createLiveDragGroup,
  deviceDragOffset,
  liveTap,
  type SendTouch,
} from "./liveDrag";

/// A sink that records what it was asked to send and lets the test decide when each call
/// finishes, so the ordering guarantees can be tested against a slow phone rather than an
/// instant one -- which is the only case where they can be broken.
function recorder(behaviour?: (action: string, index: number) => boolean | Error) {
  const calls: string[] = [];
  const gates: Array<() => void> = [];
  let index = 0;
  const send: SendTouch = (action, x, y) => {
    const at = index++;
    calls.push(`${action} ${x},${y}`);
    return new Promise((resolve, reject) => {
      gates.push(() => {
        const verdict = behaviour?.(action, at) ?? true;
        if (verdict instanceof Error) reject(verdict);
        else resolve(verdict);
      });
    });
  };
  const releaseAll = async () => {
    // Each release can queue the next call, so keep draining until nothing new appears.
    while (gates.length) gates.shift()?.();
    await Promise.resolve();
    while (gates.length) {
      while (gates.length) gates.shift()?.();
      await new Promise((r) => setTimeout(r, 0));
    }
  };
  return { calls, releaseAll, send };
}

describe("live drag", () => {
  it("never lets a move overtake its down, however fast the pointer is", async () => {
    const { calls, releaseAll, send } = recorder();
    const drag = createLiveDrag(send);

    // Every one of these is issued before the DOWN has been acknowledged.
    drag.begin(10, 10);
    drag.move(11, 11);
    drag.move(12, 12);
    const done = drag.end(13, 13);
    await releaseAll();

    expect(await done).toBe("live");
    expect(calls[0]).toBe("down 10,10");
    expect(calls[calls.length - 1]).toBe("up 13,13");
    // A finger from nowhere, or one left behind, is what a wrong order looks like on the phone.
    expect(calls.filter((c) => c.startsWith("down"))).toHaveLength(1);
    expect(calls.filter((c) => c.startsWith("up"))).toHaveLength(1);
  });

  it("drops the samples a newer one overtook rather than queueing them", async () => {
    const { calls, releaseAll, send } = recorder();
    const drag = createLiveDrag(send);

    drag.begin(0, 0);
    // Ten samples arrive while the DOWN is still in flight. Sending all ten would put the
    // phone further behind the finger with every one; only the last is still true.
    for (let i = 1; i <= 10; i += 1) drag.move(i, i);
    const done = drag.end(99, 99);
    await releaseAll();
    await done;

    const moves = calls.filter((c) => c.startsWith("move"));
    // The eight that were already stale when the next arrived are gone...
    expect(moves.length).toBeLessThan(10);
    expect(moves).not.toContain("move 3,3");
    // ...but the newest surviving sample is not, and neither is the release. Collapsing to
    // the release alone would leave the phone a single jump with no velocity, and a flick
    // that carries no velocity scrolls nothing.
    expect(moves).toContain("move 10,10");
    expect(moves).toContain("move 99,99");
    expect(calls.indexOf("move 10,10")).toBeLessThan(calls.indexOf("move 99,99"));
  });

  it("ends with the release position as a move, so a flick keeps its velocity", async () => {
    const { calls, releaseAll, send } = recorder();
    const drag = createLiveDrag(send);
    drag.begin(0, 0);
    drag.move(5, 5);
    const done = drag.end(40, 80);
    await releaseAll();
    await done;

    expect(calls).toContain("move 40,80");
    expect(calls.indexOf("move 40,80")).toBeLessThan(calls.indexOf("up 40,80"));
  });

  it("asks for a fallback when the phone has no producer, and lifts nothing", async () => {
    // `false` on the DOWN means the phone is not streaming. No finger ever landed, so a
    // rescue UP would be a release of a touch that never happened.
    const { calls, releaseAll, send } = recorder((action) => action !== "down");
    const drag = createLiveDrag(send);
    drag.begin(1, 1);
    drag.move(2, 2);
    const done = drag.end(3, 3);
    await releaseAll();

    expect(await done).toBe("fallback");
    expect(calls.filter((c) => c.startsWith("up"))).toHaveLength(0);
  });

  it("lifts the finger when the socket dies mid-drag", async () => {
    // The DOWN landed and then the socket went. Without a rescue UP the phone keeps a
    // pointer down forever and every later gesture joins the abandoned one.
    const { calls, releaseAll, send } = recorder((action) =>
      action === "move" ? new Error("socket closed") : true,
    );
    const drag = createLiveDrag(send);
    drag.begin(1, 1);
    drag.move(2, 2);
    const done = drag.end(3, 3);
    await releaseAll();

    expect(await done).toBe("fallback");
    expect(calls.filter((c) => c.startsWith("up"))).toHaveLength(1);
  });

  it("taps by pressing and releasing the same point, in that order", async () => {
    const calls: string[] = [];
    const send: SendTouch = async (action, x, y) => {
      calls.push(`${action} ${x},${y}`);
      return true;
    };
    expect(await liveTap(send, 30, 40)).toBe("live");
    expect(calls).toEqual(["down 30,40", "up 30,40"]);
  });

  it("leaves the tap to the agent when the phone has no producer", async () => {
    // And sends nothing else: an UP with no DOWN behind it is a release of a touch that
    // never happened, and the caller is about to tap properly through uiautomator2 anyway.
    const calls: string[] = [];
    const reasons: string[] = [];
    const send: SendTouch = async (action) => {
      calls.push(action);
      return false;
    };
    expect(await liveTap(send, 1, 1, (reason) => reasons.push(reason))).toBe("fallback");
    expect(calls).toEqual(["down"]);
    expect(reasons).toHaveLength(1);
  });

  it("reports a stuck pointer rather than asking for a second tap on top of it", async () => {
    // The DOWN landed and the UP did not. Falling back here would tap again over a pointer
    // that never came up, so this stays "live" and says what happened instead.
    const reasons: string[] = [];
    const send: SendTouch = async (action) => {
      if (action === "up") throw new Error("socket closed");
      return true;
    };
    expect(await liveTap(send, 1, 1, (reason) => reasons.push(reason))).toBe("live");
    expect(reasons[0]).toContain("stuck");
  });

  it("does nothing at all when the caller never began", async () => {
    // A tap keeps the proven agent path: it never calls begin, and must not leave a stray
    // event on the control socket.
    const { calls, send } = recorder();
    const drag = createLiveDrag(send);
    drag.move(5, 5);
    expect(await drag.end(5, 5)).toBe("fallback");
    expect(calls).toHaveLength(0);
  });
});

describe("live drag across a group", () => {
  it("puts the gesture on every phone in the selection", async () => {
    const a = recorder();
    const b = recorder();
    const group = createLiveDragGroup([
      { udid: "phone-a", send: a.send },
      { udid: "phone-b", send: b.send },
    ]);

    group.begin(10, 10);
    group.move(10, 40);
    const settled = group.end(10, 80);
    // Alternating rounds, because `recorder` was written for one phone: draining A can
    // queue B's next call and the other way round, so one pass each leaves gates behind.
    for (let round = 0; round < 8; round += 1) {
      await a.releaseAll();
      await b.releaseAll();
    }
    const split = await settled;

    expect(split.live.sort()).toEqual(["phone-a", "phone-b"]);
    expect(split.fallback).toEqual([]);
    // The shape reaches both, not just the endpoints: this is the whole difference from
    // `group_input`, which has no path command and sends the two ends.
    expect(a.calls[0]).toBe("down 10,10");
    expect(b.calls[0]).toBe("down 10,10");
    expect(a.calls.at(-1)).toBe("up 10,80");
    expect(b.calls.at(-1)).toBe("up 10,80");
  });

  it("names the phones that fell back instead of hiding them behind the ones that worked", async () => {
    // The split is the point. Replaying the gesture on a phone that already ran it live
    // scrolls twice; skipping one that fell back does nothing at all. A single verdict for
    // the whole group has to be wrong in one of those two directions.
    const good = recorder();
    const dead = recorder((action) => (action === "down" ? false : true));
    const group = createLiveDragGroup([
      { udid: "works", send: good.send },
      { udid: "no-producer", send: dead.send },
    ]);

    group.begin(5, 5);
    group.move(5, 50);
    const settled = group.end(5, 90);
    for (let round = 0; round < 8; round += 1) {
      await good.releaseAll();
      await dead.releaseAll();
    }
    const split = await settled;

    expect(split.live).toEqual(["works"]);
    expect(split.fallback).toEqual(["no-producer"]);
  });

  it("lets a fast phone stay smooth while a slow one drops points", async () => {
    // One instance per phone, not one fanning out inside `send`. Shared, every point would
    // wait for the slowest socket and the group would run at the speed of its worst device.
    const fast = recorder();
    const slow = recorder();
    const group = createLiveDragGroup([
      { udid: "fast", send: fast.send },
      { udid: "slow", send: slow.send },
    ]);

    group.begin(0, 0);
    await fast.releaseAll();
    // Only the fast phone has acknowledged its DOWN; the slow one is still holding it.
    for (let y = 10; y <= 60; y += 10) {
      group.move(0, y);
      await fast.releaseAll();
    }

    const fastMoves = fast.calls.filter((call) => call.startsWith("move")).length;
    const slowMoves = slow.calls.filter((call) => call.startsWith("move")).length;
    expect(fastMoves).toBeGreaterThan(slowMoves);
  });
});

describe("deviceDragOffset", () => {
  it("leaves the first device on the true pointer", () => {
    expect(deviceDragOffset(0, 8)).toEqual({ dx: 0, dy: 0 });
  });

  it("is a no-op when the policy sets no jitter", () => {
    expect(deviceDragOffset(3, 0)).toEqual({ dx: 0, dy: 0 });
    expect(deviceDragOffset(3, -5)).toEqual({ dx: 0, dy: 0 });
  });

  it("keeps every offset inside the ±maxPx box", () => {
    const maxPx = 6;
    for (let index = 1; index < 50; index += 1) {
      const { dx, dy } = deviceDragOffset(index, maxPx);
      expect(Math.abs(dx)).toBeLessThanOrEqual(maxPx);
      expect(Math.abs(dy)).toBeLessThanOrEqual(maxPx);
    }
  });

  it("is stable for a given index so a drag does not warp mid-gesture", () => {
    expect(deviceDragOffset(7, 5)).toEqual(deviceDragOffset(7, 5));
  });

  it("does not hand every device the same offset", () => {
    const seen = new Set(
      Array.from({ length: 12 }, (_, i) => JSON.stringify(deviceDragOffset(i + 1, 6))),
    );
    // The whole point is that twenty phones are not pixel-identical; a couple of collisions
    // are fine, but they must not all land on one point.
    expect(seen.size).toBeGreaterThan(4);
  });
});

describe("live drag group offset", () => {
  it("tracks the pointer on the first phone and jitters the rest by a fixed per-device amount", async () => {
    const a = recorder();
    const b = recorder();
    const maxPx = 6;
    const off = deviceDragOffset(1, maxPx);
    const group = createLiveDragGroup(
      [
        { udid: "lead", send: a.send },
        { udid: "follower", send: b.send },
      ],
      undefined,
      maxPx,
    );

    group.begin(100, 100);
    const settled = group.end(100, 160);
    for (let round = 0; round < 8; round += 1) {
      await a.releaseAll();
      await b.releaseAll();
    }
    await settled;

    // The lead phone (index 0) gets the exact pointer.
    expect(a.calls[0]).toBe("down 100,100");
    expect(a.calls.at(-1)).toBe("up 100,160");
    // The follower gets the same offset on both endpoints — the gesture is shifted, not warped.
    expect(b.calls[0]).toBe(`down ${100 + off.dx},${100 + off.dy}`);
    expect(b.calls.at(-1)).toBe(`up ${100 + off.dx},${160 + off.dy}`);
  });
});
