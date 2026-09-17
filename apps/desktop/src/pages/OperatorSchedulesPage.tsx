import { useState } from "react";
import { CalendarClock, RefreshCw } from "lucide-react";
import {
  automationList,
  automationScheduleList,
  automationScheduleUpdate,
} from "../api";
import { AutomationScheduleControl } from "../components/AutomationScheduleControl";
import type { AutomationScheduleV1 } from "../types";
import { describeError } from "../describeError";
import { EmptyState, LoadingState, StatusNotice } from "../components/States";
import { useAsyncList } from "../useAsyncList";

async function readSchedules() {
  const [profiles, rows] = await Promise.all([automationList(), automationScheduleList()]);
  return { profiles, rows };
}

export function OperatorSchedulesPage() {
  const [selected, setSelected] = useState("");
  const [error, setError] = useState<string | null>(null),
    [busy, setBusy] = useState(false);
  const { data, error: loadError, loading, initialLoading, refreshing, load } = useAsyncList(readSchedules);
  const profiles = data?.profiles ?? [];
  const rows = data?.rows ?? [];
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
      {rows.length > 0 && <table aria-label="Lịch chạy" aria-busy={refreshing}>
        <thead>
          <tr>
            <th>Tên lịch</th>
            <th>Ứng dụng</th>
            <th>Phiên bản</th>
            <th>Trạng thái</th>
            <th>Lần chạy kế tiếp</th>
            <th>Kết quả gần nhất</th>
            <th>Thao tác</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.id}>
              <td>{row.name}</td>
              <td>
                {profiles.find((p) => p.id === row.definitionId)?.name ??
                  row.definitionId}
              </td>
              <td>{row.definitionRevision}</td>
              <td>{row.enabled ? "Đang bật" : "Đã tắt"}</td>
              <td>{row.nextDueAt?new Date(row.nextDueAt).toLocaleString("vi-VN"):"—"}</td>
              <td>{row.lastErrorCode??"—"}</td>
              <td>
                <button
                  type="button"
                  disabled={busy}
                  onClick={async () => {
                    setBusy(true);
                    try {
                      await automationScheduleUpdate(
                        row.id,
                        row.revision,
                        row.name,
                        row.definitionId,
                        row.definitionRevision,
                        !row.enabled,
                        row.schedule as AutomationScheduleV1,
                      );
                      setError(null);
                      await load();
                    } catch (cause) {
                      setError(describeError(cause));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  {row.enabled ? "Tắt lịch" : "Bật lịch"}
                </button>
                <button
                  type="button"
                  onClick={() => setSelected(row.definitionId)}
                >
                  Chỉnh sửa
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>}
      {!loading && !loadError && data !== undefined && rows.length === 0 && (
        <EmptyState compact title="Chưa có lịch" hint="Chọn cấu hình ứng dụng để tạo lịch chạy." />
      )}
      <label>
        Cấu hình ứng dụng
        <select value={selected} onChange={(e) => setSelected(e.target.value)}>
          <option value="">Chọn cấu hình…</option>
          {profiles.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name} · bản {p.latestRevision}
            </option>
          ))}
        </select>
      </label>
      {selected && (
        <AutomationScheduleControl
          key={selected}
          profile={profiles.find((p) => p.id === selected) ?? null}
        />
      )}
    </section>
  );
}
