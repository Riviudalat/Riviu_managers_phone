import { deviceControlBegin, deviceControlEnd } from "../../api";
import { controlFailure } from "../../describeError";

// Overlay and group control can share the same device. Only
// the last subscriber releases its native session; a reopen waits for cleanup.
const sessions = new Map<string, { count: number; ready: Promise<void> }>();
const closing = new Map<string, Promise<void>>();
function closeControlSession(udid: string, ready: Promise<void>) {
  const done = ready
    .catch(() => undefined)
    .then(() => deviceControlEnd(udid))
    .catch(() => undefined);
  closing.set(udid, done);
  void done.then(() => {
    if (closing.get(udid) === done) closing.delete(udid);
  });
  return done;
}
export function acquireControlSession(udid: string) {
  let entry = sessions.get(udid);
  if (!entry) {
    const previous = closing.get(udid);
    const ready = (async () => {
      if (previous) await previous;
      for (let attempt = 0; ; attempt++) {
        try {
          await deviceControlBegin(udid);
          return;
        } catch (error) {
          if (
            attempt >= 19 ||
            !controlFailure(error).transientIdleSweep
          )
            throw error;
          await new Promise((resolve) => setTimeout(resolve, 500));
        }
      }
    })();
    entry = { count: 0, ready };
    sessions.set(udid, entry);
    const opening = entry;
    // A second subscriber must not keep a rejected begin cached forever. Retire
    // only this generation; the next begin waits for its native cleanup.
    void ready.catch(() => {
      if (sessions.get(udid) !== opening) return;
      closeControlSession(udid, ready);
      sessions.delete(udid);
    });
  }
  entry.count++;
  let released = false;
  let releaseDone: Promise<void> | undefined;
  return {
    ready: entry.ready,
    release() {
      if (released) return releaseDone;
      released = true;
      if (--entry.count) return;
      // A failed generation was already retired. Its last subscriber must not
      // close a newer session acquired by another overlay in the meantime.
      if (sessions.get(udid) !== entry) return;
      sessions.delete(udid);
      const done = closeControlSession(udid, entry.ready);
      releaseDone = done;
      return done;
    },
  };
}

// A confirmed roster disconnect retires the shared generation even while another
// overlay subscribes. Its old releases must never close the replacement session.
export function invalidateDisconnectedControlSession(udid: string) {
  const entry = sessions.get(udid);
  if (!entry) return;
  sessions.delete(udid);
  return closeControlSession(udid, entry.ready);
}
