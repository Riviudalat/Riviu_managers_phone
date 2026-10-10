import { useEffect, useId, useMemo, useRef, useState } from "react";
import {
  Bell,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
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
  type ToastRecord,
} from "../toastStore";

type ActivityFilter = "all" | "attention";

const KIND_LABEL: Record<ToastKind, string> = {
  ok: "Hoàn tất",
  info: "Thông tin",
  warn: "Cần kiểm tra",
  error: "Thất bại",
};

const KIND_RANK: Record<ToastKind, number> = { ok: 0, info: 1, warn: 2, error: 3 };

/** One cause with every record that carried it; device IDs are their union, in record order. */
interface ActivityGroup {
  cause: string;
  records: ToastRecord[];
  udids: string[];
  kind: ToastKind;
}

type ActivityRow =
  | { type: "record"; record: ToastRecord }
  | { type: "group"; group: ActivityGroup };

/**
 * Newest first, one row per cause. A cause seen once stays a plain record. Entries without a
 * typed cause are never grouped: grouping on prose would merge unrelated failures.
 */
function groupActivities(records: ToastRecord[]): ActivityRow[] {
  const byCause = new Map<string, ToastRecord[]>();
  for (const record of records) {
    if (record.cause) byCause.set(record.cause, [...(byCause.get(record.cause) ?? []), record]);
  }
  const rows: ActivityRow[] = [];
  const placed = new Set<string>();
  for (const record of records) {
    const members = record.cause ? byCause.get(record.cause) ?? [] : [];
    if (!record.cause || members.length < 2) {
      rows.push({ type: "record", record });
      continue;
    }
    if (placed.has(record.cause)) continue;
    placed.add(record.cause);
    rows.push({
      type: "group",
      group: {
        cause: record.cause,
        records: members,
        udids: [...new Set(members.flatMap((member) => member.udids))],
        kind: members.reduce<ToastKind>(
          (worst, member) => (KIND_RANK[member.kind] > KIND_RANK[worst] ? member.kind : worst),
          "ok",
        ),
      },
    });
  }
  return rows;
}

/** The human sentence first; a code the error prefixed to it moves under Chi tiết. */
function humanDetail(record: ToastRecord): string | undefined {
  const prefix = record.cause ? `${record.cause}: ` : "";
  return prefix && record.detail?.startsWith(prefix)
    ? record.detail.slice(prefix.length)
    : record.detail;
}

function runIdsOf(records: readonly ToastRecord[]): string[] {
  return [...new Set(records.flatMap((record) => (record.runId ? [record.runId] : [])))];
}

function ActivityFacts({
  cause,
  udids,
  runIds,
}: {
  cause?: string;
  udids: readonly string[];
  runIds: readonly string[];
}) {
  if (!cause && udids.length === 0 && runIds.length === 0) return null;
  return (
    <details className="activity-center-facts">
      <summary>Chi tiết</summary>
      <dl>
        {cause && (<><dt>Mã nguyên nhân</dt><dd><code>{cause}</code></dd></>)}
        {runIds.length > 0 && (<><dt>Mã lượt chạy</dt><dd><code>{runIds.join(", ")}</code></dd></>)}
        {udids.length > 0 && (<><dt>Mã thiết bị</dt><dd><code>{udids.join(", ")}</code></dd></>)}
      </dl>
    </details>
  );
}

function SelectDevicesButton({
  udids,
  onSelectDevices,
}: {
  udids: readonly string[];
  onSelectDevices?: (udids: string[]) => void;
}) {
  if (!onSelectDevices || udids.length === 0) return null;
  return (
    <button
      type="button"
      className="activity-center-select"
      onClick={() => onSelectDevices([...udids])}
    >
      Chọn {udids.length} máy này
    </button>
  );
}

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
 * A RAM-only history and a dismissible compact outcome; diagnostics stay in the history.
 * Durable evidence lives in the run monitors and the evidence ledger, never here.
 */
