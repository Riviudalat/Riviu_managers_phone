import { describe, expect, it } from "vitest";
import type { NurtureSessionStatus, OperationRunItem, OperationRunState, OperationRunSummary } from "../../types";
import { compactLogEntries, deviceRows, deviceStateCounts, logMessage, logTime, monitorDeviceName, runOptionLabel, runProgress } from "./operationProgress";

const run = { sourceId: "one", kind: "nurture", state: "running", totalItems: 2, completedItems: 0, targetCount: 2 } as OperationRunSummary;
describe("operation progress", () => {
  it("counts only the selected run, reserves the unreported device and waits for cleanup", () => {
    const row = { udid: "a", runId: "one", running: true, videosDone: 5, videoTarget: 10, phase: "watching", startedAt: null, deadlineAt: null } as NurtureSessionStatus;
    expect(runProgress(run, [row, { ...row, runId: "older", videosDone: 10 }])).toBe(.25);
    expect(runProgress(run, [{ ...row, videosDone: 10 }, { ...row, udid: "b", videosDone: 10 }])).toBe(.99);
  });
  it("keeps unknown work indeterminate, and terminal failure is processed not successful", () => {
    expect(runProgress({ ...run, kind: "flow", totalItems: 0 }, [])).toBeNull();
    expect(runProgress({ ...run, state: "failed" }, [])).toBe(1);
    expect(runProgress({ ...run, kind: "publish", completedItems: 20, totalItems: 2 }, [])).toBe(.99);
  });
  it("does not double count a Flow parent plus its child attempts", () => {
    const item = { id: "device", udid: "a", kind: "device", state: "running" } as OperationRunItem;
    expect(deviceRows([item, { ...item, id: "attempt", kind: "attempt", state: "succeeded" }])[0].fraction).toBe(0);
    expect(deviceRows([{ ...item, state: "queued" }])[0].state).toBe("queued");
  });
  it.each([
    ["succeeded", "cancelled"],
    ["cancelled", "succeeded"],
    ["succeeded", "skipped"],
    ["skipped", "succeeded"],
    ["succeeded", "failed"],
    ["failed", "succeeded"],
    ["cancelled", "skipped"],
  ] satisfies [OperationRunState, OperationRunState][])("reports mixed terminal %s/%s as partial regardless of item order", (first, second) => {
    const items = [first, second].map((state, index) => ({ id: `${index}`, udid: "a", kind: "assignment", state }) as OperationRunItem);
    expect(deviceRows(items)[0]).toMatchObject({ state: "partial", fraction: 1 });
  });
  it.each(["succeeded", "failed", "partial", "cancelled", "skipped"] satisfies OperationRunState[])("prioritizes uncertain above terminal %s in either order", (state) => {
    const item = { id: "first", udid: "a", kind: "assignment", state } as OperationRunItem;
    const uncertain = { ...item, id: "second", state: "uncertain" } as OperationRunItem;
    expect(deviceRows([item, uncertain])[0].state).toBe("uncertain");
    expect(deviceRows([uncertain, item])[0].state).toBe("uncertain");
  });
  it.each(["succeeded", "failed", "partial", "cancelled", "skipped", "uncertain"] satisfies OperationRunState[])("preserves homogeneous terminal %s", (state) => {
    const item = { id: "first", udid: "a", kind: "assignment", state } as OperationRunItem;
    expect(deviceRows([item, { ...item, id: "second" }])[0]).toMatchObject({ state, fraction: 1 });
  });
  it("keeps active work ahead of terminal outcomes until the device settles", () => {
    const items = ["uncertain", "queued", "running"].map((state, index) => ({ id: `${index}`, udid: "a", kind: "assignment", state }) as OperationRunItem);
    expect(deviceRows(items)[0]).toMatchObject({ state: "running", fraction: 1 / 3 });
    expect(deviceRows(items.slice(0, 2))[0]).toMatchObject({ state: "queued", fraction: 1 / 2 });
  });
  it("accounts for every machine state without counting task-wide aggregate rows", () => {
    const states: OperationRunState[] = ["queued", "running", "succeeded", "partial", "failed", "uncertain", "cancelled", "skipped"];
    const rows = states.map((state, index) => ({ udid: `device-${index}`, state }));
    const counts = deviceStateCounts([...rows, { udid: "", state: "failed" }]);
    expect(counts).toEqual({ total: 8, succeeded: 1, issues: 3, active: 2, stopped: 2 });
    expect(counts.succeeded + counts.issues + counts.active + counts.stopped).toBe(counts.total);
    expect(deviceStateCounts([])).toEqual({ total: 0, succeeded: 0, issues: 0, active: 0, stopped: 0 });
  });
  it("counts a machine once when its assignment results are mixed", () => {
    const item = { id: "first", udid: "a", kind: "assignment", state: "succeeded" } as OperationRunItem;
    const rows = deviceRows([item, { ...item, id: "second", state: "cancelled" }, { ...item, id: "aggregate", udid: null }]);
    expect(deviceStateCounts(rows)).toEqual({ total: 1, succeeded: 0, issues: 1, active: 0, stopped: 0 });
  });
  it("uses seconds from the source and never invents a timestamp or success", () => {
    expect(logTime("2026-09-07T12:34:56")).toBe("12:34:56");
    expect(logTime(null)).toBe("--:--:--");
    expect(logTime("broken")).toBe("--:--:--");
    expect(logMessage({ id: "x", at: null, action: "save", state: "uncertain", text: null, detail: null })).toBe("Lưu bài · Chưa xác nhận");
  });
  it("summarizes technical nurture messages without changing their raw evidence", () => {
    const row = { id: "x", at: "2026-09-07T12:34:56", action: "nurture", state: "opening", text: "queued", detail: null };
    expect(logMessage(row)).toBe("Đang chờ bắt đầu");
    const technical = { ...row, text: "nhãn đã đo: com.ss.android.ugc.trill / en (SM-G955F)" };
    expect(logMessage(technical)).toBe("Đã nhận diện cấu hình TikTok");
    expect(technical.text).toContain("com.ss.android.ugc.trill");
    expect(logMessage({ ...row, text: "failed — 0/1 video, 1 tim (hierarchy), đã tắt sạch TikTok" })).toBe("Thất bại: 0/1 video, 1 tim, đã tắt sạch TikTok");
  });
  it("groups only identical consecutive records, preserving first and last time", () => {
    const row = { id: "a", at: "2026-09-07T12:34:56", action: "nurture", state: "opening", text: "mở phiên điều khiển mới", detail: null };
    const rows = compactLogEntries([row, { ...row, id: "b", at: "2026-09-07T12:34:59" }, { ...row, id: "c", detail: "new error" }, { ...row, id: "d" }]);
    expect(rows).toHaveLength(3);
    expect(rows[0]).toEqual({ entry: row, count: 2, lastAt: "2026-09-07T12:34:59" });
  });
  it("makes machine number primary but never strips a user alias", () => {
    expect(monitorDeviceName("Máy 3 · SM G955F")).toEqual({ name: "Máy 3", model: "SM G955F" });
    expect(monitorDeviceName("Máy 3 · Tài khoản nội dung")).toEqual({ name: "Máy 3 · Tài khoản nội dung", model: null });
    expect(runOptionLabel({ ...run, title: "Nuôi TikTok", createdAt: "2026-09-07T12:34:56", updatedAt: null })).toContain("12:34");
  });
});
