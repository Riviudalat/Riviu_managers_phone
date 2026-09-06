import type { OperationRunSummary } from "../../types";
import { activeRun } from "./operationProgress";

export const MONITOR_RECORDS_KEY = "riviu.monitor.dismissed.v1";
const RETENTION_MS = 7 * 24 * 60 * 60_000;
export interface DismissedRecord { key: string; at: number }

/** A new outcome or restarted run must reappear even if an older snapshot was dismissed. */
export function monitorRecordKey(run: OperationRunSummary): string {
  return JSON.stringify([run.id, run.state, run.updatedAt, run.totalItems, run.completedItems, run.issueCount]);
}

export function readDismissedRecords(now = Date.now()): DismissedRecord[] {
  const raw = localStorage.getItem(MONITOR_RECORDS_KEY);
  if (!raw) return [];
  const parsed: unknown = JSON.parse(raw);
  if (!Array.isArray(parsed) || parsed.some((row) => !row || typeof row.key !== "string" || !Number.isFinite(row.at))) {
    throw new Error("Danh sách bản ghi đã xoá không hợp lệ.");
  }
  return parsed.filter((row: DismissedRecord) => row.at <= now && now - row.at < RETENTION_MS);
}

export function writeDismissedRecords(records: DismissedRecord[]) {
  localStorage.setItem(MONITOR_RECORDS_KEY, JSON.stringify(records));
}

export function dismissMonitorRecords(records: DismissedRecord[], runs: OperationRunSummary[], now = Date.now()): DismissedRecord[] {
  const next = new Map(records.filter((row) => now - row.at < RETENTION_MS).map((row) => [row.key, row]));
  for (const run of runs) {
    if (!activeRun(run)) next.set(monitorRecordKey(run), { key: monitorRecordKey(run), at: now });
  }
  return [...next.values()];
}

export function visibleMonitorRuns(runs: OperationRunSummary[], records: DismissedRecord[]): OperationRunSummary[] {
  const dismissed = new Set(records.map((row) => row.key));
  return runs.filter((run) => activeRun(run) || !dismissed.has(monitorRecordKey(run)));
}
