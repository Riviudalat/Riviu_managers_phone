import { describe, expect, it } from "vitest";

import interactionCssRaw from "../../styles/interaction.css?raw";
import detailSourceRaw from "./InteractionCampaignDetail.tsx?raw";

/**
 * Every class the web-lookup panel puts in the markup has a rule behind it.
 *
 * **What this checks, and what it deliberately does not.** The seven tests in
 * `InteractionTargetNotes.test.tsx` prove the panel renders the right *text* and keeps its three
 * states apart. None of them can see a stylesheet: jsdom applies no author CSS, so a `className`
 * with no matching rule renders as an unstyled table and every assertion still passes. That is
 * the failure this test exists for — a class in the markup that nothing styles, which on a table
 * means an unreadable wall of borderless text.
 *
 * It does **not** check that the result looks good. Pixel aesthetics need an eye, and pretending
 * a snapshot nobody opens is the same thing would be worse than saying so: see the note in
 * AGENTS.md §9.115 about why this panel was not verified by driving the app — the Interaction
 * screen has "Chạy ngay" on it, and that has posted a real comment to a customer's post once.
 *
 * Read through Vite (`?raw`) rather than `node:fs`, for the reason `designTokens.test.ts` gives:
 * this app's tsconfig declares only `vite/client`, so `readFileSync` passes under vitest and
 * then breaks `npm run build`.
 */

/** A token named in a comment is not a use of it — the same trap `designTokens.test.ts` hit. */
const withoutComments = (source: string) =>
  source.replace(/\/\*[\s\S]*?\*\//g, " ").replace(/^\s*\/\/.*$/gm, " ");

const css = withoutComments(interactionCssRaw);
const tsx = withoutComments(detailSourceRaw);

/** Class names the panel's own markup asks for, taken from the source rather than a list. */
const usedClasses = [
  ...tsx.matchAll(/className=(?:"([^"]+)"|\{`([^`]+)`\}|\{"([^"]+)"\})/g),
]
  .flatMap((match) => (match[1] ?? match[2] ?? match[3] ?? "").split(/\s+/))
  .map((name) => name.trim())
  .filter((name) => name.startsWith("interaction-notes") || name.startsWith("interaction-note-"));

describe("the web-lookup panel's styles", () => {
  it("finds the panel's classes in the markup at all", () => {
    // A scanner that reads nothing passes every assertion below it. Four prefixed classes: the
    // section, the table, and the two cell states (`refused`, `blank`). The row-state class is
    // `is-refused`, which carries no prefix and is checked on its own below.
    expect(new Set(usedClasses).size).toBeGreaterThanOrEqual(4);
  });

  it("has a rule behind every class the panel uses", () => {
    const unstyled = [...new Set(usedClasses)].filter(
      (name) => !new RegExp(`\\.${name}(?![\\w-])`).test(css),
    );
    expect(unstyled, "classes in the markup with no rule in interaction.css").toEqual([]);
  });

  it("declares no panel rule the markup never asks for", () => {
    // The other direction, and it is not symmetry for its own sake: a rule left behind after a
    // class is renamed is dead weight that reads as intent, and the next person styles against
    // it.
    const declared = [
      ...css.matchAll(/\.(interaction-notes[\w-]*|interaction-note-[\w-]+)/g),
    ].map((match) => match[1]);
    const orphans = [...new Set(declared)].filter((name) => !usedClasses.includes(name));
    expect(orphans, "rules in interaction.css that no markup uses").toEqual([]);
  });

  it("scopes the row-state class instead of styling `is-refused` globally", () => {
    // `is-refused` is a generic name, so its rule has to be scoped to this table or it reaches
    // every row in the app that happens to use the same word. Checked separately because the
    // class carries no `interaction-note` prefix and the scan above deliberately ignores it.
    expect(tsx).toContain('"is-refused"');
    expect(css).toMatch(/\.interaction-notes-table\s+tr\.is-refused/);
  });

  it("styles the table through tokens, not hard-coded colours", () => {
    // The app is themed by the token layer; a literal hex here is a cell that ignores the
    // theme. Scoped to this panel's own block so it says nothing about the rest of the file.
    const block = css.slice(css.indexOf(".interaction-notes"));
    expect(block).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(block).toMatch(/var\(--/);
  });
});
