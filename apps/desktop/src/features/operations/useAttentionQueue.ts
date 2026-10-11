import { useCallback, useMemo } from "react";
import { useQueries } from "@tanstack/react-query";
import { operationQueryRuns } from "../../api";
import { describeError } from "../../describeError";
import { readQueryClient } from "../../readQuery";
import type { OperationRunSummary } from "../../types";
import {
  deviceAttentionItems, publishAttentionItems, runAttentionItems, sortAttentionItems,
  type AttentionGroup, type DeviceAttention, type QueuePhone,
} from "./attentionQueueModel";
import { monitorRecordKey, type DismissedRecord } from "./monitorRecords";
import { activeRun, issueState, runOptionLabel } from "./operationProgress";
import { operationRunScope, readOperationRun } from "./operationRunRead";
import { publishRunScope, readPublishRun } from "./publishRunRead";
import { useMonitorRead } from "./useMonitorRead";

const DAY_MS = 24 * 60 * 60_000;
/** Chưa rõ and Sheet errors outlive the 24 h run window; older ones are in Lượt chạy. */
export const UNRESOLVED_LOOKBACK_DAYS = 30;
/** Finite budgets: at most this many campaigns and runs are read for the queue. */
const MAX_PUBLISH_READS = 30;
const MAX_RUN_READS = 20;

const newestFirst = (left: OperationRunSummary, right: OperationRunSummary) =>
  (Date.parse(right.updatedAt ?? "") || 0) - (Date.parse(left.updatedAt ?? "") || 0) || left.id.localeCompare(right.id);

export interface AttentionQueue {
  groups: AttentionGroup[];
  total: number;
  /** Some source is unread or failed, so `total` is a floor, never the whole count. */
  unknown: boolean;
  /** Every source answered once; new items after this raise the notice. */
  settled: boolean;
  /** Campaigns or runs beyond the read budget. */
  omitted: number;
}

/**
 * The Cần xử lý queue from the reads other surfaces already make: the 24 h run list, each
 * campaign's full assignments (the shared outcome key) and each failed run's items (the run
 * monitor's key). Polls only; nothing here sends a command.
 */
