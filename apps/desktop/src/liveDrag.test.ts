import { describe, expect, it } from "vitest";

import { createLiveDrag, type SendTouch } from "./liveDrag";

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
