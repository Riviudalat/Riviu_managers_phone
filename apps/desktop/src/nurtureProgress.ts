import type { NurtureSessionStatus } from "./types";

/**
 * How far along a nurture run is — per device and over a whole run.
 *
 * **Why this arithmetic is here and not on the wire.** A nurture session ends at whichever
 * of two bounds arrives first: the video count, or a wall clock that for a manual start is a
 * randomised 2–3 hour horizon. The clock half means the bar has to advance *between* status
 * pushes — a phone watching a long video emits nothing for twenty seconds, and a bar that
 * only moved on events would freeze while the run genuinely progressed. So the fraction is
 * recomputed locally on a timer, which rules out shipping a percentage from Rust.
 *
 * The cost of that is a policy implemented twice, so the rules are pinned twice: every test
 * in `nurtureProgress.test.ts` has a counterpart in `crates/core/src/types.rs`'s
 * `progress_tests`, and `NurtureSessionStatus::progress_fraction` is the reference. If you
 * change a rule here, change it there.
 */

/** Which bound is closer to ending a session. Mirrors Rust's `NurtureBound`. */
export type NurtureBound = "videos" | "clock";

const TERMINAL_PHASE = "finished";

/** Milliseconds from an ISO timestamp, or `null` when it is absent or unparseable. */
function msOf(iso: string | null | undefined): number | null {
  if (!iso) return null;
  const at = new Date(iso).getTime();
  return Number.isNaN(at) ? null : at;
}

/**
 * The two fractions, each `null` when its bound is not known yet.
 *
 * `null` rather than `0` throughout: a queued device with no target is *unknown*, and a bar
 * that draws unknown as an empty track looks like a stall.
 */
function bounds(
  status: NurtureSessionStatus,
  now: number,
): { byVideos: number | null; byClock: number | null } {
  const byVideos =
    status.videoTarget > 0 ? status.videosDone / status.videoTarget : null;
  const started = msOf(status.startedAt);
  const deadline = msOf(status.deadlineAt);
  let byClock: number | null = null;
  if (started !== null && deadline !== null) {
    const total = deadline - started;
    // A non-positive window is nonsense, not zero progress: it would divide by zero or
    // invert the fraction.
    if (total > 0) byClock = Math.max(0, now - started) / total;
  }
  return { byVideos, byClock };
}

/**
 * How far along one device is, in `0..1`, or `null` when there is nothing to divide by.
 *
 * The **maximum** of the two bounds, because the session ends at whichever arrives first. A
 * count-only reading sits at 40% on a run twelve minutes from finishing; a clock-only
 * reading sits at 3% on a run about to hit its video cap.
 *
 * A terminal phase reads 1 whatever the counters say — a session that stopped at 40 of 120
 * is finished, and leaving its bar short reads as still working. The *colour* carries the
 * verdict, not the length.
 */
export function deviceProgress(
  status: NurtureSessionStatus,
  now: number = Date.now(),
): number | null {
  if (status.phase === TERMINAL_PHASE) return 1;
  const { byVideos, byClock } = bounds(status, now);
  const candidates = [byVideos, byClock].filter((v): v is number => v !== null);
  if (!candidates.length) return null;
  return Math.min(1, Math.max(0, Math.max(...candidates)));
}

/**
 * How far ahead the clock must be before the label switches to it.
 *
 * Without this the clock wins the moment a run starts — `videosDone` is 0, so any elapsed
 * second beats it — and the first thing an operator saw was "còn ~154 phút" instead of the
 * "0/5 video" they had just typed. Measured on the live fleet on 23/08/2026, which is where
 * that reading came from. Five points of lead is enough to mean the clock is genuinely the
 * bound that will end the run.
 *
 * The bar's *fill* still takes the plain maximum. This is only about which sentence to print:
 * a fill that ignored a leading clock would under-report, while a label that names it too
 * early is just noise.
 */
const CLOCK_LABEL_LEAD = 0.05;

/** Which bound is governing, for the label beside the bar. `null` for a finished row. */
export function governingBound(
  status: NurtureSessionStatus,
  now: number = Date.now(),
): NurtureBound | null {
  if (status.phase === TERMINAL_PHASE) return null;
  const { byVideos, byClock } = bounds(status, now);
  if (byVideos !== null && byClock !== null) {
    return byClock > byVideos + CLOCK_LABEL_LEAD ? "clock" : "videos";
  }
  if (byVideos !== null) return "videos";
  if (byClock !== null) return "clock";
  return null;
}

/** Whole minutes left on the clock bound, or `null` when there is no deadline. */
export function minutesLeft(
  status: NurtureSessionStatus,
  now: number = Date.now(),
): number | null {
  const deadline = msOf(status.deadlineAt);
  if (deadline === null) return null;
  return Math.max(0, Math.round((deadline - now) / 60_000));
}

