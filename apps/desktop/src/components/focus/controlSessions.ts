import { deviceControlBegin, deviceControlEnd } from "../../api";
import { describeError } from "../../describeError";

// Overlay and group control can share the same device. Only
// the last subscriber releases its native session; a reopen waits for cleanup.
const sessions = new Map<string, { count: number; ready: Promise<void> }>();
const closing = new Map<string, Promise<void>>();
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
            !/DeviceBusy.*IdleSweep/.test(describeError(error))
          )
            throw error;
          await new Promise((resolve) => setTimeout(resolve, 500));
        }
      }
    })();
    entry = { count: 0, ready };
    sessions.set(udid, entry);
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
      sessions.delete(udid);
      const done = entry.ready
        .catch(() => undefined)
        .then(() => deviceControlEnd(udid))
        .catch(() => undefined);
      closing.set(udid, done);
      releaseDone = done;
      void done.then(() => {
        if (closing.get(udid) === done) closing.delete(udid);
      });
      return done;
    },
  };
}
