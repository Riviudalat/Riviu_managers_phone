export type ScheduleRow = { id: string; bundleId: string; udid: string; time: string; timeMode: "common" | "custom" };
export type ScheduleDraft = { version: 2; sourceRoot: string; date: string; commonTime: string; rows: ScheduleRow[]; selectedMachines: string[]; requestId: string };
export const SCHEDULE_DRAFT_KEY = "riviu.publish.daily-schedule.v1";
export const scheduleTime = (row: ScheduleRow, commonTime: string) => row.timeMode === "custom" ? row.time : commonTime;

/** The old draft has only per-row times. Preserve them, including its request identity. */
export function decodeScheduleDraft(raw: string | null, sourceRoot: string): ScheduleDraft | null {
  try {
    const d = JSON.parse(raw ?? "null");
    if (!d || d.sourceRoot !== sourceRoot || typeof d.date !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(d.date)
      || typeof d.requestId !== "string" || !Array.isArray(d.rows) || d.rows.length > 100
      || !d.rows.every((r: ScheduleRow) => r && [r.id, r.bundleId, r.udid, r.time].every(v => typeof v === "string"))
      || new Set(d.rows.map((r: ScheduleRow) => r.id)).size !== d.rows.length
      || new Set(d.rows.map((r: ScheduleRow) => r.bundleId)).size !== d.rows.length) return null;
    return { version: 2, sourceRoot, date: d.date, requestId: d.requestId,
      commonTime: d.version === 2 && typeof d.commonTime === "string" ? d.commonTime : "",
      selectedMachines: [...new Set<string>((Array.isArray(d.selectedMachines) ? d.selectedMachines.filter((v: unknown) => typeof v === "string") : d.rows.map((r: ScheduleRow) => r.udid)).filter(Boolean))],
      rows: d.rows.map((r: ScheduleRow) => ({ ...r, timeMode: d.version === 2 && r.timeMode === "common" ? "common" : "custom" })) };
  } catch { return null; }
}

export function machineHasScheduleConflict(rows: ScheduleRow[], bundleId: string, udid: string, commonTime: string) {
  const row = rows.find(r => r.bundleId === bundleId);
  return !row || rows.some(r => r.bundleId !== bundleId && r.udid === udid && scheduleTime(r, commonTime) === scheduleTime(row, commonTime));
}

/** Fill only free slots, in source/device order. Unlike fillAssignments, partial capacity is useful here. */
export function allocateScheduleRows(rows: ScheduleRow[], bundleIds: string[], machineIds: string[], commonTime: string, directMachine?: string) {
  const next = rows.map(r => ({ ...r }));
  const assigned: { bundleId: string; udid: string }[] = [];
  const missing: string[] = [];
  for (const bundleId of [...new Set(bundleIds)]) {
    const row = next.find(r => r.bundleId === bundleId);
    if (!row || (row.udid && !directMachine)) continue;
    const candidates = directMachine ? machineIds.filter(id => id === directMachine) : machineIds;
    const udid = candidates.find(id => !machineHasScheduleConflict(next, bundleId, id, commonTime));
    if (!udid) { missing.push(bundleId); continue; }
    if (row.udid === udid) continue;
    row.udid = udid;
    assigned.push({ bundleId, udid });
  }
  return { rows: next, assigned, missing };
}
