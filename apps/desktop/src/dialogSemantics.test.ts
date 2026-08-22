import { describe, expect, it } from "vitest";

/**
 * A dialog drawn over the app must say it is modal.
 *
 * Five of the eight `role="dialog"` elements in this app did not. That is not a lint nit: a
 * screen reader treats a dialog without `aria-modal` as one more region on the page, so it
 * keeps reading the twenty phone tiles behind it and the operator never learns that a dialog
 * opened, let alone that everything else is inert. The two shells below are the ones that
 * *are* inert behind — `.modal` sits inside `.modal-backdrop`, `.flow-dialog` inside
 * `.flow-dialog-layer`, and both of those cover the window.
 *
 * `FlowToolbar`'s compile preview is deliberately not here: it is a popover anchored in the
 * toolbar with nothing covering the app behind it, so it is a non-modal dialog and claiming
 * `aria-modal` would be the lie in the other direction.
 */

const sources = import.meta.glob("./**/*.tsx", { eager: true, query: "?raw", import: "default" });

/** The element that starts at `<` before `index`, up to the `>` that closes its open tag. */
function openingTag(text: string, index: number): string {
  const start = text.lastIndexOf("<", index);
  let depth = 0;
  let end = index;
  while (end < text.length) {
    const ch = text[end];
    if (ch === "{") depth += 1;
    else if (ch === "}") depth -= 1;
    else if (ch === ">" && depth === 0) break;
    end += 1;
  }
  return text.slice(start, end + 1);
}

const MODAL_SHELLS = ["modal ", "flow-dialog"];

describe("dialog semantics", () => {
  it("every dialog inside a covering shell declares aria-modal", () => {
    const missing: string[] = [];
    let checked = 0;
    for (const [path, raw] of Object.entries(sources)) {
      if (path.endsWith(".test.tsx")) continue;
      const text = raw as string;
      let at = text.indexOf('role="dialog"');
      while (at !== -1) {
        const tag = openingTag(text, at);
        if (MODAL_SHELLS.some((shell) => tag.includes(shell))) {
          checked += 1;
          if (!tag.includes("aria-modal")) {
            missing.push(`${path}:${text.slice(0, at).split("\n").length}`);
          }
        }
        at = text.indexOf('role="dialog"', at + 1);
      }
    }
    // A scan that finds nothing would pass silently, which is how this kind of test rots.
    expect(checked).toBeGreaterThanOrEqual(6);
    expect(missing).toEqual([]);
  });
});
