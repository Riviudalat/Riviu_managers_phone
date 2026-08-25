import { describe, expect, it } from "vitest";

/**
 * A button that disables itself has to enable itself again.
 *
 * Every "busy" flag in this app follows the same shape: set it true, do the slow thing, set
 * it false. Miss the reset on one path and the control is dead for the rest of the session —
 * no error, no toast, just a button that stops working, which is the hardest kind of bug to
 * report and the easiest to introduce.
 *
 * The discipline is already there: all thirty-nine sites reset, thirty-eight of them inside a
 * `finally`. This holds it, rather than replacing thirty-nine working call sites with a hook
 * for the sake of uniformity — the plan proposed `useAsyncAction`, and measuring first showed
 * there was no bug to fix, only a shape to keep.
 */

const sources = import.meta.glob("./**/*.tsx", { eager: true, query: "?raw", import: "default" });

const BUSY_NAME = /busy|saving|running|loading|pending|working|installing|checking|scanning|testing|sending|starting/i;

/** The function body containing `index`, by brace depth from the nearest `=> {` or `) {`. */
function enclosingBody(text: string, index: number): string {
  let start = index;
  let depth = 0;
  while (start > 0) {
    const ch = text[start];
    if (ch === "}") depth += 1;
    else if (ch === "{") {
      if (depth === 0) break;
      depth -= 1;
    }
    start -= 1;
  }
  let end = index;
  depth = 0;
  while (end < text.length) {
    const ch = text[end];
    if (ch === "{") depth += 1;
    else if (ch === "}") {
      if (depth === 0) break;
      depth -= 1;
    }
    end += 1;
  }
  return text.slice(start, end + 1);
}

describe("busy flags", () => {
  it("every flag that is switched on is switched off in the same function", () => {
    const stuck: string[] = [];
    let checked = 0;
    for (const [path, raw] of Object.entries(sources)) {
      if (path.endsWith(".test.tsx")) continue;
      const text = raw as string;
      for (const m of text.matchAll(/\bset([A-Z]\w*)\(true\)/g)) {
        const setter = m[1];
        if (!BUSY_NAME.test(setter)) continue;
        checked += 1;
        const body = enclosingBody(text, m.index ?? 0);
        if (!body.includes(`set${setter}(false)`)) {
          const line = text.slice(0, m.index).split("\n").length;
          stuck.push(`${path}:${line} set${setter}`);
        }
      }
    }
    // A scan that matches nothing would pass, which is how a source test rots.
    expect(checked).toBeGreaterThanOrEqual(30);
    expect(stuck).toEqual([]);
  });
});
