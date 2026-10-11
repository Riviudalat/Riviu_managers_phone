import { useRef, useState } from "react";
import { CalendarClock, RefreshCw } from "lucide-react";
import {
  automationList,
  automationScheduleList,
  automationScheduleUpdate,
} from "../api";
import { AutomationScheduleControl } from "../components/AutomationScheduleControl";
import { needsGroups, useDeviceGroups, useScheduleTargets } from "../useScheduleScope";
import type { AutomationScheduleV1, DeviceInfo, DeviceMeta } from "../types";
import { describeError } from "../describeError";
import { localTimeZoneLabel, scheduleErrorLabel, scheduleScope, type ScheduleScope } from "../scheduleScope";
import { EmptyState, LoadingState, StatusNotice } from "../components/States";
import { useAsyncList } from "../useAsyncList";
import "../styles/operator-schedules.css";

async function readSchedules() {
  const [profiles, rows] = await Promise.all([automationList(), automationScheduleList()]);
  return { profiles, rows };
}

function scopeCell(scope: ScheduleScope, approved: string[] | null) {
  switch (scope.kind) {
    case "unknown":
      return approved ? `${approved.length} máy` : "Đang đọc…";
    case "missingGroup":
      return "Nhóm không còn";
    case "unapproved":
      return "Chưa duyệt";
    case "changed":
      return `Phạm vi đổi (+${scope.added.length} −${scope.removed.length}) · cần duyệt lại`;
    case "approved":
      return `${(approved ?? scope.designated).length} máy`;
  }
}

export function OperatorSchedulesPage({
  devices = [],
  metas,
  onCreateConfiguration,
}: {
  devices?: DeviceInfo[];
  metas?: Map<string, DeviceMeta>;
  /** Opens the place a configuration is made (Nuôi TikTok → Hẹn giờ). */
  onCreateConfiguration?: () => void;
}) {
  const [selected, setSelected] = useState("");
  const [error, setError] = useState<string | null>(null),
    [busy, setBusy] = useState(false);
  const profileSelect = useRef<HTMLSelectElement>(null);
  const { data, error: loadError, loading, initialLoading, refreshing, load } = useAsyncList(readSchedules);
  const profiles = data?.profiles ?? [];
  const rows = data?.rows ?? [];
  const targets = useScheduleTargets(rows);
  const groups = useDeviceGroups(needsGroups(targets.values()));
  const scopeOf = (row: (typeof rows)[number]) =>
    scheduleScope(row.approvedUdids, targets.get(`${row.definitionId}:${row.definitionRevision}`), devices, groups);
  return (
    <section className="operator-schedules">
      <div className="operator-toolbar">
        <CalendarClock size={20} />
        <strong>Lịch tự động</strong>
        <div className="grow" />
        <button type="button" aria-label="Làm mới lịch chạy" disabled={loading} onClick={() => void load()}>
          <RefreshCw size={16} aria-hidden="true" />
          Làm mới
        </button>
      </div>
      {error && <StatusNotice tone="error">{error}</StatusNotice>}
      {initialLoading && <LoadingState label="Đang tải lịch chạy…" />}
      {refreshing && <LoadingState label="Đang làm mới lịch chạy…" />}
      {loadError && (
        <StatusNotice tone="error" action={<button type="button" onClick={() => void load()}>Thử lại</button>}>
          Không tải được lịch chạy: {loadError}{data !== undefined && " · Đang giữ dữ liệu lần tải trước."}
        </StatusNotice>
      )}
      <div className="operator-schedule-layout">
        <section className="operator-schedule-list" aria-label="Danh sách lịch">
          {rows.length > 0 && <p className="operator-schedule-zone">Mọi giờ theo {localTimeZoneLabel()}.</p>}
          {rows.length > 0 && <table aria-label="Lịch chạy" aria-busy={refreshing}>
            <thead>
              <tr>
                <th>Tên lịch</th>
                <th>Ứng dụng</th>
                <th>Phiên bản</th>
                <th>Trạng thái</th>
                <th>Máy đã duyệt</th>
                <th>Lần chạy kế tiếp</th>
                <th>Kết quả gần nhất</th>
                <th>Thao tác</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => {
                const scope = scopeOf(row);
                // Switching on waits for a scope that matches the approval; switching off never does.
                const enableBlocked = !row.enabled && scope.kind !== "approved";
                return (
                  <tr key={row.id}>
                    <td>{row.name}</td>
                    <td>{profiles.find((p) => p.id === row.definitionId)?.name ?? row.definitionId}</td>
                    <td>{row.definitionRevision}</td>
                    <td>{row.enabled ? "Đang bật" : "Đã tắt"}</td>
                    <td>{scopeCell(scope, row.approvedUdids)}</td>
                    <td>{row.nextDueAt ? new Date(row.nextDueAt).toLocaleString("vi-VN") : "—"}</td>
                    <td>{scheduleErrorLabel(row.lastErrorCode) ?? "—"}</td>
                    <td>
                      <button type="button" disabled={busy || enableBlocked}
                        title={enableBlocked ? "Duyệt danh sách máy trong Chỉnh sửa trước khi bật" : undefined}
                        onClick={async () => {
                          setBusy(true);
                          try {
                            await automationScheduleUpdate(
                              row.id, row.revision, row.name, row.definitionId,
                              row.definitionRevision, !row.enabled,
                              row.schedule as AutomationScheduleV1,
                            );
                            setError(null);
                            await load();
                          } catch (cause) {
                            setError(describeError(cause));
                          } finally {
                            setBusy(false);
                          }
                        }}>
                        {row.enabled ? "Tắt lịch" : "Bật lịch"}
                      </button>
                      <button type="button" onClick={() => setSelected(row.definitionId)}>Chỉnh sửa</button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>}
          {!loading && !loadError && data !== undefined && rows.length === 0 && (profiles.length === 0 ? (
            <EmptyState
              compact
              icon={<CalendarClock size={17} />}
              title="Chưa có lịch"
              hint="Lịch chạy một cấu hình đã lưu. Mở Hẹn giờ của Nuôi TikTok, chọn máy và bấm Lưu hoặc Lưu & bật để tạo cấu hình cùng lịch đầu tiên. Tương tác cũng có tab Hẹn giờ."
              action={onCreateConfiguration && (
                <button type="button" className="primary" onClick={onCreateConfiguration}>Tạo cấu hình để hẹn giờ</button>
              )}
            />
          ) : (
            <EmptyState
              compact
              icon={<CalendarClock size={17} />}
              title="Chưa có lịch"
              hint="Chọn một cấu hình ứng dụng, đặt tên và chu kỳ, rồi bấm Lưu hoặc Lưu & bật."
              action={(
                <button type="button" className="primary" onClick={() => {
                  setSelected((current) => current || profiles[0].id);
                  profileSelect.current?.focus();
                }}>Tạo lịch từ cấu hình</button>
              )}
            />
          ))}
        </section>
        <section className="operator-schedule-detail" aria-label="Cấu hình lịch">
          <label>
            Cấu hình ứng dụng
            <select ref={profileSelect} value={selected} onChange={(e) => setSelected(e.target.value)}>
              <option value="">Chọn cấu hình…</option>
              {profiles.map((p) => <option key={p.id} value={p.id}>
                {p.name} · bản {p.latestRevision}
              </option>)}
            </select>
          </label>
          {selected && <AutomationScheduleControl
            key={selected}
            profile={profiles.find((p) => p.id === selected) ?? null}
            devices={devices}
            metas={metas}
          />}
        </section>
      </div>
    </section>
  );
}
