import { useCallback, useEffect, useMemo, useState } from "react";
import { useQueries } from "@tanstack/react-query";
import { operationQueryRuns } from "../../api";
import { describeError } from "../../describeError";
import { readQueryClient } from "../../readQuery";
import type { OperationRunDetail, OperationRunKind, OperationRunSummary } from "../../types";
import {
  deviceAttentionItems, publishAttentionItems, runAttentionItems, sortAttentionItems,
  type AttentionGroup, type AttentionItem, type DeviceAttention, type QueuePhone,
} from "./attentionQueueModel";
import { monitorRecordKey, type DismissedRecord } from "./monitorRecords";
import { activeRun, issueState, runOptionLabel } from "./operationProgress";
import { operationRunScope, readOperationRun } from "./operationRunRead";
import { publishRunScope, readPublishRun, type PublishRunRead } from "./publishRunRead";
import { useMonitorRead } from "./useMonitorRead";

/** Finite budgets per poll; whatever they leave unread makes the count a floor ("≥N"). */
const MAX_RECENT_PUBLISH_READS = 30;
const MAX_RECENT_RUN_READS = 20;
const MAX_CLASSIFY_READS = 20;
/** One page of each all-age discovery read; a further page is reported, never assumed empty. */
const DISCOVERY_PAGE = 200;
export const UNRESOLVED_REGISTRY_KEY = "riviu.queue.unresolved.v1";

const newestFirst = (left: OperationRunSummary, right: OperationRunSummary) =>
  (Date.parse(right.updatedAt ?? "") || 0) - (Date.parse(left.updatedAt ?? "") || 0) || left.id.localeCompare(right.id);

function isSummary(value: unknown): value is OperationRunSummary {
  const run = value as Partial<OperationRunSummary> | null;
  return !!run && typeof run.id === "string" && typeof run.sourceId === "string" && typeof run.kind === "string"
    && typeof run.title === "string" && typeof run.state === "string";
}

/** Runs known to hold Chưa rõ or Sheet-error rows, kept across restarts until a read clears them. */
function readRegistry(): OperationRunSummary[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(UNRESOLVED_REGISTRY_KEY) ?? "[]");
    return Array.isArray(parsed) ? parsed.filter(isSummary) : [];
  } catch { return []; }
}

function writeRegistry(runs: readonly OperationRunSummary[]) {
  try { localStorage.setItem(UNRESOLVED_REGISTRY_KEY, JSON.stringify(runs)); } catch { /* kept in memory */ }
}

export interface AttentionQueue {
  groups: AttentionGroup[];
  total: number;
  /** Some source is unread, failed or truncated, so `total` is a floor, never the whole count. */
  unknown: boolean;
  /** Every source answered once; new items after this raise the notice. */
  settled: boolean;
  /** Runs and campaigns left unread by a budget. */
  omitted: number;
}

type Read = { data?: unknown; error: unknown; errorUpdateCount: number };
type Planned = Pick<OperationRunSummary, "id" | "sourceId" | "kind"> & { active: boolean };

function itemsOf(run: OperationRunSummary, data: unknown, phones: ReadonlyMap<string, QueuePhone>, labels: ReadonlyMap<string, string>): AttentionItem[] {
  return run.kind === "publish"
    ? publishAttentionItems(data as PublishRunRead, run, phones, labels)
    : runAttentionItems(data as OperationRunDetail, run, phones, labels);
}

const pollQuery = (run: Planned) => run.kind === "publish"
  ? {
    queryKey: publishRunScope(run.sourceId),
    queryFn: () => readPublishRun(run.sourceId),
    // A finished campaign changes only through background checks and Sheet retries.
    refetchInterval: run.active ? 2000 : 10_000,
    refetchIntervalInBackground: true,
  }
  : {
    queryKey: operationRunScope(run.id),
    queryFn: () => readOperationRun(run.id),
    refetchInterval: 5000,
    refetchIntervalInBackground: true,
  };

/** One read per run version: a finished run holding nothing unresolved is not polled again. */
const classifyKey = (run: Pick<OperationRunSummary, "id" | "updatedAt">) => ["queueClassify", run.id, run.updatedAt ?? ""] as const;
const classifyQuery = ([id, sourceId, kind, updatedAt]: [string, string, OperationRunKind, string]) => ({
  queryKey: classifyKey({ id, updatedAt }),
  queryFn: () => kind === "publish" ? readPublishRun(sourceId) : readOperationRun(id),
  staleTime: Infinity,
  gcTime: Infinity,
});

