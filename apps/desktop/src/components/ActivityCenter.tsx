import { useEffect, useId, useMemo, useRef, useState } from "react";
import {
  Bell,
  CheckCircle2,
  CircleAlert,
  Info,
  ListFilter,
  Trash2,
  TriangleAlert,
  X,
} from "lucide-react";

import {
  clearToasts,
  dismissToast,
  useToasts,
  type ToastKind,
} from "../toastStore";

type ActivityFilter = "all" | "attention";

const KIND_LABEL: Record<ToastKind, string> = {
  ok: "Hoàn tất",
  info: "Thông tin",
  warn: "Cần kiểm tra",
  error: "Thất bại",
};

function ActivityIcon({ kind }: { kind: ToastKind }) {
  if (kind === "ok") return <CheckCircle2 size={16} aria-hidden="true" />;
  if (kind === "error") return <CircleAlert size={16} aria-hidden="true" />;
  if (kind === "warn") return <TriangleAlert size={16} aria-hidden="true" />;
  return <Info size={16} aria-hidden="true" />;
}

function formatActivityTime(value: number): string {
  return new Intl.DateTimeFormat("vi-VN", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(value);
}

/**
 * Operator-controlled history for cross-page outcomes. New entries only update the badge;
 * they never float over the current task or disappear before the operator can inspect them.
 */
export function ActivityCenter() {
  const activities = useToasts();
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState<ActivityFilter>("all");
  const [lastSeenId, setLastSeenId] = useState(0);
  const panelId = useId();
  const titleId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLElement>(null);
  const latest = activities[0];
  const unread = activities.filter((activity) => activity.id > lastSeenId).length;
  const attention = activities.filter(
    (activity) => activity.kind === "warn" || activity.kind === "error",
  ).length;
  const visible = useMemo(
    () =>
      filter === "attention"
        ? activities.filter(
            (activity) => activity.kind === "warn" || activity.kind === "error",
          )
        : activities,
    [activities, filter],
  );

  useEffect(() => {
    if (!open || activities.length === 0) return;
    setLastSeenId(activities[0].id);
  }, [activities, open]);

  useEffect(() => {
    if (!open) return;
    panelRef.current?.focus();
    const onPointerDown = (event: PointerEvent) => {
      if (rootRef.current?.contains(event.target as Node)) return;
      setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setOpen(false);
      triggerRef.current?.focus();
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <div className="activity-center" ref={rootRef}>
      {latest && (
        <div
          className={`activity-center-current is-${latest.kind}`}
          role={latest.kind === "error" ? "alert" : "status"}
          aria-live={latest.kind === "error" ? "assertive" : "polite"}
          aria-atomic="true"
          title={latest.detail ? `${latest.title}: ${latest.detail}` : latest.title}
        >
          <ActivityIcon kind={latest.kind} />
          <span className="activity-center-current-copy">
            <strong>{latest.title}</strong>
            {latest.detail && <small>{latest.detail}</small>}
          </span>
        </div>
      )}
      <button
        ref={triggerRef}
        type="button"
        className={`icon-btn activity-center-trigger ${open ? "active" : ""}`}
        title="Hoạt động"
        aria-label={unread ? `Hoạt động, ${unread} mục chưa xem` : "Hoạt động"}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((current) => !current)}
      >
        <Bell size={17} aria-hidden="true" />
        {unread > 0 && (
          <span className="activity-center-badge" aria-hidden="true">
            {unread > 99 ? "99+" : unread}
          </span>
        )}
      </button>

      {open && (
        <aside
          ref={panelRef}
          id={panelId}
          className="activity-center-panel"
          role="dialog"
          aria-modal="false"
          aria-labelledby={titleId}
          tabIndex={-1}
        >
          <header className="activity-center-header">
            <div>
              <h2 id={titleId}>Hoạt động</h2>
              <p>Kết quả thao tác trên toàn hệ thống</p>
            </div>
            <div className="activity-center-header-actions">
              <button
                type="button"
                className="icon-btn"
                title="Xóa lịch sử"
                aria-label="Xóa toàn bộ lịch sử hoạt động"
                disabled={activities.length === 0}
                onClick={clearToasts}
              >
                <Trash2 size={16} aria-hidden="true" />
              </button>
              <button
                type="button"
                className="icon-btn"
                title="Đóng"
                aria-label="Đóng trung tâm hoạt động"
                onClick={() => {
                  setOpen(false);
                  triggerRef.current?.focus();
                }}
              >
                <X size={17} aria-hidden="true" />
              </button>
            </div>
          </header>

          <div className="activity-center-filters" aria-label="Lọc hoạt động">
            <ListFilter size={15} aria-hidden="true" />
            <button
              type="button"
              aria-pressed={filter === "all"}
              onClick={() => setFilter("all")}
            >
              Tất cả <span>{activities.length}</span>
            </button>
            <button
              type="button"
              aria-pressed={filter === "attention"}
              onClick={() => setFilter("attention")}
            >
              Cần xử lý <span>{attention}</span>
            </button>
          </div>

          {visible.length === 0 ? (
            <div className="activity-center-empty">
              <Bell size={20} aria-hidden="true" />
              <strong>{activities.length ? "Không có mục cần xử lý" : "Chưa có hoạt động"}</strong>
              <span>Kết quả thao tác sẽ được lưu tại đây.</span>
            </div>
          ) : (
            <ol className="activity-center-list" aria-label="Lịch sử hoạt động">
              {visible.map((activity) => (
                <li key={activity.id} className={`is-${activity.kind}`}>
                  <span className="activity-center-icon">
                    <ActivityIcon kind={activity.kind} />
                  </span>
                  <div className="activity-center-copy">
                    <div>
                      <strong>{activity.title}</strong>
                      <time dateTime={new Date(activity.createdAt).toISOString()}>
                        {formatActivityTime(activity.createdAt)}
                      </time>
                    </div>
                    <span className="activity-center-kind">{KIND_LABEL[activity.kind]}</span>
                    {activity.detail && <p>{activity.detail}</p>}
                  </div>
                  <button
                    type="button"
                    className="icon-btn activity-center-dismiss"
                    title="Xóa mục"
                    aria-label={`Xóa hoạt động: ${activity.title}`}
                    onClick={() => dismissToast(activity.id)}
                  >
                    <X size={14} aria-hidden="true" />
                  </button>
                </li>
              ))}
            </ol>
          )}
        </aside>
      )}
    </div>
  );
}
