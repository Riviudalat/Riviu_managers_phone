import type { TouchAction } from "./api";
import { describeError } from "./describeError";

/// A drag the phone follows while it happens, instead of after it ends.
///
/// The old path buffered every `pointerMove` and posted one swipe on release, so the picture
/// stood still under a moving finger and then jumped. This streams the middle of the gesture
/// down the scrcpy control socket as it is made.
///
/// Three rules shape everything here:
///
///  1. **Order is not negotiable.** DOWN, then MOVEs, then UP, on a phone that has no idea
///     these came from a browser. A MOVE that overtakes its DOWN is a touch from nowhere; an
///     UP that overtakes a MOVE strands a finger down. So every send goes on one chain and
///     nothing is ever sent in parallel with anything else.
///  2. **Never queue what is already stale.** A pointer fires faster than a round trip
///     completes, and a backlog of MOVEs would make the phone lag further behind the longer
///     the drag ran. Only the newest point survives; the ones it overtook are dropped, which
///     is exactly right for a position.
///  3. **A failure is a fallback, not an error.** If the phone is not streaming, the caller
///     still has the buffered samples and the agent, which is what it always used.

export type SendTouch = (action: TouchAction, x: number, y: number) => Promise<boolean>;

export type DragOutcome =
  /// Every event reached the phone; the gesture is already complete and the caller should
  /// not post a swipe on top of it.
  | "live"
  /// Nothing usable was injected. The caller should send the gesture the old way.
  | "fallback";

export interface LiveDrag {
  /// Begin injecting, at the point the gesture *started* rather than where it is now — the
  /// caller waits for the tap threshold before deciding this is a drag, and the phone still
  /// needs the finger to land where the operator put it.
  begin(x: number, y: number): void;
  move(x: number, y: number): void;
  end(x: number, y: number): Promise<DragOutcome>;
}

/// Told once per drag when the live path gives up, and why.
///
/// Without this the fallback is invisible: the gesture still reaches the phone the old way,
/// so nothing looks broken and the live path can be dead for weeks. It cost an afternoon to
/// find that out the first time.
export type OnFallback = (reason: string) => void;

/// How long a live tap keeps the finger down.
///
/// Android calls a press a tap below `ViewConfiguration`'s 500 ms long-press threshold, and
/// views that measure a press at all want more than zero — a DOWN and UP in the same
/// millisecond is not something a finger can do, and some views ignore it. 60 ms is what a
/// quick human click measures, comfortably short of long-press and comfortably above nothing.
const TAP_HOLD_MS = 60;

/// One tap, down the control socket, without asking uiautomator2 for anything.
///
/// **This is fault tolerance, not speed.** The 55 ms the agent costs was never the problem.
/// The problem is that the agent is a single point of failure for every operator action and
/// it has a failure mode measured in tens of seconds: when something takes `UiAutomation`
/// away, a tap costs two 10 s queries and an instrumentation restart, and if the server
/// wedges into the state where it will not open a session at all it costs an error and
/// nothing else, forever (AGENTS.md §9.79). The control socket does not know what
/// `UiAutomation` is, so none of that can reach it.
///
/// Text and keys stay on the agent regardless — `INJECT_TEXT` cannot type Vietnamese
/// diacritics, and no socket makes that untrue.
///
/// Resolves `"fallback"` when the phone has no producer to touch, which is the caller's cue
/// to use the agent exactly as it always did.
export async function liveTap(
  send: SendTouch,
  x: number,
  y: number,
  onFallback?: OnFallback,
): Promise<DragOutcome> {
  try {
    if (!(await send("down", x, y))) {
      onFallback?.("tap down refused: no producer");
      return "fallback";
    }
  } catch (error) {
    onFallback?.(`tap down threw: ${describeError(error)}`);
    return "fallback";
  }
  await new Promise((resolve) => setTimeout(resolve, TAP_HOLD_MS));
  try {
    await send("up", x, y);
  } catch (error) {
    // The finger is down and this is the only thing that lifts it. Nothing left to try, and
    // reporting a fallback would be worse than useless: the caller would tap again on top of
    // a pointer that never came up.
    onFallback?.(`tap up threw, the pointer may be stuck: ${describeError(error)}`);
  }
  return "live";
}

