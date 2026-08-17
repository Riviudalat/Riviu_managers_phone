import type { TouchAction } from "./api";

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
        giveUp(`${action} threw: ${String(error)}`);
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
          giveUp(`move threw: ${String(error)}`);
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
