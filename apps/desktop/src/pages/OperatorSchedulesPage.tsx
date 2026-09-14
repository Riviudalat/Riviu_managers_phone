import { useCallback, useEffect, useState } from "react";
import { CalendarClock, RefreshCw } from "lucide-react";
import {
  automationList,
  automationScheduleList,
  automationScheduleUpdate,
} from "../api";
import { AutomationScheduleControl } from "../components/AutomationScheduleControl";
import type {
  AutomationDefinition,
  AutomationSchedule,
  AutomationScheduleV1,
} from "../types";
import { describeError } from "../describeError";

export function OperatorSchedulesPage() {
  const [profiles, setProfiles] = useState<AutomationDefinition[]>([]),
    [rows, setRows] = useState<AutomationSchedule[]>([]),
    [selected, setSelected] = useState("");
  const [error, setError] = useState<string | null>(null),
    [busy, setBusy] = useState(false);
  const load = useCallback(async () => {
    try {
      const [a, b] = await Promise.all([
        automationList(),
        automationScheduleList(),
      ]);
      setProfiles(a);
      setRows(b);
      setError(null);
    } catch (cause) {
      setError(describeError(cause));
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);
  return (
    <section className="operator-schedules">
      <div className="operator-toolbar">
        <CalendarClock size={20} />
        <strong>Lịch tự động</strong>
        <button type="button" onClick={() => void load()}>
          <RefreshCw size={16} />
          Làm mới
        </button>
      </div>
      {error && <p role="alert">{error}</p>}
      <table>
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
      </table>
      {!rows.length && (
        <p className="operator-empty">
          Chưa có lịch. Chọn cấu hình ứng dụng để tạo lịch chạy.
        </p>
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
