import { describeError } from "./describeError";

/**
 * Why the phones that failed, failed — grouped, named, and in the phone's own words.
 *
 * **A count is not a reason.** Every fan-out in the group tools reports `ok/total`, which is
 * honest as far as it goes, and then fills the detail line with a guess: "Máy còn lại cần Riviu
 * helper", "Máy còn lại chưa root". Those are plausible and frequently wrong, and they crowd out
 * the sentence the backend actually returned. An operator told "3/20, máy còn lại chưa root" goes
 * and checks Magisk; an operator told "máy 4, 11, 19 — DeviceBusy: đang nuôi" goes and stops the
 * campaign.
 *
 * Modelled on `groupInputOutcome`, which already does this for `group_input` — same grouping,
 * same last-six-of-the-udid naming. The difference is that this works on
 * `Promise.allSettled` results, which is what every group tool uses and what
 * `groupInputOutcome` cannot read.
 *
 * Returns `null` when nothing failed, so a caller can use `??` to keep its own wording for the
 * all-clear case.
 */
export function fanOutReasons(
  udids: readonly string[],
  results: readonly PromiseSettledResult<unknown>[],
): string | null {
  const byReason = new Map<string, string[]>();
  results.forEach((result, index) => {
    if (result.status !== "rejected") return;
    // `describeError`, not `String`: a Tauri command rejects with `{ code, message }`, and
    // `String` on that is `[object Object]`. Three sites in `RootTool` were printing exactly
    // that into the log panel an operator reads after a factory reset.
    const reason = describeError(result.reason);
    const list = byReason.get(reason) ?? [];
    // The last six characters are what the tiles show, so this is the name the operator can
    // match against something on screen.
    list.push((udids[index] ?? "?").slice(-6));
    byReason.set(reason, list);
  });
  if (byReason.size === 0) return null;
  return [...byReason.entries()]
    .map(([reason, named]) => `${named.join(", ")} — ${reason}`)
    .join("\n");
}

/** How many of a fan-out actually succeeded. */
export function fanOutReached(results: readonly PromiseSettledResult<unknown>[]): number {
  return results.filter((result) => result.status === "fulfilled").length;
}
