import { describe, expect, it } from "vitest";
import {
  budgetCeiling,
  budgetFree,
  budgetUsed,
  clampToBudget,
  fitToBudget,
  isOverBudget,
  isRateEnabled,
  type BudgetValues,
} from "./nurtureBudget";

function rates(like: number, comment: number, follow: number, frenzy: number): BudgetValues {
  return { likeProb: like, commentProb: comment, followProb: follow, frenzyProb: frenzy };
}

describe("the shared 100% budget", () => {
  it("adds up what the four rates spend, and what is left", () => {
    expect(budgetUsed(rates(60, 20, 10, 0))).toBe(90);
    expect(budgetFree(rates(60, 20, 10, 0))).toBe(10);
  });

  /**
   * The operator's own example: one rate at 90 leaves 10 for the other three, so three at 3
   * leaves one able to reach 4.
   */
  it("lets the other three share what one leaves, to the last point", () => {
    const withNinety = rates(90, 0, 0, 0);
    expect(budgetCeiling(withNinety, "commentProb")).toBe(10);

    const shared = rates(90, 3, 3, 0);
    expect(budgetCeiling(shared, "frenzyProb")).toBe(4);
    expect(clampToBudget(shared, "frenzyProb", 4)).toBe(4);
    // And not one point further.
    expect(clampToBudget(shared, "frenzyProb", 5)).toBe(4);
  });

  it("counts a rate's own value inside its own ceiling", () => {
    // Otherwise every slider would be out of its own range the moment it was drawn.
    const values = rates(40, 30, 20, 10);
    expect(budgetCeiling(values, "likeProb")).toBe(40);
    expect(clampToBudget(values, "likeProb", 40)).toBe(40);
  });

  it("clamps rather than taking percent off a neighbour", () => {
    const values = rates(50, 30, 10, 0);
    expect(clampToBudget(values, "likeProb", 95)).toBe(60);
    // The neighbours are untouched — this function answers about one rate only.
    expect(values).toEqual(rates(50, 30, 10, 0));
  });

  it("floors a fraction, because the engine's rates are whole percent", () => {
    expect(clampToBudget(rates(0, 0, 0, 0), "likeProb", 3.9)).toBe(3);
  });

  it("refuses to go below zero, and treats junk as no change", () => {
    expect(clampToBudget(rates(10, 0, 0, 0), "likeProb", -5)).toBe(0);
    expect(clampToBudget(rates(10, 0, 0, 0), "likeProb", Number.NaN)).toBe(10);
  });

  it("reads a missing rate as spending nothing rather than as NaN", () => {
    expect(budgetUsed({ likeProb: 20 })).toBe(20);
    expect(budgetCeiling({ likeProb: 20 }, "commentProb")).toBe(80);
  });
});

describe("a config saved before the budget existed", () => {
  /** The measured shape of the operator's own settings: 100 + 28 + 3 + 0 = 131. */
  const legacy = rates(100, 28, 3, 0);

  it("is recognised as over budget, and free never reads negative", () => {
    expect(isOverBudget(legacy)).toBe(true);
    expect(budgetUsed(legacy)).toBe(131);
    expect(budgetFree(legacy)).toBe(0);
  });

  /**
   * Without this the panel is a dead end: every ceiling is already 0, so no slider can be
   * dragged anywhere and the operator cannot get back under the budget by hand.
   */
  it("comes down to exactly 100, taking from the largest first", () => {
    const fitted = fitToBudget(legacy);
    expect(budgetUsed(fitted)).toBe(100);
    // The shape of what they had survives: comment and follow keep their tuned numbers.
    expect(fitted).toEqual(rates(69, 28, 3, 0));
  });

  it("leaves a config that already fits exactly as it is", () => {
    const fine = rates(60, 20, 10, 5);
    expect(fitToBudget(fine)).toEqual(fine);
    expect(isOverBudget(fine)).toBe(false);
  });

  it("can bring an all-maxed config down without going negative", () => {
    const fitted = fitToBudget(rates(100, 100, 100, 100));
    expect(budgetUsed(fitted)).toBe(100);
    expect(Object.values(fitted).every((value) => value >= 0)).toBe(true);
  });
});

/**
 * A rate whose switch is off spends nothing.
 *
 * This is not a courtesy to the operator, it is what the engine does:
 * `NurtureSettings::into_effective` zeroes `like_prob` when `like_enabled` is false and the
 * loop only ever sees that copy. Charging the budget for it would be charging for posts that
 * provably never happen.
 */
describe("a rate that is switched off", () => {
  it("spends nothing, and hands its percent back to the other three", () => {
    const allOn = rates(35, 0, 3, 6);
    expect(budgetUsed(allOn)).toBe(44);

    const followOff = { ...allOn, followEnabled: false };
    expect(budgetUsed(followOff)).toBe(41);
    expect(budgetFree(followOff)).toBe(59);
    // Thích may now take the 3 that Follow gave back.
    expect(budgetCeiling(allOn, "likeProb")).toBe(91);
    expect(budgetCeiling(followOff, "likeProb")).toBe(94);
  });

  it("keeps its own number draggable, because that is what the switch promises", () => {
    // Follow is off and the rate that is on spends all 100, so there is nothing left for it.
    // Held to the ceiling it could not be moved off 0 at all — and an operator who switched a
    // feature off to tune it later would find the number they were protecting taken away.
    const full = { ...rates(100, 0, 3, 0), followEnabled: false };
    expect(budgetCeiling(full, "followProb")).toBe(0);
    expect(clampToBudget(full, "followProb", 40)).toBe(40);
    // Still a percentage, so still 0..100.
    expect(clampToBudget(full, "followProb", 140)).toBe(100);
    // And a rate beside it that *is* on is still held to the budget — to 0, here.
    expect(clampToBudget(full, "commentProb", 40)).toBe(0);
  });

  it("is over budget the moment its switch comes back on over a number that does not fit", () => {
    // The one state a switch click can create, and the reason the panel says it rather than
    // quietly trimming the row the operator just enabled.
    const parked = { ...rates(97, 0, 40, 0), followEnabled: false };
    expect(isOverBudget(parked)).toBe(false);

    const switchedOn = { ...parked, followEnabled: true };
    expect(isOverBudget(switchedOn)).toBe(true);
    expect(budgetUsed(switchedOn)).toBe(137);
  });

  it("is not trimmed by the fix button, which would cost a number and free nothing", () => {
    const over = { ...rates(97, 0, 40, 0), commentEnabled: false, commentProb: 30 };
    expect(budgetUsed(over)).toBe(137);

    const fitted = fitToBudget(over);
    // Taken from Thích, the largest *enabled* rate. Bình luận keeps its parked 30.
    expect(fitted).toEqual(rates(60, 30, 40, 0));
    expect(budgetUsed({ ...over, ...fitted })).toBe(100);
  });

  it("reads an absent switch as on, the same as the panel and the stored row do", () => {
    expect(isRateEnabled(rates(10, 0, 0, 0), "likeProb")).toBe(true);
    expect(isRateEnabled({ likeProb: 10, likeEnabled: false }, "likeProb")).toBe(false);
  });
});
