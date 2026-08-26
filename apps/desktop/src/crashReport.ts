import { describeError } from "./describeError";

/**
 * What kind of failure this was, in the words the log uses.
 *
 * `render` is a React subtree that threw and was caught by the error boundary; `error` is a
 * synchronous throw that reached `window`; `unhandledrejection` is a promise nobody caught.
 */
export type CrashKind = "render" | "error" | "unhandledrejection";

export type CrashReporter = (kind: CrashKind, message: string, source?: string) => void;

/**
 * Where an `ErrorEvent` says it came from, as one string, or nothing.
 *
 * Returns `undefined` rather than a half-filled string when the browser did not say: a source
 * of `":0:0"` reads like a real location and points at nothing.
 */
export function crashSource(event: {
  filename?: string;
  lineno?: number;
  colno?: number;
}): string | undefined {
  const file = event.filename;
  if (!file) return undefined;
  const line = event.lineno ?? 0;
  const column = event.colno ?? 0;
  return line > 0 ? `${file}:${line}:${column}` : file;
}

/**
 * **Send every uncaught frontend failure somewhere it can be read.**
 *
 * `index.html` already listened for `error` and `unhandledrejection` before this existed, but
 * both handlers only wrote into `#boot-marker` — and `main.tsx` *removes* that element as soon
 * as React mounts. So the app had global error handling for roughly its first second of life
 * and none at all for the rest of it: after mount, `getElementById` returned null and the
 * handler did nothing. A throw inside a render unmounted the React tree and left a blank
 * window with an empty log.
 *
 * Three properties that matter more than the wiring:
 *
 * * **`describeError`, not `String`.** A Tauri command rejects with `{ code, message }`, and
 *   `String` on that yields `[object Object]`. The boot-path handlers in `index.html` had
 *   exactly that bug, which is the same one already fixed in 47 other places.
 * * **The reporter must never throw or reject.** It is called *from* the unhandled-rejection
 *   handler; a reporter that rejects would raise a new unhandled rejection and report itself
 *   forever. Callers pass a reporter that swallows its own failures, and this function also
 *   wraps each call so a synchronous throw cannot escape.
 * * **Listeners are removed on teardown**, so tests can install and uninstall without leaking
 *   into each other.
 */
export function installCrashReporting(
  report: CrashReporter,
  target: Pick<EventTarget, "addEventListener" | "removeEventListener"> = window,
): () => void {
  const safely = (kind: CrashKind, message: string, source?: string) => {
    try {
      report(kind, message, source);
    } catch {
      // Nowhere left to report a failure in the reporting path, and throwing here would
      // become the next event this very handler receives.
    }
  };

  const onError = (event: Event) => {
    const error = event as ErrorEvent;
    safely("error", describeError(error.error ?? error.message), crashSource(error));
  };
  const onRejection = (event: Event) => {
    const rejection = event as PromiseRejectionEvent;
    safely("unhandledrejection", describeError(rejection.reason));
  };

  target.addEventListener("error", onError);
  target.addEventListener("unhandledrejection", onRejection);
  return () => {
    target.removeEventListener("error", onError);
    target.removeEventListener("unhandledrejection", onRejection);
  };
}

/**
 * How long the loading marker waits for React before it admits something is wrong.
 *
 * A mount takes tens of milliseconds even on a cold farm machine. Ten seconds is far past any
 * honest mount and short enough that an operator has not yet decided the app is broken and
 * gone to do something else.
 */
export const BOOT_MOUNT_DEADLINE_MS = 10_000;

/**
 * What the loading marker should do on this frame.
 *
 * **Pure, because the loop it replaces could not be tested and was wrong.** `main.tsx` used to
 * poll `rootElement.childElementCount > 0` on `requestAnimationFrame` with no bound at all. If
 * the first render threw, `#root` stayed empty forever and the loop spun forever with it -- the
 * app sat on "Loading Riviu Manager..." and nothing ever said why. Worse, once React *had*
 * mounted and then unmounted (a render throw with no boundary above it), the count fell back to
 * zero, so the same loop would have kept running against an app that was already gone.
 */
export function bootMarkerVerdict(mounted: boolean, elapsedMs: number): "wait" | "clear" | "stuck" {
  if (mounted) return "clear";
  return elapsedMs >= BOOT_MOUNT_DEADLINE_MS ? "stuck" : "wait";
}
