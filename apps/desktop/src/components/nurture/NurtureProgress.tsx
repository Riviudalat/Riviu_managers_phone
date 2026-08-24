import {
  currentRun,
  deviceProgress,
  deviceProgressLabel,
  type RunProgress,
} from "../../nurtureProgress";
import type { NurtureSessionStatus } from "../../types";

/**
 * The first progress bar in this app was here, and it earned itself on one property: a
 * nurture run is bounded by a wall clock as well as a video count, so "42 of 120" cannot tell
 * an operator whether a phone is nearly done. Everything else here follows the house idiom —
 * the numbers stay on screen beside the bar rather than being replaced by it.
 *
 * The bar itself now lives in `components/ProgressBar.tsx` because the interaction panel grew
 * one too.
 */
import { ProgressBar, type BarProps } from "../ProgressBar";

/** The tone a device's bar should take from its terminal state. */
function toneOf(status: NurtureSessionStatus): BarProps["tone"] {
  if (status.phase !== "finished") return status.phase === "queued" ? "idle" : "run";
  if (status.outcome === "failed") return "failed";
  return "done";
}

/** What a device is doing, in words, when that is more informative than a number. */
const PHASE_WORDS: Record<string, string> = {
  queued: "trong hàng chờ",
  opening: "đang mở phiên điều khiển",
  awaitingFeed: "đang đưa về feed",
  recovering: "đang cứu phiên",
};

/**
 * One device's bar, with the label naming whichever bound is governing.
 *
 * The phase word matters more than the percentage for the first minute of a run: a healthy
 * phone can spend forty seconds waiting for TikTok to reach the foreground and another
 * thirty waiting for the feed, and a bar at 0% for both of those is indistinguishable from a
 * phone that never opened the app — which is exactly how two lock-screen phones went
 * unnoticed on 23/08/2026.
 */
export function NurtureDeviceProgress({
  status,
  now,
}: {
  status: NurtureSessionStatus;
  now: number;
}) {
  const fraction = deviceProgress(status, now);
  const phaseWord = PHASE_WORDS[status.phase];
  return (
    <div className="nu-device-progress">
      <ProgressBar
        fraction={fraction}
        tone={toneOf(status)}
        label={`Tiến trình ${status.udid}`}
      />
      <div className="nu-device-progress-meta">
        {phaseWord && <span className="nu-phase">{phaseWord}</span>}
        <span className="nu-bound">{deviceProgressLabel(status, now)}</span>
      </div>
    </div>
  );
}

/**
 * The run-wide bar: one line the operator can read without opening anything.
 *
 * The failure count is not decoration. A run that finishes with two dead phones is 100%
 * *settled*, and a bar alone would read that as success — so the red tail and the `N lỗi`
 * chip are what make a full bar honest.
 */
export function NurtureRunProgress({
  statuses,
  now,
}: {
  statuses: NurtureSessionStatus[];
  now: number;
}) {
  const run: RunProgress | null = currentRun(statuses, now);
  if (!run) return null;
  const settled = run.done + run.failed;
  return (
    <div className="nu-run">
      <div className="nu-run-head">
        <span className="nu-run-title">Tiến trình lượt chạy</span>
        <div className="grow" />
        <span className="nu-run-count">
          {settled}/{run.size} máy · {Math.round(run.fraction * 100)}%
        </span>
      </div>
      <ProgressBar
        fraction={run.fraction}
        failedFraction={run.failedFraction}
        tone={run.failed > 0 && run.running === 0 ? "failed" : "run"}
        label="Tiến trình cả lượt chạy"
        className="nu-bar-lg"
      />
      <div className="nu-run-chips">
        <span className="nu-chip is-run" title="đang chạy">
          ● {run.running} đang chạy
        </span>
        <span className="nu-chip is-done" title="đã xong">
          ✓ {run.done} xong
        </span>
        {/* Rendered only when there are failures: a permanent "0 lỗi" trains the eye to skip
            the spot where the number that matters will appear. */}
        {run.failed > 0 && (
          <span className="nu-chip is-failed" title="phiên lỗi — bấm dòng máy để xem lý do">
            ✕ {run.failed} lỗi
          </span>
        )}
      </div>
    </div>
  );
}
