import { beforeEach, describe, expect, it } from "vitest";
import type { OperationRunSummary } from "../../types";
import { dismissMonitorRecords, MONITOR_RECORDS_KEY, readDismissedRecords, visibleMonitorRuns, writeDismissedRecords } from "./monitorRecords";
import { clampMonitorPosition } from "./useFloatingMonitor";

const run = { id: "nurture:one", state: "succeeded", updatedAt: "2026-09-07T00:00:00Z", totalItems: 20, completedItems: 20, issueCount: 0 } as OperationRunSummary;
beforeEach(() => localStorage.clear());
describe("monitor records", () => {
  it("removes only settled snapshots and persists across reload", () => {
    const active = { ...run, id: "nurture:two", state: "running" } as OperationRunSummary;
    const records = dismissMonitorRecords([], [run, active]);
    writeDismissedRecords(records);
    expect(visibleMonitorRuns([run, active], readDismissedRecords())).toEqual([active]);
    expect(records).toHaveLength(1);
  });
  it("does not hide a resumed or newly updated result", () => {
    const records = dismissMonitorRecords([], [run]);
    expect(visibleMonitorRuns([{ ...run, state: "running" }], records)).toHaveLength(1);
    expect(visibleMonitorRuns([{ ...run, updatedAt: "2026-09-07T00:01:00Z" }], records)).toHaveLength(1);
    expect(visibleMonitorRuns([{ ...run, state: "uncertain" }], records)).toHaveLength(1);
  });
  it("rejects corrupt storage and prunes expired records", () => {
    localStorage.setItem(MONITOR_RECORDS_KEY, '{"bad":true}');
    expect(() => readDismissedRecords()).toThrow();
    writeDismissedRecords([{ key: "old", at: 1 }]);
    expect(readDismissedRecords()).toEqual([]);
  });
  it("keeps the full floating window inside a resized viewport", () => {
    expect(clampMonitorPosition({ left: 1200, top: 850 }, 760, 500, 900, 560)).toEqual({ left: 132, top: 52 });
    expect(clampMonitorPosition({ left: -20, top: -20 }, 760, 500, 900, 560)).toEqual({ left: 8, top: 8 });
  });
});
