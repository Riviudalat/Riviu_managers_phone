import { describe, expect, it } from "vitest";

/**
 * **The path from "something threw" to "somebody can read it" must stay connected.**
 *
 * It was disconnected in four places at once, and every one of them was silent:
 *
 * 1. No error boundary anywhere, so a render throw unmounted the app to a blank window.
 * 2. `index.html` listened for `error` and `unhandledrejection`, but both handlers wrote into
 *    `#boot-marker` — which `main.tsx` removes as soon as React mounts. Global error handling
 *    that works for the app's first second and for none of the rest of its life.
 * 3. No bridge at all from the frontend to the log: nothing in `src-tauri` accepted a report.
 * 4. The boot marker's `requestAnimationFrame` loop had no bound, so a failed first render
 *    left the app on "Loading Riviu Manager..." forever.
 *
 * Each link is cheap to break again by accident — deleting a wrapper, renaming a command,
 * "simplifying" the loop back. So each link is asserted here rather than trusted.
 *
 * Read from disk rather than through `import.meta.glob` because two of the files this has to
 * see (`index.html`, and the Rust command table) live outside `src/`. That is not incidental:
 * the `String(event.reason)` bug in `index.html` survived a sweep that closed 47 other sites
 * precisely because the scan only looked inside `src/`.
 */
/**
 * Read through vite rather than through `node:fs`.
 *
 * `tsconfig.app.json` sets `types: ["vite/client"]` and `@types/node` is not installed, so
 * `readFileSync` type-checks nowhere -- and this file shipped once already with `tsc -b` red
 * because the typecheck was not re-run after it was added. `import.meta.glob` needs no new
 * dependency, and both files outside `src/` are still inside vite's root.
 */
const sources = import.meta.glob(
  [
    "./main.tsx",
    "./api.ts",
    "./crashReport.ts",
    "../index.html",
    "../src-tauri/src/lib.rs",
    "../src-tauri/src/commands/system.rs",
  ],
  { eager: true, query: "?raw", import: "default" },
) as Record<string, string>;

function read(relative: string): string {
  const key = Object.keys(sources).find((path) => path.endsWith(relative));
  if (!key) {
    throw new Error(`${relative} was not loaded; known: ${Object.keys(sources).join(", ")}`);
  }
  return sources[key];
}

describe("the path from a frontend crash to the log", () => {
  it("wraps the app in an error boundary", () => {
    const main = read("main.tsx");
    expect(main).toContain("ErrorBoundary");
    expect(main).toMatch(/<ErrorBoundary[\s\S]*<App\s*\/>[\s\S]*<\/ErrorBoundary>/);
  });

  it("installs the window-level handlers at startup", () => {
    expect(read("main.tsx")).toContain("installCrashReporting(");
  });

  /** Both events, not just the one that is easier to remember. */
  it("listens for a throw and for a rejection", () => {
    const reporter = read("crashReport.ts");
    expect(reporter).toContain('addEventListener("error"');
    expect(reporter).toContain('addEventListener("unhandledrejection"');
    expect(reporter).toContain('removeEventListener("error"');
    expect(reporter).toContain('removeEventListener("unhandledrejection"');
  });

  /** The bridge has to exist on both sides of the wire, under the same name. */
  it("carries the report across to a registered Rust command", () => {
    expect(read("api.ts")).toContain('invoke<void>("log_frontend_error"');
    const lib = read("lib.rs");
    expect(lib).toContain("commands::log_frontend_error,");
    expect(read("system.rs")).toContain(
      "pub fn log_frontend_error(",
    );
  });

  /**
   * The boot marker must reach a verdict rather than poll forever.
   *
   * Asserting on `bootMarkerVerdict` rather than on the absence of a loop: the loop is fine,
   * the *unbounded* loop was the bug, and the bound lives in that function.
   */
  it("bounds the wait for React to mount", () => {
    expect(read("main.tsx")).toContain("bootMarkerVerdict(");
  });

  /**
   * **`index.html` is inside the blast radius of the `[object Object]` rule too.**
   *
   * Its inline script cannot import `describeError`, so it restates the rule by hand — but it
   * must not go back to `String(event.reason)`, which is what it did until now and what made
   * a refused folder read as `[object Object]` on the boot path.
   */
  it("keeps the boot-path handlers off String(reason)", () => {
    const html = read("index.html");
    expect(html).toContain('addEventListener("error"');
    expect(html).toContain('addEventListener("unhandledrejection"');
    expect(html).not.toMatch(/String\(\s*event\.reason/);
    expect(html).toContain("cause.message");
  });
});
