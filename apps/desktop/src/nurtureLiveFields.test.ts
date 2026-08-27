import { describe, expect, it } from "vitest";
import { LIVE_TUNABLE_FIELDS, RESTART_REQUIRED_REASONS } from "./types";

/**
 * **The two lists that answer the same question have to disagree about nothing.**
 *
 * `LIVE_TUNABLE_FIELDS` says which settings a running session picks up on its next post.
 * `RESTART_REQUIRED_REASONS` says which ones it will not, with the sentence to show. They are
 * written independently, and nothing connected them — so a field could appear in both, and the
 * operator would get a badge saying "restart to apply this" on a value the loop had already
 * absorbed. That is not a cosmetic contradiction: it is the app telling somebody to stop twenty
 * phones for no reason.
 *
 * This also gives `LIVE_TUNABLE_FIELDS` a second reader, which matters for a reason that has
 * nothing to do with correctness. It had exactly one — a Rust test that `include_str!`s
 * `types.ts` — so every dead-export scan reports it as the safest deletion in the file, and
 * deleting it turns `cargo test` red from a change made entirely in TypeScript. A constant whose
 * only consumer is invisible from its own language is a trap regardless of whether it is right.
 */
describe("what a running nurture session absorbs", () => {
  it("never lists a field as both live-tunable and restart-required", () => {
    const contradictory = Object.keys(RESTART_REQUIRED_REASONS).filter((field) =>
      LIVE_TUNABLE_FIELDS.has(field as never),
    );
    expect(
      contradictory,
      "these would show a restart badge for a value the loop already picks up",
    ).toEqual([]);
  });

  /** Anti-rot: two empty sets are trivially disjoint. */
  it("has both lists populated, or the check above proves nothing", () => {
    expect(LIVE_TUNABLE_FIELDS.size).toBeGreaterThan(20);
    expect(Object.keys(RESTART_REQUIRED_REASONS).length).toBeGreaterThan(5);
  });

  /**
   * Every restart reason is a sentence an operator can act on, not a field name echoed back.
   *
   * Cheap to assert and it catches the one way this map rots: a field added with a placeholder.
   */
  it("gives every restart-required field a real sentence", () => {
    for (const [field, reason] of Object.entries(RESTART_REQUIRED_REASONS)) {
      expect(reason.length, `${field} has no usable reason`).toBeGreaterThan(15);
      expect(reason, `${field}'s reason is just its own name`).not.toBe(field);
    }
  });
});
