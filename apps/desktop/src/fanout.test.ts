import { describe, expect, it } from "vitest";
import { fanOutReached, fanOutReasons } from "./fanout";

/** What `Promise.allSettled` hands back, without needing real promises to build it. */
function settled(outcomes: (true | unknown)[]): PromiseSettledResult<unknown>[] {
  return outcomes.map((outcome) =>
    outcome === true
      ? ({ status: "fulfilled", value: undefined } as PromiseFulfilledResult<unknown>)
      : ({ status: "rejected", reason: outcome } as PromiseRejectedResult),
  );
}

describe("reporting a fan-out across phones", () => {
  /**
   * **A rejection is an object, and `String` on it prints `[object Object]`.**
   *
   * Three sites in `RootTool` were doing exactly that, into the log panel an operator reads
   * after a factory reset across twenty phones. The gate meant to catch this class could not
   * see them: it looks for `catch` bindings, and `Promise.allSettled` never produces one.
   */
  it("describes a rejected command by its message", () => {
    const reasons = fanOutReasons(
      ["10969614", "23021RAAEG"],
      settled([true, { code: "OperationFailed", message: "Permission denied" }]),
    );
    expect(reasons).toContain("Permission denied");
    expect(reasons).not.toContain("[object Object]");
  });

  /**
   * **Names the phones, not just the count.**
   *
   * "3 máy lỗi" is not actionable. The last six characters are what the tiles show, so they are
   * the name an operator can match against something on screen — the same choice
   * `groupInputOutcome` already made for `group_input`.
   */
  it("names which phones failed, by the digits the tiles show", () => {
    const reasons = fanOutReasons(
      ["aaaa10969614", "bbbb23021RAA", "cccc99999999"],
      settled([{ message: "chưa root" }, true, { message: "chưa root" }]),
    );
    expect(reasons).toContain("969614");
    expect(reasons).toContain("999999");
    expect(reasons).not.toContain("23021RAA");
  });

  /** Two phones failing the same way are one line, not two. */
  it("groups phones that failed for the same reason", () => {
    const reasons = fanOutReasons(
      ["phone-a", "phone-b", "phone-c"],
      settled([
        { message: "chưa root" },
        { message: "chưa root" },
        { message: "máy đang bận" },
      ]),
    );
    expect(reasons?.split("\n")).toHaveLength(2);
    expect(reasons).toMatch(/hone-a, hone-b — chưa root/);
  });

  /**
   * `null` when nothing failed, so a caller can keep its own all-clear wording with `??`.
   *
   * Not an empty string: an empty detail line renders as a toast with a blank second row, which
   * reads as something having gone wrong that nobody would name.
   */
  it("says nothing at all when every phone answered", () => {
    expect(fanOutReasons(["a", "b"], settled([true, true]))).toBeNull();
  });

  /** Every phone failing is still reported, rather than folded into the all-clear case. */
  it("reports a total failure as a total failure", () => {
    const reasons = fanOutReasons(["a", "b"], settled([{ message: "hỏng" }, { message: "hỏng" }]));
    expect(reasons).toContain("hỏng");
    expect(fanOutReached(settled([{ message: "hỏng" }, { message: "hỏng" }]))).toBe(0);
  });

  /**
   * A results array longer than the udid list must not throw or invent a name.
   *
   * Defensive rather than reachable: the two come from the same `map` at every call site today.
   * But this runs in the failure path, and a reporting helper that throws while reporting takes
   * the report with it.
   */
  it("does not throw when it has more results than names", () => {
    const reasons = fanOutReasons([], settled([{ message: "hỏng" }]));
    expect(reasons).toContain("hỏng");
    expect(reasons).toContain("?");
  });

  it("counts what actually succeeded", () => {
    expect(fanOutReached(settled([true, {}, true, true]))).toBe(3);
    expect(fanOutReached(settled([]))).toBe(0);
  });
});
