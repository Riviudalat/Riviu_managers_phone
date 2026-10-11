import { useEffect, useState } from "react";
import {
  formatScheduleTime,
  localTimeZoneLabel,
  runsIfEnabledAt,
  upcomingRuns,
  type ScheduleScope,
} from "../scheduleScope";
import { StatusNotice } from "./States";
import { deviceNames } from "../useScheduleScope";
import "../styles/schedule-review.css";

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
  const [now, setNow] = useState(() => Date.now());
  // "If switched on now" moves with the clock; a page left open must not show stale slots.
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, []);
  const valid = Number.isInteger(everyMinutes) && everyMinutes > 0;
  const { overdue, times } = enabled
    ? upcomingRuns(nextDueAt, everyMinutes, now)
    : { overdue: false, times: valid ? runsIfEnabledAt(now, everyMinutes) : [] };
  const title = enabled ? "5 lần chạy tới" : "Nếu bật lúc này, 5 lần chạy đầu";
  const body = times.length ? (
    <ol className="schedule-runs-list">
      {times.map((time, index) => (
        <li key={time.toISOString()}>
          <time dateTime={time.toISOString()}>{formatScheduleTime(time)}</time>
          {overdue && index === 0 && <span> · quá hạn, chạy ở lần quét kế tiếp khi Riviu đang mở</span>}
        </li>
      ))}
    </ol>
  ) : (
    <p className="hint">{valid ? "Chưa có lần chạy kế tiếp." : "Nhập chu kỳ hợp lệ để xem lịch chạy."}</p>
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

/** The approved device list and how the live target departs from it. */
export function ScheduleScopeNotice({
  scope,
  approved,
  label,
  busy = false,
  onApprove,
}: {
  scope: ScheduleScope;
  approved: string[] | null;
  label: (udid: string) => string;
  busy?: boolean;
  onApprove?: (udids: string[]) => void;
}) {
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
