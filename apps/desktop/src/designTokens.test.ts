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

describe("keyboard focus", () => {
  // Before the design pass this app had exactly one `outline` rule in 3300 lines, so
  // tabbing moved an invisible cursor: nothing showed which of twenty device tiles,
  // which toolbar button or which field was about to take the Enter key. On a tool
  // somebody drives all day, that is a defect and not a polish item.
  it("is visible on anything that can take it", () => {
    expect(code(indexCssRaw)).toMatch(/:focus-visible\s*\{[^}]*outline:/);
    // Read the selector list itself rather than probing it with a built regex: the
    // whole value of this assertion is naming which element fell out of the list.
    const ring = code(indexCssRaw).match(/:where\(([^)]*(?:\[[^\]]*\])?[^)]*)\):focus-visible/);
    expect(ring, "there is no :where(...):focus-visible rule at all").not.toBeNull();
    const covered = ring![1];
    for (const el of ["a", "button", "input", "select", "textarea"]) {
      expect(
        covered.split(",").map((part) => part.trim()),
        `${el} is not covered by the focus ring`,
      ).toContain(el);
    }
  });

  it("is never switched off without a replacement", () => {
    // `outline: none` is how focus styling dies: someone removes the ring a mouse
    // click left behind instead of reaching for :focus-visible, and keyboard users
    // lose it too. If a rule ever needs it, it must draw its own.
    const offenders = [...code(appCssRaw).matchAll(/([^{}]*)\{[^}]*outline:\s*(none|0)[^}]*\}/g)]
      .filter((m) => !/box-shadow|border-color/.test(m[0]))
      .map((m) => m[1].trim().replace(/\s+/g, " ").slice(-60));
    expect(offenders, `kills focus with nothing in its place: ${offenders.join(", ")}`).toEqual([]);
  });
});

describe("the type scale", () => {
  // 27 distinct font sizes between 0.55rem and 1.2rem existed here, most of them one
  // or two rules apart — nobody chose them, they drifted one edit at a time, and the
  // result is an interface where nothing lines up because nothing shares a size.
  it("leaves no raw font size anywhere in the stylesheets", () => {
    for (const [name, css] of [
      ["index.css", indexCssRaw],
      ["App.css", appCssRaw],
    ] as const) {
      const raw = [...code(css).matchAll(/font-size:\s*([0-9][^;]*);/g)].map((m) => m[1]);
      expect(raw, `${name} still sets sizes off the scale: ${raw.join(", ")}`).toEqual([]);
    }
  });

  it("uses steps that exist", () => {
    const declared = new Set(
      [...code(indexCssRaw).matchAll(/^\s*(--text-[a-z0-9]+):/gm)].map((m) => m[1]),
    );
    const used = new Set(
      [...code(appCssRaw).matchAll(/font-size:\s*var\((--text-[a-z0-9]+)\)/g)].map((m) => m[1]),
    );
    expect(used.size).toBeGreaterThan(3);
    for (const step of used) expect(declared, `${step} is not declared`).toContain(step);
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

describe("colours", () => {
  // The stylesheet carried 79 distinct hardcoded colours: twelve greys, fifteen reds,
  // ten blues, all one-offs. Everything with an exact counterpart in the palette, and
  // every role that appeared more than once, is now a token — carrying the identical
  // value, so this named what was there instead of restyling it.
  //
  // What is left is the tail: near-duplicate greys and reds that differ by a few
  // percent. Merging those changes what the operator sees on pages no screenshot
  // covers, so it is a decision to take with the app open, not a sweep to run blind.
  // This is a ratchet, not a target — the number may fall and must never rise.
  const CEILING = 60;

  it("adds no new hardcoded colour", () => {
    const literals = new Set(
      (code(appCssRaw).match(/#[0-9a-fA-F]{3,8}/g) ?? []).map((c) => c.toLowerCase()),
    );
    expect(
      literals.size,
      `one-off colours went up to ${literals.size}; use a token in index.css`,
    ).toBeLessThanOrEqual(CEILING);
  });

  it("keeps the stylesheet itself free of them where a token exists", () => {
    // These eight had an exact match in the palette all along, which is why their
    // removal changed nothing on screen and why coming back would be a mistake.
    for (const gone of ["#fff", "#ffffff", "#303133", "#f5f7fa", "#dcdfe6", "#909399", "#67c23a", "#f56c6c"]) {
      expect(
        code(appCssRaw).toLowerCase(),
        `${gone} is back, and it has a token`,
      ).not.toContain(gone);
    }
  });
});
