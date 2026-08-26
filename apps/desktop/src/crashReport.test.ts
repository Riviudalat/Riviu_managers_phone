import { describe, expect, it, vi } from "vitest";
import {
  bootMarkerVerdict,
  BOOT_MOUNT_DEADLINE_MS,
  crashSource,
  installCrashReporting,
  type CrashKind,
} from "./crashReport";

/** A minimal EventTarget stand-in, so a test never installs handlers on the real window. */
function fakeTarget() {
  const listeners = new Map<string, EventListener[]>();
  return {
    addEventListener(type: string, listener: EventListener) {
      listeners.set(type, [...(listeners.get(type) ?? []), listener]);
    },
    removeEventListener(type: string, listener: EventListener) {
      listeners.set(type, (listeners.get(type) ?? []).filter((entry) => entry !== listener));
    },
    fire(type: string, event: unknown) {
      for (const listener of listeners.get(type) ?? []) listener(event as Event);
    },
    count(type: string) {
      return (listeners.get(type) ?? []).length;
    },
  };
}

describe("reporting a frontend crash", () => {
  /**
   * **A rejected Tauri command must not report as `[object Object]`.**
   *
   * Commands reject with `{ code, message }`. The handlers this replaces — the inline pair in
   * `index.html` — used `String(event.reason)`, which is the exact bug already fixed in 47
   * other places in this app and missed here because the scan only read `src/`.
   */
  it("describes a rejected command by its message, not by String()", () => {
    const target = fakeTarget();
    const seen: [CrashKind, string, string?][] = [];
    installCrashReporting((...args) => seen.push(args), target);

    target.fire("unhandledrejection", {
      reason: { code: "OperationFailed", message: "Permission denied" },
    });

    expect(seen).toHaveLength(1);
    expect(seen[0][0]).toBe("unhandledrejection");
    expect(seen[0][1]).toBe("Permission denied");
    expect(seen[0][1]).not.toContain("[object Object]");
  });

  /** A named code earns its place in the line; a generic one does not. */
  it("keeps a code that means something", () => {
    const target = fakeTarget();
    const seen: string[] = [];
    installCrashReporting((_kind, message) => seen.push(message), target);

    target.fire("unhandledrejection", {
      reason: { code: "DeviceBusy", message: "máy đang chạy nuôi" },
    });

    expect(seen[0]).toBe("DeviceBusy: máy đang chạy nuôi");
  });

  it("reports a synchronous throw with the place it came from", () => {
    const target = fakeTarget();
    const seen: [CrashKind, string, string?][] = [];
    installCrashReporting((...args) => seen.push(args), target);

    target.fire("error", {
      error: new Error("deviceControlBegin is not a function"),
      message: "Uncaught TypeError",
      filename: "/src/App.tsx",
      lineno: 412,
      colno: 7,
    });

    expect(seen[0][0]).toBe("error");
    expect(seen[0][1]).toBe("deviceControlBegin is not a function");
    expect(seen[0][2]).toBe("/src/App.tsx:412:7");
  });

  /**
   * **The reporter throwing must not become the next thing reported.**
   *
   * This handler is what receives unhandled rejections, so a reporter that throws — or that
   * returns a rejecting promise — would report itself, forever, as fast as the event loop
   * allows. That is the one failure mode that turns a diagnostic into an outage.
   */
  it("swallows a reporter that throws instead of looping on itself", () => {
    const target = fakeTarget();
    const exploding = vi.fn(() => {
      throw new Error("the log bridge is down");
    });
    installCrashReporting(exploding, target);

    expect(() =>
      target.fire("unhandledrejection", { reason: "something failed" }),
    ).not.toThrow();
    expect(exploding).toHaveBeenCalledTimes(1);
  });

  it("removes both listeners when torn down", () => {
    const target = fakeTarget();
    const stop = installCrashReporting(() => {}, target);
    expect(target.count("error")).toBe(1);
    expect(target.count("unhandledrejection")).toBe(1);
    stop();
    expect(target.count("error")).toBe(0);
    expect(target.count("unhandledrejection")).toBe(0);
  });
});

describe("naming where a crash came from", () => {
  /**
   * A location the browser did not supply must read as absent, not as line zero.
   *
   * `":0:0"` looks like a real place and points at nothing, which sends the reader looking for
   * a file that was never named.
   */
  it("says nothing rather than pointing at line zero", () => {
    expect(crashSource({})).toBeUndefined();
    expect(crashSource({ filename: "", lineno: 12 })).toBeUndefined();
    expect(crashSource({ filename: "/src/main.tsx", lineno: 0, colno: 0 })).toBe("/src/main.tsx");
  });
});

describe("the loading marker", () => {
  /**
   * **A mount that never happens has to end in a sentence, not in a spin.**
   *
   * The loop this replaces polled `childElementCount > 0` on `requestAnimationFrame` with no
   * bound. A first render that threw left `#root` empty forever, so the app sat on
   * "Loading Riviu Manager..." for as long as the operator was willing to look at it, and the
   * loop kept running behind it. That is the literal report this work came from.
   */
  it("gives up and says so once the deadline passes", () => {
    expect(bootMarkerVerdict(false, 0)).toBe("wait");
    expect(bootMarkerVerdict(false, BOOT_MOUNT_DEADLINE_MS - 1)).toBe("wait");
    expect(bootMarkerVerdict(false, BOOT_MOUNT_DEADLINE_MS)).toBe("stuck");
  });

  /** A mounted app clears the marker, however long it took to get there. */
  it("clears as soon as React has rendered something", () => {
    expect(bootMarkerVerdict(true, 0)).toBe("clear");
    expect(bootMarkerVerdict(true, BOOT_MOUNT_DEADLINE_MS * 10)).toBe("clear");
  });

  /**
   * The deadline has to be far past an honest mount, or a slow machine reads as a broken one.
   */
  it("waits far longer than a mount can honestly take", () => {
    expect(BOOT_MOUNT_DEADLINE_MS).toBeGreaterThanOrEqual(5_000);
  });
});
