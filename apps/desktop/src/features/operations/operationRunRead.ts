import { operationGetRun } from "../../api";

/** One run's items, refused when the source answers for another run. */
export async function readOperationRun(runId: string) {
  const next = await operationGetRun(runId);
  if (!next || next.summary.id !== runId) throw new Error("Tác vụ không còn trong nguồn dữ liệu.");
  return next;
}

/** The query key every reader of one run's items shares (the run monitor and the Cần xử lý queue). */
export const operationRunScope = (runId: string) => ["monitor", "run", runId] as const;