export function useAttentionQueue({ runs, runsSettled, runsError, phones, attention, attentionKnown, labels, dismissed }: {
  runs: readonly OperationRunSummary[];
  runsSettled: boolean;
  runsError: string | null;
  phones: ReadonlyMap<string, QueuePhone>;
  attention: readonly DeviceAttention[];
  attentionKnown: boolean;
  labels: ReadonlyMap<string, string>;
  dismissed: readonly DismissedRecord[];
}): AttentionQueue {
  const readOlder = useCallback(() => operationQueryRuns({
    kind: "publish", limit: 200, offset: 0,
    since: new Date(Date.now() - UNRESOLVED_LOOKBACK_DAYS * DAY_MS).toISOString(),
  }), []);
  const older = useMonitorRead(readOlder, 30_000, ["runs", "publish", `last${UNRESOLVED_LOOKBACK_DAYS}d`]);
  const { publishRuns, issueRuns, omitted } = useMemo(() => {
    const byId = new Map<string, OperationRunSummary>();
    // Every publish run of the day: the shared model can leave a backend-finished post Chưa rõ.
    for (const run of runs) if (run.kind === "publish") byId.set(run.id, run);
    for (const run of older.value?.runs ?? []) {
      if (run.kind === "publish" && (run.state === "uncertain" || run.state === "partial") && !byId.has(run.id)) byId.set(run.id, run);
    }
    const allPublish = [...byId.values()].sort(newestFirst);
    const allIssues = runs.filter(run => run.kind !== "publish" && (issueState(run.state) || run.issueCount > 0)).sort(newestFirst);
    return {
      publishRuns: allPublish.slice(0, MAX_PUBLISH_READS),
      issueRuns: allIssues.slice(0, MAX_RUN_READS),
      omitted: Math.max(0, allPublish.length - MAX_PUBLISH_READS) + Math.max(0, allIssues.length - MAX_RUN_READS),
    };
  }, [runs, older.value]);
  // Stable query options, so TanStack does not re-apply fresh option objects after every render.
  const publishPlan = JSON.stringify(publishRuns.map(run => [run.sourceId, activeRun(run)]));
  const publishQueries = useMemo(() => (JSON.parse(publishPlan) as [string, boolean][]).map(([campaignId, active]) => ({
    queryKey: publishRunScope(campaignId),
    queryFn: () => readPublishRun(campaignId),
    // A finished campaign changes only through background checks and Sheet retries.
    refetchInterval: active ? 2000 : 10_000,
    refetchIntervalInBackground: true,
  })), [publishPlan]);
  const runPlan = JSON.stringify(issueRuns.map(run => run.id));
  const runQueries = useMemo(() => (JSON.parse(runPlan) as string[]).map(runId => ({
    queryKey: operationRunScope(runId),
    queryFn: () => readOperationRun(runId),
    refetchInterval: 5000,
    refetchIntervalInBackground: true,
  })), [runPlan]);
  const publishReads = useQueries({ queries: publishQueries }, readQueryClient);
  const runReads = useQueries({ queries: runQueries }, readQueryClient);

  return useMemo(() => {
    const hidden = new Set(dismissed.map(record => record.key));
    const groups: AttentionGroup[] = [];
    // A source that has not answered yet makes the total a floor, exactly like one that failed.
    const olderAnswered = older.value !== null || older.error !== null;
    let unknown = !attentionKnown || !runsSettled || runsError !== null || !olderAnswered || older.error !== null;
    let settled = runsSettled && olderAnswered;
    if (attention.length) {
      groups.push({ key: "devices", title: "Máy cần xử lý", run: null, items: sortAttentionItems(deviceAttentionItems(attention, phones, labels)) });
    }
    const runGroups: AttentionGroup[] = [];
    const collect = (run: OperationRunSummary, read: { data?: unknown; error: unknown; errorUpdateCount: number }, items: () => AttentionGroup["items"]) => {
      // A read with no data that failed before stays an error while it retries: TanStack resets a
      // data-less query to pending on refetch, and letting the group vanish and come back would
      // remount its readers, whose fetch-on-mount resets it again, without end.
      if (read.error || (read.data === undefined && read.errorUpdateCount > 0)) {
        unknown = true;
        runGroups.push({ key: run.id, title: runOptionLabel(run), run, items: [], error: read.error ? describeError(read.error) : "chưa đọc được; đang đọc lại" });
        return;
      }
      if (read.data === undefined) { unknown = true; settled = false; return; }
      // A dismissed snapshot hides only the failures; Chưa rõ and Sheet errors stay.
      const shown = items().filter(item => item.persistent || !hidden.has(monitorRecordKey(run)));
      if (shown.length) runGroups.push({ key: run.id, title: runOptionLabel(run), run, items: sortAttentionItems(shown) });
    };
    publishRuns.forEach((run, index) => {
      const read = publishReads[index];
      collect(run, read, () => read.data ? publishAttentionItems(read.data, run, phones, labels) : []);
    });
    issueRuns.forEach((run, index) => {
      const read = runReads[index];
      collect(run, read, () => read.data ? runAttentionItems(read.data, run, phones, labels) : []);
    });
    runGroups.sort((left, right) => left.run && right.run ? newestFirst(left.run, right.run) : 0);
    groups.push(...runGroups);
    return {
      groups,
      total: groups.reduce((sum, group) => sum + group.items.length, 0),
      unknown,
      settled,
      omitted,
    };
  }, [attention, attentionKnown, phones, labels, dismissed, runsError, runsSettled, older.value, older.error, publishRuns, publishReads, issueRuns, runReads, omitted]);
}
