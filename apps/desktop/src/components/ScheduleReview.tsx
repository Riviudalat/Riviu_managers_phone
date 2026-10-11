import {
  formatScheduleTime,
  localTimeZoneLabel,
  runsIfEnabledAt,
  upcomingRuns,
  type ScheduleScope,
} from "../scheduleScope";
import { StatusNotice } from "./States";
import { deviceNames, usePreviewClock } from "../useScheduleScope";
import "../styles/schedule-review.css";

/** A titled list of run times with the zone they are in; compact folds it into a disclosure. */
export function RunTimesPreview({
  title,
  times,
  overdueFirst = false,
  compact = false,
  emptyText = "Chưa có lần chạy kế tiếp.",
}: {
  title: string;
  times: Date[];
  overdueFirst?: boolean;
  compact?: boolean;
  emptyText?: string;
}) {
  const body = times.length ? (
    <ol className="schedule-runs-list">
      {times.map((time, index) => (
        <li key={time.toISOString()}>
          <time dateTime={time.toISOString()}>{formatScheduleTime(time)}</time>
          {overdueFirst && index === 0 && <span> · quá hạn, chạy ở lần quét kế tiếp khi Riviu đang mở</span>}
        </li>
      ))}
    </ol>
  ) : (
    <p className="hint">{emptyText}</p>
  );
  const zone = <p className="schedule-runs-zone">{localTimeZoneLabel()}</p>;
  if (compact) {
    return (
      <details className="schedule-runs" aria-label={title}>
        <summary>{title}</summary>
        {zone}
        {body}
      </details>
    );
  }
  return (
    <section className="schedule-runs" aria-label={title}>
      <strong>{title}</strong>
      {zone}
      {body}
    </section>
  );
}

/** The next five runs, with the zone they are in. A schedule that is off shows what switching it on now would do. */
export function NextRunsPreview({
  nextDueAt,
  everyMinutes,
  enabled,
  compact = false,
}: {
  nextDueAt: string | null;
  everyMinutes: number;
  enabled: boolean;
  compact?: boolean;
}) {
  const now = usePreviewClock();
  const valid = Number.isInteger(everyMinutes) && everyMinutes > 0;
  const { overdue, times } = enabled
    ? upcomingRuns(nextDueAt, everyMinutes, now)
    : { overdue: false, times: valid ? runsIfEnabledAt(now, everyMinutes) : [] };
  return (
    <RunTimesPreview
      title={enabled ? "5 lần chạy tới" : "Nếu bật lúc này, 5 lần chạy đầu"}
      times={times}
      overdueFirst={overdue}
      compact={compact}
      emptyText={valid ? "Chưa có lần chạy kế tiếp." : "Nhập chu kỳ hợp lệ để xem lịch chạy."}
    />
  );
}

/** The approved device list and how the live target departs from it. */
export function ScheduleScopeNotice({
  scope,
  approved,
  label,
  busy = false,
  onApprove,
  error,
  onRetry,
}: {
  scope: ScheduleScope;
  approved: string[] | null;
  label: (udid: string) => string;
  busy?: boolean;
  onApprove?: (udids: string[]) => void;
  /** A failed membership read: the scope stays unknown, never approved. */
  error?: string | null;
  onRetry?: () => void;
}) {
  if (scope.kind === "unknown" && error) {
    return (
      <StatusNotice tone="error" action={onRetry && <button type="button" onClick={onRetry}>Đọc lại nhóm máy</button>}>
        Chưa đọc được nhóm máy nên chưa biết phạm vi: {error}. Lịch không bật được cho tới khi đọc lại.
      </StatusNotice>
    );
  }
  if (scope.kind === "unknown") return <p className="schedule-scope-line">Đang đọc phạm vi máy…</p>;
  if (scope.kind === "missingGroup") {
    return (
      <StatusNotice tone="error">
        Nhóm máy của lịch không còn. Lần chạy sẽ bị từ chối trước khi gửi lệnh; tạo lịch mới với phạm vi khác.
      </StatusNotice>
    );
  }
  const approveButton = onApprove && scope.designated.length > 0 && (
    <button type="button" disabled={busy} onClick={() => onApprove(scope.designated)}>
      {scope.kind === "unapproved" ? `Duyệt ${scope.designated.length} máy` : `Duyệt lại ${scope.designated.length} máy`}
    </button>
  );
  if (scope.kind === "unapproved") {
    return (
      <StatusNotice tone="warning" action={approveButton}>
        Chưa duyệt danh sách máy. Lịch không chạy cho tới khi duyệt.
        {scope.designated.length > 0
          ? ` Phạm vi hiện có ${scope.designated.length} máy: ${deviceNames(scope.designated, label)}.`
          : " Phạm vi hiện không có máy nào."}
      </StatusNotice>
    );
  }
  if (scope.kind === "changed") {
    return (
      <StatusNotice tone="warning" action={approveButton}>
        Phạm vi khác danh sách đã duyệt
        {scope.added.length > 0 && ` · thêm ${deviceNames(scope.added, label)}`}
        {scope.removed.length > 0 && ` · bớt ${deviceNames(scope.removed, label)}`}
        . Lịch không chạy và không tự đổi phạm vi cho tới khi duyệt lại.
      </StatusNotice>
    );
  }
  const list = approved ?? scope.designated;
  return (
    <details className="schedule-scope-line">
      <summary>Đã duyệt {list.length} máy</summary>
      <p>{deviceNames(list, label)}</p>
    </details>
  );
}
