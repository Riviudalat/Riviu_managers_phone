import type { OperationRunSummary } from "../../types";

/**
 * Lượt chạy → the Cần xử lý drawer: show one run's phones and logs in "Đang chạy". Finished runs
 * live in Lượt chạy; this is how the operator reopens one in the run monitor. Only an operator's
 * press sends it, so the drawer still never opens on its own.
 */
export const QUEUE_WATCH_RUN = "riviu:queue-watch-run";

export function watchRunInQueue(run: OperationRunSummary): void {
  window.dispatchEvent(new CustomEvent<OperationRunSummary>(QUEUE_WATCH_RUN, { detail: run }));
}
