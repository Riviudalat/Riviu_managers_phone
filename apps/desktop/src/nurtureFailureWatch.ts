import { pushToast } from "./toastStore";
import type { NurtureSessionStatus } from "./types";

/**
 * Telling the operator that phones failed, even when the Nuôi TT panel is closed.
 *
 * **The hole this closes.** The `nurtureStatus` event had exactly one listener and it lived
 * inside `NurturePopup`, so a session that failed while the panel was shut was never seen by
 * anything. On 23/08/2026 two of fourteen phones failed on their lock screens and the fleet
 * summary still read "14 sẵn sàng", because that chip counts phones that stream — and a
 * locked phone streams its lock screen perfectly.
 *
 * **One toast, not fourteen.** `toastStore` keeps four visible; a burst of per-phone toasts
 * would evict everything including itself. So failures are collected over a short window and
 * reported once, grouped by reason — the same shape `groupInput.ts` uses for a group action
 * that partly failed.
 */

/** How long to gather failures before saying anything. */
const BATCH_MS = 2_500;

/**
 * The part of a failure message that identifies *what went wrong*, without the specifics.
 *
 * Everything up to the first colon. The engine's nine failure producers all write
 * `failed — <reason>` and then, optionally, `: <detail>` carrying a udid, a size or a nested
 * driver error — which is exactly the part that differs between two phones that failed the
 * same way. Grouping on the whole string would report "2 phones, 2 reasons" for one problem.
 */
export function failureReason(lastMessage: string): string {
  const withoutPrefix = lastMessage.replace(/^\s*failed\s*[—-]\s*/i, "");
  const head = withoutPrefix.split(":")[0].trim();
  return head || "không rõ lý do";
}

/** A phone's short name for a toast — the tail of its udid, which is what is on the shelf. */
function shortUdid(udid: string): string {
  return udid.length > 6 ? udid.slice(-6) : udid;
}

export interface FailureReport {
  title: string;
  detail: string;
}

/**
 * One line for a batch of failures, grouped by reason.
 *
 * Returns `null` for an empty batch so the caller has nothing to decide.
 */
export function summariseFailures(rows: NurtureSessionStatus[]): FailureReport | null {
  if (!rows.length) return null;
  const byReason = new Map<string, string[]>();
  for (const row of rows) {
    const reason = failureReason(row.lastMessage);
    byReason.set(reason, [...(byReason.get(reason) ?? []), shortUdid(row.udid)]);
  }
  const parts = [...byReason.entries()]
    // Biggest group first: on a fleet run the common cause is the one worth reading.
    .sort((a, b) => b[1].length - a[1].length)
    .map(([reason, udids]) => `${reason} (${udids.length}): ${udids.join(", ")}`);
  return {
    title:
      rows.length === 1
        ? `1 máy nuôi TT bị lỗi`
        : `${rows.length} máy nuôi TT bị lỗi`,
    detail: parts.join(" · "),
  };
}

/**
 * Collects terminal failures and reports them in batches.
 *
 * Deliberately not a React hook: it is subscribed once from `useFleet`, which is mounted for
 * the life of the app, and a hook would tie the only listener for this to a component's
 * lifecycle — which is the shape that produced the hole in the first place.
 */
export class NurtureFailureWatch {
  private pending: NurtureSessionStatus[] = [];
  private timer: ReturnType<typeof setTimeout> | null = null;
  /** Phones already reported, so a repeated terminal push does not toast twice. */
  private reported = new Set<string>();

  private readonly announce: (report: FailureReport) => void;

  // A plain field rather than a parameter property: this project builds with
  // `erasableSyntaxOnly`, which rejects the shorthand because it emits code.
  constructor(announce: (report: FailureReport) => void = defaultAnnounce) {
    this.announce = announce;
  }

  /** Feed every nurture status here. Non-terminal and non-failed rows are ignored. */
  observe(status: NurtureSessionStatus): void {
    if (status.phase !== "finished" || status.outcome !== "failed") {
      // A phone that is running again has a fresh session, so it may be reported again.
      this.reported.delete(status.udid);
      return;
    }
    // The engine pushes a terminal status more than once on some paths — the summary and
    // then the tagged final row — and both carry the same verdict.
    if (this.reported.has(status.udid)) return;
    this.reported.add(status.udid);
    this.pending.push(status);
    if (this.timer) return;
    this.timer = setTimeout(() => this.flush(), BATCH_MS);
  }

  /** Report whatever has gathered. Exposed so a test does not have to wait out the window. */
  flush(): void {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    const batch = this.pending;
    this.pending = [];
    const report = summariseFailures(batch);
    if (report) this.announce(report);
  }
}

function defaultAnnounce(report: FailureReport): void {
  pushToast("error", report.title, report.detail);
}