/**
 * The Cần xử lý queue from the reads other surfaces already make: the 24 h run list, each
 * campaign's full assignments (the shared outcome key) and each failed run's items (the run
 * monitor's key). Chưa rõ and Sheet errors are not bound to that window: runs of any age are
 * discovered (every publish run, every uncertain or partial run) and a run seen holding one is
 * kept, across restarts, until a successful read shows none left; no age, budget or newer run
 * removes it. A failed read keeps the rows of its last success and says so. Every budget and
 * unread page turns the count into a floor. Polls only; nothing here sends a command.
 */
export function useAttentionQueue({ runs, runsSettled, runsError, runsHasMore = false, phones, attention, attentionKnown, labels, dismissed }: {
  runs: readonly OperationRunSummary[];
  runsSettled: boolean;
  runsError: string | null;
  /** The 24 h list itself stopped at its page size. */
  runsHasMore?: boolean;
  phones: ReadonlyMap<string, QueuePhone>;
  attention: readonly DeviceAttention[];
  attentionKnown: boolean;
  labels: ReadonlyMap<string, string>;
  dismissed: readonly DismissedRecord[];
}): AttentionQueue {
  const readPublishAll = useCallback(() => operationQueryRuns({ kind: "publish", limit: DISCOVERY_PAGE, offset: 0 }), []);
  const readUncertainAll = useCallback(() => operationQueryRuns({ state: "uncertain", limit: DISCOVERY_PAGE, offset: 0 }), []);
  const readPartialAll = useCallback(() => operationQueryRuns({ state: "partial", limit: DISCOVERY_PAGE, offset: 0 }), []);
  const publishAll = useMonitorRead(readPublishAll, 60_000, ["runs", "publish", "all"]);
  const uncertainAll = useMonitorRead(readUncertainAll, 60_000, ["runs", "uncertain", "all"]);
  const partialAll = useMonitorRead(readPartialAll, 60_000, ["runs", "partial", "all"]);
  const [registry, setRegistry] = useState<OperationRunSummary[]>(readRegistry);
  // Bumped when a one-shot read lands, so the next runs move into the read window.
  const [classifyTick, setClassifyTick] = useState(0);

  const recentIds = useMemo(() => new Set(runs.map(run => run.id)), [runs]);
  const plan = useMemo(() => {
    const latest = new Map<string, OperationRunSummary>();
    const offer = (run: OperationRunSummary) => {
      const known = latest.get(run.id);
      if (!known || newestFirst(run, known) < 0) latest.set(run.id, run);
    };
    // The registry first and uncapped: what is known unresolved is always read.
    const polled = new Set<string>();
    for (const run of registry) { offer(run); polled.add(run.id); }
    // The day's runs within a budget: every publish run (the shared model can leave a
    // backend-finished post Chưa rõ) and every non-publish run with issues.
    const recentPublish = runs.filter(run => run.kind === "publish").sort(newestFirst);
    const recentIssues = runs.filter(run => run.kind !== "publish" && (issueState(run.state) || run.issueCount > 0)).sort(newestFirst);
    for (const run of [...recentPublish.slice(0, MAX_RECENT_PUBLISH_READS), ...recentIssues.slice(0, MAX_RECENT_RUN_READS)]) {
      offer(run); polled.add(run.id);
    }
    let omitted = [...recentPublish.slice(MAX_RECENT_PUBLISH_READS), ...recentIssues.slice(MAX_RECENT_RUN_READS)]
      .filter(run => !polled.has(run.id)).length;
    // Runs of any age that may hold unresolved rows, each read once per version.
    const candidates = new Map<string, OperationRunSummary>();
    for (const run of publishAll.value?.runs ?? []) if (run.kind === "publish") candidates.set(run.id, run);
    for (const run of [...(uncertainAll.value?.runs ?? []), ...(partialAll.value?.runs ?? [])]) candidates.set(run.id, run);
    const classify: OperationRunSummary[] = [];
    const classified: OperationRunSummary[] = [];
    for (const run of [...candidates.values()].sort(newestFirst)) {
      offer(run);
      if (polled.has(run.id)) continue;
      if (readQueryClient.getQueryData(classifyKey(run)) !== undefined) classified.push(run);
      else if (classify.length < MAX_CLASSIFY_READS) classify.push(run);
      else omitted += 1;
    }
    const polledRuns = [...polled].map(id => latest.get(id)).filter((run): run is OperationRunSummary => !!run).sort(newestFirst);
    return { polled: polledRuns, classify, classified, omitted, tick: classifyTick };
  }, [registry, runs, publishAll.value, uncertainAll.value, partialAll.value, classifyTick]);

  // Stable query options, so TanStack does not re-apply fresh option objects after every render.
  const pollPlan = JSON.stringify(plan.polled.map((run): Planned => ({ id: run.id, sourceId: run.sourceId, kind: run.kind, active: activeRun(run) })));
  const pollQueries = useMemo(() => (JSON.parse(pollPlan) as Planned[]).map(pollQuery), [pollPlan]);
  const classifyPlan = JSON.stringify(plan.classify.map(run => [run.id, run.sourceId, run.kind, run.updatedAt ?? ""]));
  const classifyQueries = useMemo(() => (JSON.parse(classifyPlan) as [string, string, OperationRunKind, string][]).map(classifyQuery), [classifyPlan]);
  const pollReads = useQueries({ queries: pollQueries }, readQueryClient);
  const classifyReads = useQueries({ queries: classifyQueries }, readQueryClient);
  const classifyLanded = classifyReads.some(read => read.data !== undefined);
  useEffect(() => { if (classifyLanded) setClassifyTick(tick => tick + 1); }, [classifyLanded, classifyPlan]);

  const queue = useMemo(() => {
    const hidden = new Set(dismissed.map(record => record.key));
    const groups: AttentionGroup[] = [];
    const unresolved = new Map<string, OperationRunSummary>();
    const cleared = new Set<string>();
    let unknown = !attentionKnown || !runsSettled || runsError !== null || runsHasMore || plan.omitted > 0;
    let settled = runsSettled;
    for (const source of [publishAll, uncertainAll, partialAll]) {
      if (source.value === null && source.error === null) { unknown = true; settled = false; }
      if (source.error !== null || source.value?.hasMore) unknown = true;
    }
    if (attention.length) {
      groups.push({ key: "devices", title: "Máy cần xử lý", run: null, items: sortAttentionItems(deviceAttentionItems(attention, phones, labels)) });
    }
    const runGroups: AttentionGroup[] = [];
    const collect = (run: OperationRunSummary, read: Read | undefined, polled: boolean) => {
      const data = read?.data;
      // A failed read keeps the rows of its last success beside the error. A read that never
      // succeeded stays an error while it retries: TanStack resets a data-less query to pending
      // on refetch, and letting the group vanish and come back would remount its readers.
      const error = read?.error ? describeError(read.error)
        : data === undefined && (read?.errorUpdateCount ?? 0) > 0 ? "chưa đọc được; đang đọc lại" : undefined;
      if (error) unknown = true;
      if (data === undefined) {
        if (error) runGroups.push({ key: run.id, title: runOptionLabel(run), run, items: [], error });
        else { unknown = true; settled = false; }
        return;
      }
      const all = itemsOf(run, data, phones, labels);
      if (all.some(item => item.persistent)) unresolved.set(run.id, run);
      else if (polled && !error) cleared.add(run.id);
      // Outside the day's window only what stays until a conclusion is shown; a dismissed
      // snapshot hides only the failures.
      const shown = all.filter(item => item.persistent || (recentIds.has(run.id) && !hidden.has(monitorRecordKey(run))));
      if (shown.length || error) runGroups.push({ key: run.id, title: runOptionLabel(run), run, items: sortAttentionItems(shown), error, stale: !!error });
    };
    plan.polled.forEach((run, index) => collect(run, pollReads[index], true));
    plan.classify.forEach((run, index) => collect(run, classifyReads[index], false));
    for (const run of plan.classified) collect(run, { data: readQueryClient.getQueryData(classifyKey(run)), error: null, errorUpdateCount: 0 }, false);
    runGroups.sort((left, right) => left.run && right.run ? newestFirst(left.run, right.run) : 0);
    groups.push(...runGroups);
    return {
      groups,
      total: groups.reduce((sum, group) => sum + group.items.length, 0),
      unknown,
      settled,
      omitted: plan.omitted,
      unresolved,
      cleared,
    };
  }, [attention, attentionKnown, phones, labels, dismissed, runsError, runsSettled, runsHasMore, recentIds, plan, pollReads, classifyReads,
    publishAll, uncertainAll, partialAll]);

  // Keep the registry in step: add runs seen holding unresolved rows; retire a run only when a
  // successful read of it shows none left.
  useEffect(() => {
    const added = [...queue.unresolved.values()].filter(run => !registry.some(known => known.id === run.id));
    const removed = registry.filter(run => queue.cleared.has(run.id)).map(run => run.id);
    if (!added.length && !removed.length) return;
    const next = [...registry.filter(run => !removed.includes(run.id)), ...added];
    writeRegistry(next);
    setRegistry(next);
  }, [queue, registry]);

  return { groups: queue.groups, total: queue.total, unknown: queue.unknown, settled: queue.settled, omitted: queue.omitted };
}