/** What one device's bar should say beside itself. */
export function deviceProgressLabel(
  status: NurtureSessionStatus,
  now: number = Date.now(),
): string {
  if (status.phase === TERMINAL_PHASE) {
    const words: Record<string, string> = {
      done: "xong",
      partial: "xong một phần",
      failed: "lỗi",
      stopped: "đã dừng",
    };
    const verdict = status.outcome ? words[status.outcome] : null;
    // The counters still matter on a terminal row: "lỗi · 0/120 video" says something very
    // different from "lỗi · 96/120 video".
    const counted = status.videoTarget > 0
      ? `${status.videosDone}/${status.videoTarget} video`
      : `${status.videosDone} video`;
    return verdict ? `${verdict} · ${counted}` : counted;
  }
  if (governingBound(status, now) === "clock") {
    const left = minutesLeft(status, now);
    // Named rather than inferred: the clock is ahead, so the video count is not what will
    // end this run and printing it would be the wrong sentence.
    return left === null ? "theo thời gian" : `còn ~${left} phút`;
  }
  if (status.videoTarget > 0) return `${status.videosDone}/${status.videoTarget} video`;
  return `${status.videosDone} video`;
}

/** How one run is going, over the devices that belong to it. */
export interface RunProgress {
  runId: string;
  /** Rows in this run, in the order given. */
  rows: NurtureSessionStatus[];
  /** Devices the run was started with — the denominator. */
  size: number;
  /** `0..1` over `size`, so a device that never reported still occupies its slot. */
  fraction: number;
  running: number;
  done: number;
  failed: number;
  /** Share of the whole run that ended badly, `0..1` — drawn as a red tail on the bar. */
  failedFraction: number;
}

/**
 * Roll up the rows belonging to the newest run.
 *
 * **Filtered by run id, and that filter is the whole point.** The status list is keyed by
 * udid and never pruned, so it accumulates every phone that has run since the app started:
 * summing over all of it counts finished phones from earlier runs, and restarting one phone
 * makes the total go *backwards* because that row's counters reset to zero while the others
 * keep their finished values.
 *
 * Returns `null` when no row carries a run id — which is every row from before this existed,
 * and every row the idle popup sweep wrote.
 */
export function currentRun(
  statuses: NurtureSessionStatus[],
  now: number = Date.now(),
): RunProgress | null {
  const withRun = statuses.filter((s) => s.runId);
  if (!withRun.length) return null;

  // "Newest" is the run with a live session if there is one, else the run holding the most
  // recently started device. A run id is a uuid4 and carries no time, so it cannot be sorted.
  const byRun = new Map<string, NurtureSessionStatus[]>();
  for (const status of withRun) {
    const key = status.runId as string;
    byRun.set(key, [...(byRun.get(key) ?? []), status]);
  }
  const score = (rows: NurtureSessionStatus[]) => {
    const live = rows.some((r) => r.running) ? 1 : 0;
    const latest = Math.max(
      0,
      ...rows.map((r) => msOf(r.startedAt) ?? 0),
    );
    return { live, latest };
  };
  let best: { runId: string; rows: NurtureSessionStatus[] } | null = null;
  let bestScore = { live: -1, latest: -1 };
  for (const [runId, rows] of byRun) {
    const s = score(rows);
    if (s.live > bestScore.live || (s.live === bestScore.live && s.latest > bestScore.latest)) {
      best = { runId, rows };
      bestScore = s;
    }
  }
  if (!best) return null;

  const { runId, rows } = best;
  // The run's own denominator, not the number of rows present. A phone that failed before
  // producing a second status still occupies a slot, and one that never produced a row at
  // all must not shrink the total — otherwise a 14-phone run with 2 that never started
  // reads 100% when 12 finish.
  const size = Math.max(rows[0]?.runSize ?? 0, rows.length);
  let sum = 0;
  let running = 0;
  let done = 0;
  let failed = 0;
  for (const row of rows) {
    // An unknown fraction counts as zero *progress* while still occupying its slot — the
    // honest reading for a device that has not started.
    sum += deviceProgress(row, now) ?? 0;
    if (row.phase !== TERMINAL_PHASE) running += 1;
    else if (row.outcome === "failed") failed += 1;
    else done += 1;
  }
  return {
    runId,
    rows,
    size,
    fraction: size > 0 ? Math.min(1, sum / size) : 0,
    running,
    done,
    failed,
    // Drawn as a red tail rather than folded into `fraction`, so a run that finishes with
    // two dead phones reads "full, and two of them failed" instead of a clean 100%.
    failedFraction: size > 0 ? Math.min(1, failed / size) : 0,
  };
}
