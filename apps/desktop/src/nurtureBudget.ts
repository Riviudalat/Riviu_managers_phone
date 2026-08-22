/**
 * The four nurture interaction rates share one 100% budget.
 *
 * **What this changes, said plainly.** In the engine the four are *independent* dice rolled
 * per post: `likeProb` is "what share of posts get a like", `followProb` is "what share get
 * the author followed, on its own", and so on — nothing ever added them up, which is why a
 * saved config could read 100 + 0 + 3 + 0 = 103. Sharing a budget makes them shares of the
 * same hundred posts instead, so **an existing config whose rates sum past 100 has to come
 * down before it can be saved.** That is the operator's decision and it was asked for; the
 * engine is untouched, this is a stricter way to configure it.
 *
 * The rule, and the whole of it: a rate may rise to whatever the other three leave free. One
 * at 90 leaves 10 for the other three — three at 3 leaves one able to reach 4.
 *
 * **A switched-off rate spends nothing.** Not a courtesy — it is what the engine does:
 * `NurtureSettings::into_effective` (crates/core/src/types.rs) zeroes `like_prob` when
 * `like_enabled` is false, and the loop only ever sees that zeroed copy. So a budget that
 * charged for a switch that is off would be charging for posts that provably never happen.
 * The number itself is kept and stays editable, which is the whole point of having a switch
 * separate from a percentage.
 *
 * Pure, because every bug a shared budget can have is arithmetic: a slider that lets the sum
 * past 100, one that cannot reach the last free point, one that silently rewrites a
 * neighbour, one that charges for a feature that is off. None of that needs a DOM to prove.
 */

/** The four rates that share the budget, in the order the panel lists them. */
export const BUDGET_KEYS = ["likeProb", "commentProb", "followProb", "frenzyProb"] as const;

export type BudgetKey = (typeof BUDGET_KEYS)[number];

/** The switch that decides whether a rate spends anything at all. */
const SWITCH_OF: Record<BudgetKey, "likeEnabled" | "commentEnabled" | "followEnabled" | "frenzyEnabled"> = {
  likeProb: "likeEnabled",
  commentProb: "commentEnabled",
  followProb: "followEnabled",
  frenzyProb: "frenzyEnabled",
};

/** Total percent the four rates may spend between them. */
export const BUDGET_TOTAL = 100;

export type BudgetValues = Record<BudgetKey, number>;

/** Rates plus the switches that gate them. Both halves are optional, as they are on the wire. */
export type BudgetInput = Partial<BudgetValues> &
  Partial<Record<(typeof SWITCH_OF)[BudgetKey], boolean>>;

/** Reads a rate defensively: a missing or non-finite value spends nothing. */
function rate(values: BudgetInput, key: BudgetKey): number {
  const value = values[key];
  return typeof value === "number" && Number.isFinite(value) ? Math.max(0, value) : 0;
}

/**
 * Whether this rate's switch is on. Absent reads as **on**, matching both the panel's
 * `settings.likeEnabled ?? true` and the Rust `#[serde(default = "yes")]` for a row stored
 * before the switches existed.
 */
export function isRateEnabled(values: BudgetInput, key: BudgetKey): boolean {
  return values[SWITCH_OF[key]] !== false;
}

/** What this rate actually costs the budget: its value, or nothing while its switch is off. */
function spend(values: BudgetInput, key: BudgetKey): number {
  return isRateEnabled(values, key) ? rate(values, key) : 0;
}

/** How much of the budget the four rates spend together. Switched-off rates spend nothing. */
export function budgetUsed(values: BudgetInput): number {
  return BUDGET_KEYS.reduce((sum, key) => sum + spend(values, key), 0);
}

/** What is left unspent. Never negative, so a legacy over-budget config reads 0 rather than -3. */
export function budgetFree(values: BudgetInput): number {
  return Math.max(0, BUDGET_TOTAL - budgetUsed(values));
}

/**
 * The highest this one rate may be set to: everything the other three are not using.
 *
 * Includes the rate's own current value, because a slider's own position must be inside its
 * own range — a max computed from the total *including* itself would make every slider
 * unable to stay where it is. For a switched-off rate this is what it *would* occupy if the
 * switch came back on, which is exactly the number the operator needs to see while deciding.
 */
export function budgetCeiling(values: BudgetInput, key: BudgetKey): number {
  const others = BUDGET_KEYS.filter((other) => other !== key).reduce(
    (sum, other) => sum + spend(values, other),
    0,
  );
  return Math.max(0, BUDGET_TOTAL - others);
}

/**
 * The value this rate actually takes when the operator asks for `next`.
 *
 * Clamped, never redistributed: taking percent away from a neighbour the operator did not
 * touch is the behaviour that makes a shared budget feel possessed. Fractions are floored to
 * whole percent — the engine's rates are integers and a slider that reports 3.5 writes a
 * number the backend then rounds behind the operator's back.
 *
 * A **switched-off** rate is not held to the ceiling, only to 0..100. It is spending nothing,
 * so there is nothing to clamp it against, and clamping it anyway would break the promise the
 * switch makes: that turning a feature off keeps its tuned number intact and editable. If it
 * no longer fits when the switch comes back on, [`isOverBudget`] says so.
 */
export function clampToBudget(values: BudgetInput, key: BudgetKey, next: number): number {
  if (!Number.isFinite(next)) return rate(values, key);
  const ceiling = isRateEnabled(values, key) ? budgetCeiling(values, key) : BUDGET_TOTAL;
  return Math.max(0, Math.min(ceiling, Math.floor(next)));
}

/**
 * True when a config spends more than the budget.
 *
 * Two ways in: a config saved before the budget existed, and a switch turned back on over a
 * number that no longer fits.
 */
export function isOverBudget(values: BudgetInput): boolean {
  return budgetUsed(values) > BUDGET_TOTAL;
}

/**
 * Bring an over-budget config down to exactly the budget, largest rate first.
 *
 * For the one case the operator cannot fix with a slider: over the budget, every ceiling sits
 * at or below where its rate already is, so nothing can be dragged up and the panel would be
 * a dead end. Taking from the largest keeps the shape of what they had — a 131 of 100/28/3/0
 * becomes 69/28/3/0 rather than four equal quarters.
 *
 * Only switched-off rates are left alone: they are not part of the sum, so trimming them
 * would destroy a tuned number without freeing a single percent.
 */
export function fitToBudget(values: BudgetInput): BudgetValues {
  const next = {
    likeProb: rate(values, "likeProb"),
    commentProb: rate(values, "commentProb"),
    followProb: rate(values, "followProb"),
    frenzyProb: rate(values, "frenzyProb"),
  };
  // The switches come from `values`, not from `next`, which carries the rates alone.
  let excess = budgetUsed(values) - BUDGET_TOTAL;
  if (excess <= 0) return next;
  const order = BUDGET_KEYS.filter((key) => isRateEnabled(values, key)).sort(
    (a, b) => rate(next, b) - rate(next, a),
  );
  for (const key of order) {
    if (excess <= 0) break;
    const take = Math.min(excess, rate(next, key));
    next[key] = rate(next, key) - take;
    excess -= take;
  }
  return next;
}
