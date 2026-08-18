import { describe, expect, it } from "vitest";

import appCssRaw from "./App.css?raw";
import fontsCssRaw from "./assets/fonts/fonts.css?raw";
import indexCssRaw from "./index.css?raw";

// Read through Vite rather than through `node:fs`: this project's app tsconfig
// declares only `vite/client`, so a test that reaches for `readFileSync` compiles
// under vitest and then breaks `npm run build`, which runs `tsc -b` first.
const shipped = Object.keys(
  import.meta.glob("./assets/fonts/*.woff2", { eager: false }),
).map((path) => path.replace("./assets/fonts/", ""));

/** Strip comments before scanning: a token named in a comment is not a use of it.
 *  Learned immediately — the comment explaining a token that had just been removed
 *  was itself read as a reference to it, and the test failed on its own footnote. */
const code = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, " ");

describe("the token layer", () => {
  // The point of the layer is one place to change a step, and a step that exists at
  // all. A half-declared scale is worse than none: callers reach for `var(--space-7)`,
  // get nothing, and the declaration silently does nothing. That is not theoretical —
  // the first run of this test found `var(--surface-2)` in App.css, used once and
  // declared nowhere, so a screenshot preview had no background at all.
  it("declares every token the rest of the CSS reaches for", () => {
    const declared = new Set(
      [...code(indexCssRaw).matchAll(/^\s*(--[a-z0-9-]+):/gm)].map((m) => m[1]),
    );
    const used = new Set(
      [
        ...code(indexCssRaw).matchAll(/var\((--[a-z0-9-]+)/g),
        ...code(appCssRaw).matchAll(/var\((--[a-z0-9-]+)/g),
      ].map((m) => m[1]),
    );
    const missing = [...used].filter((name) => !declared.has(name));
    expect(missing, `used but never declared: ${missing.join(", ")}`).toEqual([]);
  });

  it("keeps the two tokens the old stylesheet was built on at their old values", () => {
    // `--radius` and `--shadow` are referenced throughout 3000+ lines written before
    // the scale existed. Growing a scale *around* them is safe; redefining them
    // restyles every surface at once, which is not a token change but a redesign
    // wearing one.
    expect(indexCssRaw).toMatch(/--radius:\s*8px/);
    expect(indexCssRaw).toMatch(/--shadow:\s*0 4px 16px/);
  });

  it("has a step for each axis a layout needs", () => {
    for (const token of [
      "--space-1",
      "--space-4",
      "--text-base",
      "--leading-normal",
      "--weight-semibold",
      "--radius-sm",
      "--shadow-lg",
      "--duration-fast",
    ]) {
      expect(indexCssRaw, `${token} is missing`).toContain(`${token}:`);
    }
  });
});

describe("the bundled fonts", () => {
  // Invisible in development and total in the field: a desktop app that fetches its
  // typeface from fonts.googleapis.com at startup has no typeface on a machine with
  // no internet, and every size in a dense UI shifts under the fallback. It also
  // tells Google each time the app is opened.
  it("never come from the network", () => {
    for (const [name, css] of [
      ["index.css", indexCssRaw],
      ["App.css", appCssRaw],
      ["fonts.css", fontsCssRaw],
    ] as const) {
      expect(css, `${name} imports over the network`).not.toMatch(
        /@import\s+url\(\s*["']?https?:/,
      );
      expect(css, `${name} points a font at a remote host`).not.toMatch(
        /src:\s*url\(\s*["']?https?:/,
      );
    }
  });

  it("ship the file behind every face they declare", () => {
    const referenced = [...fontsCssRaw.matchAll(/url\(\.\/([^)]+)\)/g)].map((m) => m[1]);
    expect(referenced.length).toBeGreaterThan(0);
    const absent = referenced.filter((file) => !shipped.includes(file));
    expect(absent, `declared but not shipped: ${absent.join(", ")}`).toEqual([]);
  });

  it("declare a weight range, because the files are variable fonts", () => {
    // The trap this pins: a `wght@400;500;600;700` request to Google returns what
    // looks like four files and is one, delivered four times byte for byte. Written
    // out as four faces at four point weights, the app shipped 36 KB four times over
    // and pinned the `wght` axis — and with `font-synthesis: none` set in index.css
    // there is no faux bold to cover for it, so every semibold label in the app would
    // have quietly rendered at regular.
    const faces = [...fontsCssRaw.matchAll(/font-family: '([^']+)'[\s\S]*?font-weight: ([^;]+);/g)];
    expect(faces.length).toBeGreaterThan(0);
    for (const [, family, weight] of faces) {
      expect(weight.trim(), `${family} is pinned to one weight`).toMatch(/^\d+ \d+$/);
    }
    expect(fontsCssRaw).toMatch(/font-weight: 400 700;/);
    expect(indexCssRaw).toMatch(/font-synthesis: none/);
  });

  it("ship one file per subset, not the same bytes under several names", () => {
    const referenced = new Set(
      [...fontsCssRaw.matchAll(/url\(\.\/([^)]+)\)/g)].map((m) => m[1]),
    );
    expect(shipped.length).toBe(referenced.size);
  });

  it("carry Vietnamese, because the whole interface is written in it", () => {
    expect(fontsCssRaw).toMatch(/vietnamese/);
    expect(shipped.some((file) => file.includes("vietnamese"))).toBe(true);
  });
});
