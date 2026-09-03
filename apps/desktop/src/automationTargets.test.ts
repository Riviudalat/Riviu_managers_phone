import { describe, expect, it } from "vitest";

import { resolveAutomationTarget } from "./automationTargets";
import type { DeviceGroup, DeviceInfo } from "./types";

const devices = [
  { udid: "a" },
  { udid: "b" },
] as DeviceInfo[];
const groups = [
  { id: "morning", udids: ["a", "departed"] },
] as DeviceGroup[];

describe("resolveAutomationTarget", () => {
  it("resolves all and explicit targets in current roster order", () => {
    expect(resolveAutomationTarget({ type: "all" }, devices, groups)).toEqual(["a", "b"]);
    expect(
      resolveAutomationTarget({ type: "explicit", udids: ["b", "b", "departed"] }, devices, groups),
    ).toEqual(["b"]);
  });

  it("resolves a group against the current roster and never treats an empty group as all", () => {
    expect(resolveAutomationTarget({ type: "group", groupId: "morning" }, devices, groups))
      .toEqual(["a"]);
    expect(resolveAutomationTarget({ type: "group", groupId: "missing" }, devices, groups))
      .toEqual([]);
    expect(
      resolveAutomationTarget(
        { type: "group", groupId: "morning" },
        [{ udid: "b" } as DeviceInfo],
        groups,
      ),
    ).toEqual([]);
  });
});
