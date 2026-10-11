import type { NurtureSettings, NurtureWindow } from "../../types";
import { nurtureScheduleRuns } from "../../scheduleScope";
import { usePreviewClock } from "../../useScheduleScope";
import { RunTimesPreview } from "../ScheduleReview";
import { StatusNotice } from "../States";

export type NurtureScheduleSave = "keep" | "enable" | "disable";

function clock(minute: number): string {
  return `${String(Math.floor(minute / 60)).padStart(2, "0")}:${String(minute % 60).padStart(2, "0")}`;
}

function windowName(window: NurtureWindow): string {
  return `Khung ${clock(window.startMinute)}–${clock(window.endMinute)}`;
}

/**
 * Saving the Nuôi schedule from Hẹn giờ — the one place its phone lists are approved.
 *
 * Lưu writes the phones selected in Thiết lập as explicit lists (the all-day cadence's, and
 * every window still on "tất cả"); the backend runs only those lists and refuses an empty one,
 * so a phone plugged in later never joins. Shows what is stored but refused, exactly which
 * phones Lưu approves, and the next five runs with the zone they are in.
 */
export function NurtureScheduleActions({
  settings,
  targets,
  label,
  busy,
  blocked,
  describedBy,
  onSave,
}: {
  settings: NurtureSettings;
  /** The phones selected in Thiết lập: what Lưu approves. */
  targets: string[];
  label: (udid: string) => string;
  busy: boolean;
  /** A settings issue that must be fixed before the schedule is saved. */
  blocked: boolean;
  describedBy?: string;
  onSave: (mode: NurtureScheduleSave) => void;
}) {
  const now = usePreviewClock();
  const windows = settings.scheduleWindows ?? [];
  const enabled = settings.scheduleEnabled;
  const names = (udids: string[]) => udids.map(label).join(", ");
  const unapproved = windows.length
    ? windows.filter((window) => !window.udids.length).map(windowName)
    : settings.scheduleUdids.length ? [] : ["Lịch cả ngày"];
  const approvals = windows.length
    ? windows.map((window) => ({ name: windowName(window), udids: window.udids.length ? window.udids : targets }))
    : [{ name: "Lịch cả ngày", udids: targets.length ? targets : settings.scheduleUdids }];
  const times = nurtureScheduleRuns(settings, now);
  return (
    <section className="nurture-schedule-review" aria-label="Lưu hẹn giờ Nuôi">
      <p className="schedule-scope-line">
        Lịch tự chạy: <strong>{enabled ? "Đang bật" : "Đang tắt"}</strong>
      </p>
      {enabled && unapproved.length > 0 && (
        <StatusNotice tone="warning">
          Chưa duyệt danh sách máy cho {unapproved.join(", ")}: phần này không chạy cho tới khi bấm Lưu.
          Riviu không tự lấy các máy đang kết nối.
        </StatusNotice>
      )}
      <div className="schedule-scope-line">
        {approvals.map((approval) => (
          <p key={approval.name}>
            {approval.name}: {approval.udids.length
              ? `${approval.udids.length} máy · ${names(approval.udids)}`
              : "chưa có máy nào — chọn máy ở Thiết lập"}
          </p>
        ))}
        <p>Lưu ghi đúng các máy trên; máy cắm sau không được thêm.</p>
      </div>
      <RunTimesPreview
        title={enabled ? "5 lần chạy dự kiến" : "Nếu bật lúc này, 5 lần chạy đầu (dự kiến)"}
        times={times}
        emptyText="Không có lần chạy nào trong 8 ngày tới với khung giờ này."
      />
      <div className="schedule-save-actions">
        <button
          type="button"
          aria-label="Lưu hẹn giờ"
          disabled={busy || blocked || (enabled && !targets.length)}
          aria-describedby={describedBy}
          onClick={() => onSave("keep")}
        >
          Lưu
        </button>
        {enabled ? (
          <button type="button" aria-label="Tắt hẹn giờ" disabled={busy} onClick={() => onSave("disable")}>
            Tắt hẹn giờ
          </button>
        ) : (
          <button
            type="button"
            className="primary"
            aria-label="Lưu & bật hẹn giờ"
            disabled={busy || blocked || !targets.length}
            aria-describedby={describedBy}
            onClick={() => onSave("enable")}
          >
            Lưu & bật
          </button>
        )}
        <span>Giữ Riviu và máy tính đang mở để lịch chạy.</span>
      </div>
    </section>
  );
}
