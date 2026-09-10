import { describe, expect, it } from "vitest";
import { allocateScheduleRows, decodeScheduleDraft, scheduleTime, type ScheduleRow } from "./publishScheduleAllocation";
const rows = (n: number): ScheduleRow[] => Array.from({ length: n }, (_, i) => ({ id: `r${i}`, bundleId: `b${i}`, udid: "", time: "", timeMode: "common" }));
describe("schedule allocation", () => {
  it("distributes ten posts to ten machines in supplied source/number order", () => {
    const input = rows(10), machines = Array.from({ length: 10 }, (_, i) => `m${i}`);
    const result = allocateScheduleRows(input, input.map(r => r.bundleId), machines, "20:00");
    expect(result.rows.map(r => r.udid)).toEqual(machines);
    expect(result.missing).toEqual([]);
    expect(input.every(r => !r.udid)).toBe(true);
  });
  it("uses the result of consecutive allocations and preserves occupied slots", () => {
    let input = rows(4); input[0].udid = "manual";
    input = allocateScheduleRows(input, ["b1"], ["m1", "m2"], "20:00").rows;
    const result = allocateScheduleRows(input, ["b1", "b2", "b3"], ["m1", "m2"], "20:00");
    expect(result.rows.map(r => r.udid)).toEqual(["manual", "m1", "m2", ""]);
    expect(result.missing).toEqual(["b3"]);
  });
  it("rejects an occupied direct target but supports different times on one machine", () => {
    const input = rows(3); input[0].udid = "m1";
    expect(allocateScheduleRows(input, ["b1"], ["m1", "m2"], "20:00", "m1").missing).toEqual(["b1"]);
    input[1].timeMode = "custom"; input[1].time = "20:05";
    expect(allocateScheduleRows(input, ["b1"], ["m1"], "20:00", "m1").rows[1].udid).toBe("m1");
    expect(allocateScheduleRows(input, ["b2"], [], "20:00", "gone").missing).toEqual(["b2"]);
  });
  it("moves just the origin post to a free direct target and preserves its source on conflict", () => {
    const input = rows(3); input[0].udid = "m1"; input[1].udid = "m2";
    const moved = allocateScheduleRows(input, ["b0"], ["m3"], "20:00", "m3");
    expect(moved.rows.map(r => r.udid)).toEqual(["m3", "m2", ""]);
    const rejected = allocateScheduleRows(input, ["b0"], ["m2"], "20:00", "m2");
    expect(rejected.rows).toEqual(input); expect(rejected.missing).toEqual(["b0"]);
    expect(allocateScheduleRows(input, ["b0"], ["m1"], "20:00", "m1").assigned).toEqual([]);
  });
  it("migrates old times and request identity without borrowing new selections", () => {
    const old = { sourceRoot: "source", date: "2099-09-10", requestId: "original", rows: [{ id: "a", bundleId: "b", udid: "m", time: "12:05" }] };
    const migrated = decodeScheduleDraft(JSON.stringify(old), "source")!;
    expect(migrated.requestId).toBe("original");
    expect(migrated.commonTime).toBe("");
    expect(scheduleTime(migrated.rows[0], "22:00")).toBe("12:05");
    expect(migrated.selectedMachines).toEqual(["m"]);
    expect(decodeScheduleDraft(JSON.stringify(old), "other")).toBeNull();
    expect(decodeScheduleDraft(JSON.stringify({ ...old, rows: [...old.rows, ...old.rows] }), "source")).toBeNull();
  });
});
