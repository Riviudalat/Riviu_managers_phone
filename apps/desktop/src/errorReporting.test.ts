import { describe, expect, it } from "vitest";

/**
 * A source scan, because a lint rule cannot do this one.
 *
 * Every Tauri command in this app rejects with a plain object — `{code, message}`, or for
 * `flow_validate` an *array* of them — and `String()` on either yields `[object Object]`. It is
 * silent: nothing throws, the message just says nothing. `describeError` exists for it and is
 * adopted at 100+ sites.
 *
 * Two sweeps have now missed sites. The first (§9.91) fixed four device files and left fifteen.
 * The second (§9.96) claimed the class was closed while three live instances survived in the
 * Flow dialogs — because it grepped for `String(error)`, `String(e)`, `String(err)`, i.e. **by
 * variable name**, and those three had named their binding `reason`. This test looks for the
 * *shape* instead, so the name cannot hide it again.
 *
 * Why not oxlint: `no-restricted-syntax` is not in oxlint 1.77 (checked against
 * `node_modules/oxlint/configuration_schema.json`); `no-restricted-imports` is, and is used for
 * layering. Source-scanning tests are an established tool here — `designTokens.test.ts` reads
 * the stylesheets the same way.
 */

const sources = import.meta.glob("./**/*.{ts,tsx}", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

/** Strip comments: a rule quoted in a doc block is not a use of it (learned by designTokens). */
function code(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, " ").replace(/^\s*\/\/.*$/gm, " ");
}

/**
 * `catch (name)` and `.catch((name) => …)` bindings, per file. Only these matter: `String(x)` on
 * a value that was never thrown is ordinary formatting.
 */
function caughtBindings(text: string): Set<string> {
  const names = new Set<string>();
  for (const match of text.matchAll(/\bcatch\s*\(\s*([A-Za-z_$][\w$]*)/g)) names.add(match[1]);
  for (const match of text.matchAll(/\.catch\s*\(\s*\(?\s*([A-Za-z_$][\w$]*)/g)) names.add(match[1]);
  return names;
}

const EXEMPT = new Set([
  // The one implementation allowed to reach for String, and only after exhausting the object
  // cases. Its own tests pin that it never returns "[object Object]".
  "./describeError.ts",
  // This file names the pattern in order to ban it.
  "./errorReporting.test.ts",
]);

describe("error reporting", () => {
  it("never stringifies a caught value directly", () => {
    const offenders: string[] = [];
    for (const [path, raw] of Object.entries(sources)) {
      if (EXEMPT.has(path)) continue;
      const text = code(raw);
      for (const name of caughtBindings(text)) {
        // `String(reason)`, and the ternary that spells the same bug out longhand.
        const direct = new RegExp(`String\\(\\s*${name}\\s*\\)`);
        if (direct.test(text)) offenders.push(`${path}: String(${name})`);
      }
    }
    expect(
      offenders,
      `use describeError() — String() on a Tauri rejection prints [object Object]:\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  /**
   * **A rejection reached by `.reason` is a caught value too, and this gate could not see it.**
   *
   * `Promise.allSettled` hands failures back as `{ status: "rejected", reason }` — never through
   * a `catch` binding, so `caughtBindings` finds nothing and `String(r.reason)` sailed past. Four
   * live instances were sitting in `RootTool.tsx` the whole time this gate was green: the log
   * panel an operator reads after a factory reset, an identity change or a root shell across
   * twenty phones printed `[object Object]` for every phone that failed.
   *
   * `.reason` only exists on a settled result, so the shape needs no binding analysis at all:
   * `String(anything.reason)` is this bug, always. That makes it the *third* form this class has
   * taken — first a variable name, then a name the sweep did not think of, now a property
   * access — and each time the lesson has been the same: **match the shape, and widen what the
   * scan can see.**
   */
  it("never stringifies a settled rejection either", () => {
    const offenders: string[] = [];
    for (const [path, raw] of Object.entries(sources)) {
      if (EXEMPT.has(path)) continue;
      for (const match of code(raw).matchAll(/String\(\s*([A-Za-z_$][\w$.]*)\.reason\s*\)/g)) {
        offenders.push(`${path}: String(${match[1]}.reason)`);
      }
    }
    expect(
      offenders,
      `use describeError() — a rejection is an object, and String() on it prints ` +
        `[object Object]:\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("scanned a believable number of files", () => {
    // A glob that silently matches nothing would make the assertion above vacuous — the failure
    // mode of every source-scanning test.
    expect(Object.keys(sources).length).toBeGreaterThan(100);
  });
});
