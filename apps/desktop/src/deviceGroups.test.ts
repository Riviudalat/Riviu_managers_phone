import { describe, expect, it } from "vitest";
import { ALL_DEVICES_TAB, devicesInTab, groupTabs, withDeviceAdded } from "./deviceGroups";
import type { DeviceGroup, DeviceInfo } from "./types";

function device(udid: string): DeviceInfo {
  return { udid, name: udid, model: "m", platform: "android" } as unknown as DeviceInfo;
}
function group(id: string, name: string, udids: string[]): DeviceGroup {
  return { id, name, color: "#f97316", udids, createdAt: "2026-08-14T00:00:00Z" };
}

const DEVICES = [device("a"), device("b"), device("c")];
const GROUPS = [group("g1", "Đà Lạt", ["a", "b", "gone"]), group("g2", "Trống", ["gone"])];

describe("groupTabs", () => {
  it("counts devices that are actually present, not udids the group remembers", () => {
    // A group holds phones that may be unplugged. Labelling the tab with the stored
    // count promises rows the grid cannot produce.
    const tabs = groupTabs(DEVICES, GROUPS);

    expect(tabs.map((tab) => [tab.label, tab.count])).toEqual([
      ["Tất cả", 3],
      ["Đà Lạt", 2],
      ["Trống", 0],
    ]);
  });

  it("keeps a group whose phones are all elsewhere", () => {
    // Hiding it would make the group look deleted; zero is information.
    const tabs = groupTabs(DEVICES, GROUPS);

    expect(tabs.some((tab) => tab.label === "Trống")).toBe(true);
  });

  it("puts all-devices first and gives it no colour", () => {
    const [first] = groupTabs(DEVICES, GROUPS);

    expect(first.id).toBe(ALL_DEVICES_TAB);
    expect(first.color).toBeNull();
  });
});

describe("devicesInTab", () => {
  it("filters to the group's members, keeping fleet order", () => {
    expect(devicesInTab(DEVICES, GROUPS, "g1").map((d) => d.udid)).toEqual(["a", "b"]);
  });

  it("shows everything for the all-devices tab", () => {
    expect(devicesInTab(DEVICES, GROUPS, ALL_DEVICES_TAB)).toHaveLength(3);
  });

  it("falls back to every device when the group no longer exists", () => {
    // An empty grid because a group was deleted elsewhere is indistinguishable from a
    // fleet that vanished, and the operator cannot tell which happened.
    expect(devicesInTab(DEVICES, GROUPS, "deleted-in-another-window")).toHaveLength(3);
  });
});

describe("withDeviceAdded", () => {
  it("adds the device once and returns a new group", () => {
    const next = withDeviceAdded(GROUPS, "g1", "c");

    expect(next?.udids).toEqual(["a", "b", "gone", "c"]);
    expect(GROUPS[0].udids).not.toContain("c");
  });

  it("returns null when there is nothing to do", () => {
    // Saving anyway would make an idempotent click look like a change.
    expect(withDeviceAdded(GROUPS, "g1", "a")).toBeNull();
    expect(withDeviceAdded(GROUPS, "missing", "a")).toBeNull();
  });
});