export function createLiveDrag(send: SendTouch, onFallback?: OnFallback): LiveDrag {
  let chain: Promise<void> = Promise.resolve();
  let pending: { x: number; y: number } | null = null;
  let flushing = false;
  let began = false;
  let live = true;
  /// Whether a DOWN actually reached the phone. Not the same as `began`, which only says the
  /// caller asked: if the DOWN itself failed there is no finger on the screen, and the
  /// rescue UP in `end` would be a release of something that never touched down.
  let landed = false;
  let told = false;

  const giveUp = (reason: string) => {
    live = false;
    if (told) return;
    told = true;
    onFallback?.(reason);
  };

  const step = (action: TouchAction, x: number, y: number) => {
    chain = chain.then(async () => {
      if (!live) return;
      try {
        // `false` is the phone saying it has no producer to touch -- a fallback, not a
        // failure. A throw is a real one. Both end the live path the same way.
        if (!(await send(action, x, y))) giveUp(`${action} refused: no producer`);
        else if (action === "down") landed = true;
      } catch (error) {
        giveUp(`${action} threw: ${describeError(error)}`);
      }
    });
  };

  /// Drain whatever the pointer left behind, newest only.
  ///
  /// Re-checks `pending` after each send rather than looping over a queue: more samples
  /// arrive *while* the previous one is in flight, and this is the point at which they
  /// collapse into one.
  const flush = () => {
    if (flushing) return;
    flushing = true;
    chain = chain.then(async () => {
      while (pending && live) {
        const next = pending;
        pending = null;
        try {
          if (!(await send("move", next.x, next.y))) giveUp("move refused: no producer");
        } catch (error) {
          giveUp(`move threw: ${describeError(error)}`);
        }
      }
      flushing = false;
    });
  };

  return {
    begin(x, y) {
      if (began) return;
      began = true;
      step("down", x, y);
    },
    move(x, y) {
      if (!began || !live) return;
      pending = { x, y };
      flush();
    },
    async end(x, y) {
      if (!began) return "fallback";
      // Drain rather than discard. If the pointer's last sample is still waiting behind the
      // DOWN, dropping it would collapse the whole gesture into a single jump to the release
      // point -- and a jump has no velocity, so a flick would scroll nothing.
      flush();
      // The release point then goes as a MOVE before the UP even though the UP carries
      // coordinates of its own: some views read the release position, others integrate the
      // path, and a flick that ends off the last MOVE is a flick at the wrong speed.
      step("move", x, y);
      step("up", x, y);
      await chain;
      // A drag that died halfway has already put a finger on the phone and moved it. Lifting
      // it is not optional -- without this the phone keeps a pointer down forever and every
      // later gesture joins the abandoned one.
      if (!live) {
        if (landed) {
          try {
            await send("up", x, y);
          } catch {
            // Nothing left to try. The producer restarting will clear it.
          }
        }
        return "fallback";
      }
      return "live";
    },
  };
}

/// One phone in a group gesture: which device, and how to reach its control socket.
export interface LiveDragMember {
  udid: string;
  send: SendTouch;
}

/// Which phones took the gesture live, and which still need it sent the old way.
///
/// A split rather than one verdict, because the two halves need opposite treatment and
/// getting it wrong is silent either way: replay the gesture on a phone that already ran it
/// live and it scrolls twice, skip a phone that fell back and it does nothing at all.
export interface LiveDragSplit {
  live: string[];
  fallback: string[];
}

export interface LiveDragGroup {
  begin(x: number, y: number): void;
  move(x: number, y: number): void;
  end(x: number, y: number): Promise<LiveDragSplit>;
}

/// Drive one live drag per phone off a single pointer stream.
///
/// **One instance each, deliberately, rather than one instance fanning out inside `send`.**
/// `createLiveDrag` keeps a single `pending` point and drops whatever the newest one
/// overtook, so a phone that answers slowly simply receives fewer points. Share one instance
/// across twenty phones and that self-throttling inverts: every point waits for the slowest
/// socket, and the whole group runs at the speed of its worst device. Separate instances let
/// the fast nineteen stay smooth.
///
/// This is the difference the operator actually feels on a farm. Group control used to fall
/// back to `group_input`, which — as its own call site says — "has no path command; the
/// endpoints are what every device gets": the drag was decided at release from two points
/// and replayed as a straight line at constant speed. Besides looking nothing like the
/// finger that drew it, that shape is measurably weaker: on 19/08/2026 a straight constant
/// speed drag turned a TikTok photo carousel on 13 of 40 attempts where a shaped flick
/// managed 19 of 19.
/// A stable per-device coordinate offset within a ±`maxPx` box (A1 anti-detection jitter on
/// the live-drag path, the counterpart to what `group_input` applies on the batch path).
///
/// Deterministic per `(index, maxPx)` so a device's whole gesture shares one offset — begin,
/// move and end must agree or the path warps mid-drag. `index 0` gets `(0, 0)` so the phone
/// the operator is nominally tracking follows the true pointer and the rest scatter around
/// it. `maxPx <= 0` disables it, which is the default policy and keeps the old behaviour.
export function deviceDragOffset(index: number, maxPx: number): { dx: number; dy: number } {
  if (maxPx <= 0 || index <= 0) return { dx: 0, dy: 0 };
  const span = 2 * maxPx + 1;
  // Knuth multiplicative hash of the index → two independent offsets in [-maxPx, maxPx].
  const hash = (index * 2654435761) >>> 0;
  const dx = (hash % span) - maxPx;
  const dy = (Math.floor(hash / span) % span) - maxPx;
  return { dx, dy };
}

export function createLiveDragGroup(
  members: LiveDragMember[],
  onFallback?: OnFallback,
  offsetMaxPx = 0,
): LiveDragGroup {
  const drags = members.map((member, index) => ({
    udid: member.udid,
    offset: deviceDragOffset(index, offsetMaxPx),
    drag: createLiveDrag(member.send, (reason) => onFallback?.(`${member.udid}: ${reason}`)),
  }));
  return {
    begin(x, y) {
      for (const { drag, offset } of drags) drag.begin(x + offset.dx, y + offset.dy);
    },
    move(x, y) {
      for (const { drag, offset } of drags) drag.move(x + offset.dx, y + offset.dy);
    },
    async end(x, y) {
      const outcomes = await Promise.all(
        drags.map(async ({ udid, drag, offset }) => ({
          udid,
          outcome: await drag.end(x + offset.dx, y + offset.dy),
        })),
      );
      return {
        live: outcomes.filter((row) => row.outcome === "live").map((row) => row.udid),
        fallback: outcomes.filter((row) => row.outcome !== "live").map((row) => row.udid),
      };
    },
  };
}
