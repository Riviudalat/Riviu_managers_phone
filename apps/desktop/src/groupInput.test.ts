import { describe, expect, it } from "vitest";

import { groupInputOutcome } from "./groupInput";

describe("group input outcome", () => {
  it("says nothing when every phone took it", () => {
    expect(groupInputOutcome({ completedUdids: ["a", "b"], skipped: [] })).toEqual({
      kind: "ok",
    });
  });

  it("calls a batch that reached nobody a failure, however the promise resolved", () => {
    // The defect this whole helper exists for: twenty phones, twenty refusals, and the call
    // still resolved -- so the operator was told it worked.
    const outcome = groupInputOutcome({
      completedUdids: [],
      skipped: [
        { udid: "ce0517151215a00304", code: "DeviceBusy", currentOwner: "nurture" },
        { udid: "ce051715ac247a3f01", code: "DeviceBusy", currentOwner: "nurture" },
      ],
    });
    expect(outcome.kind).toBe("none");
    expect(outcome).toMatchObject({ title: expect.stringContaining("Không máy nào") });
  });

  it("names the phones and who is holding them, not just a count", () => {
    // "3 máy bị bỏ qua" is not actionable. Knowing nurture has them is.
    const outcome = groupInputOutcome({
      completedUdids: ["ok-1"],
      skipped: [{ udid: "ce0517151215a00304", code: "DeviceBusy", currentOwner: "nurture" }],
    });
    expect(outcome.kind).toBe("partial");
    if (outcome.kind === "ok") throw new Error("expected a partial outcome");
    expect(outcome.detail).toContain("a00304");
    expect(outcome.detail).toContain("nurture");
  });

  it("carries the reason for a phone that failed rather than one that was busy", () => {
    const outcome = groupInputOutcome({
      completedUdids: ["ok-1"],
      skipped: [{ udid: "ce0abc", code: "ActionFailed", message: "agent did not answer" }],
    });
    if (outcome.kind === "ok") throw new Error("expected a partial outcome");
    expect(outcome.detail).toContain("agent did not answer");
  });

  it("groups phones that share a reason onto one line", () => {
    // Twenty phones held by nurture must not print twenty lines.
    const outcome = groupInputOutcome({
      completedUdids: [],
      skipped: ["aaa111", "bbb222", "ccc333"].map((udid) => ({
        udid,
        code: "DeviceBusy",
        currentOwner: "nurture",
      })),
    });
    if (outcome.kind === "ok") throw new Error("expected a none outcome");
    expect(outcome.detail.split("\n")).toHaveLength(1);
    expect(outcome.detail).toContain("aaa111, bbb222, ccc333");
  });
});