export function ActivityCenter({
  onSelectDevices,
}: {
  /** Replace the device selection with exactly these IDs. */
  onSelectDevices?: (udids: string[]) => void;
} = {}) {
  const activities = useToasts();
  const [dismissedId, setDismissedId] = useState(0);
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState<ActivityFilter>("all");
  const [lastSeenId, setLastSeenId] = useState(0);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(() => new Set());
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
  const rows = useMemo(() => groupActivities(visible), [visible]);
  const selectDevices = onSelectDevices
    ? (udids: string[]) => {
        onSelectDevices(udids);
        setOpen(false);
        triggerRef.current?.focus();
      }
    : undefined;
  const toggleGroup = (cause: string) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (!next.delete(cause)) next.add(cause);
      return next;
    });

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

  const renderRecord = (activity: ToastRecord) => {
    const detail = humanDetail(activity);
    return (
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
          {detail && <p>{detail}</p>}
          <SelectDevicesButton udids={activity.udids} onSelectDevices={selectDevices} />
          <ActivityFacts
            cause={activity.cause}
            udids={activity.udids}
            runIds={activity.runId ? [activity.runId] : []}
          />
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
    );
  };

  const renderGroup = (group: ActivityGroup) => {
    const [newest] = group.records;
    // A title usually names one phone; when the records differ, the shared sentence leads.
    const sameTitle = group.records.every((record) => record.title === newest.title);
    const detail = humanDetail(newest);
    const headline = sameTitle ? newest.title : detail ?? newest.title;
    const isOpen = expanded.has(group.cause);
    const listId = `${panelId}-cause-${group.cause}`;
    return (
      <li key={`cause:${group.cause}`} className={`activity-center-group is-${group.kind}`}>
        <span className="activity-center-icon">
          <ActivityIcon kind={group.kind} />
        </span>
        <div className="activity-center-copy">
          <div>
            <strong>{headline}</strong>
            <time dateTime={new Date(newest.createdAt).toISOString()}>
              {formatActivityTime(newest.createdAt)}
            </time>
          </div>
          <span className="activity-center-kind">
            {group.records.length} mục{group.udids.length > 0 ? ` · ${group.udids.length} máy` : ""}
          </span>
          {sameTitle && detail && <p>{detail}</p>}
          <div className="activity-center-group-actions">
            <button
              type="button"
              className="activity-center-expand"
              aria-expanded={isOpen}
              aria-controls={listId}
              onClick={() => toggleGroup(group.cause)}
            >
              {isOpen
                ? <ChevronDown size={14} aria-hidden="true" />
                : <ChevronRight size={14} aria-hidden="true" />}
              {isOpen ? "Thu gọn" : `Xem ${group.records.length} mục`}
            </button>
            <SelectDevicesButton udids={group.udids} onSelectDevices={selectDevices} />
          </div>
          <ActivityFacts cause={group.cause} udids={group.udids} runIds={runIdsOf(group.records)} />
          {isOpen && (
            <ol
              id={listId}
              className="activity-center-group-records"
              aria-label={`Các mục cùng nguyên nhân: ${headline}`}
            >
              {group.records.map(renderRecord)}
            </ol>
          )}
        </div>
      </li>
    );
  };

  return (
    <div className="activity-center" ref={rootRef}>
      {latest && latest.id > dismissedId && (
        <div
          className={`activity-center-current is-${latest.kind}`}
          role="status"
          aria-live="polite"
          aria-atomic="true"
        >
          <ActivityIcon kind={latest.kind} />
          <span className="activity-center-current-copy">
            <strong>{latest.title}</strong>
            {latest.summary && <small>{latest.summary}</small>}
          </span>
          <button type="button" onClick={() => { setFilter("all"); setOpen(true); }}>Chi tiết</button>
          <button type="button" className="icon-btn" aria-label="Đóng thông báo" onClick={() => setDismissedId(latest.id)}><X size={16} aria-hidden="true" /></button>
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
              <p>Nhật ký tạm trong RAM, mất khi đóng app; không phải bằng chứng lưu bền.</p>
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
              <span>Kết quả thao tác sẽ được ghi tạm tại đây.</span>
            </div>
          ) : (
            <ol className="activity-center-list" aria-label="Lịch sử hoạt động">
              {rows.map((row) =>
                row.type === "group" ? renderGroup(row.group) : renderRecord(row.record),
              )}
            </ol>
          )}
        </aside>
      )}
    </div>
  );
}
