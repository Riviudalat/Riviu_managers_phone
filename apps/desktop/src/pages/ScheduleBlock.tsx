import { useEffect, useRef, useState } from "react";
import {
  deleteSchedule,
  listSchedules,
  listScripts,
  saveSchedule,
} from "../api";
import { SelectionStrip } from "../components/SelectionStrip";
import { NextRunsPreview } from "../components/ScheduleReview";
import { EmptyState, LoadingState, StatusNotice } from "../components/States";
import { IconClock } from "../components/Icons";
import { describeError } from "../describeError";
import { flash, flashError } from "../farmToast";
import { targetsOf } from "../selectionTargets";
import { scheduleDeviceLabel } from "../scheduleScope";
import type { ScheduleItem } from "../types";
import type { SelProps } from "./pageProps";

/** The recurring-schedule editor, used inside the automation page. */
export function ScheduleBlock({
  devices,
  selected,
  onSelectUdids,
}: SelProps) {
  const [items, setItems] = useState<ScheduleItem[]>([]);
  const [scripts, setScripts] = useState<[string, string][]>([]);
  const [name, setName] = useState("");
  const [scriptName, setScriptName] = useState("");
  const [mins, setMins] = useState(60);
  const [loading, setLoading] = useState(true);
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const loadTicket = useRef(0);
  const targets = targetsOf(selected, devices);
  const label = (udid: string) => scheduleDeviceLabel(udid, devices);
  const save = async (enabled: boolean) => {
    try {
      // The schedule stores exactly these phones: that list is its approval and never widens.
      await saveSchedule({
        id: "",
        name,
        scriptName,
        udids: targets,
        everyMinutes: mins,
        enabled,
      });
      await reload();
      flash(`${enabled ? "Đã lưu và bật" : "Đã lưu, đang tắt"} lịch «${name}» mỗi ${mins} phút · ${targets.length} máy`);
    } catch (e) {
      flashError(e);
    }
  };

  const reload = async () => {
    const ticket = ++loadTicket.current;
    setLoading(true);
    setLoadError(null);
    try {
      const nextItems = await listSchedules();
      const scriptsList = await listScripts();
      if (ticket !== loadTicket.current) return;
      setItems(nextItems);
      setScripts(scriptsList);
      if (!scriptName && scriptsList.length) setScriptName(scriptsList[0][0]);
      setLoaded(true);
    } catch (error) {
      if (ticket === loadTicket.current) setLoadError(describeError(error));
    } finally {
      if (ticket === loadTicket.current) setLoading(false);
    }
  };
  useEffect(() => {
    void reload();
    return () => {
      loadTicket.current += 1;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <section style={{ marginTop: 16 }}>
      <h3>Lịch chạy</h3>
      <p className="hint">
        Mỗi lịch chạy một script trên đúng các máy đã chọn; lịch lỗi giữ nguyên và nêu lý do ở đây.
      </p>
      <details className="settings-details" aria-label="Cách lịch chạy">
        <summary>Cách lịch chạy</summary>
        <p className="hint">
          Khoảng lặp tính bằng phút. Xoá hoặc đổi tên script sẽ làm lần chạy kế tiếp thất bại thay vì chạy nhầm nội dung.
        </p>
      </details>
      {loading && !loaded && <LoadingState label="Đang tải lịch chạy…" />}
      {loadError && (
        <StatusNotice
          tone="error"
          action={(
            <button type="button" className="ghost" onClick={() => void reload()}>
              Thử lại lịch chạy
            </button>
          )}
        >
          Không tải được lịch chạy: {loadError}
        </StatusNotice>
      )}
      {loaded && (
        <>
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
        onSelectUdids={onSelectUdids}
      />
      <label>
        Tên
        <input value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <label>
        Kịch bản
        <select value={scriptName} onChange={(e) => setScriptName(e.target.value)}>
          <option value="">—</option>
          {scripts.map(([n]) => (
            <option key={n} value={n}>
              {n}
            </option>
          ))}
        </select>
      </label>
      <label>
        Mỗi (phút)
        <input
          type="number"
          value={mins}
          onChange={(e) => setMins(Number(e.target.value) || 60)}
        />
      </label>
      <div className="schedule-save-actions">
        <button type="button" disabled={!scriptName || !targets.length} onClick={() => void save(false)}>
          Lưu ({targets.length})
        </button>
        <button type="button" className="primary" disabled={!scriptName || !targets.length} onClick={() => void save(true)}>
          Lưu & bật ({targets.length})
        </button>
      </div>
      {targets.length > 0 && (
        <p className="schedule-scope-line">Lưu sẽ duyệt {targets.length} máy: {targets.map(label).join(", ")}.</p>
      )}
      <NextRunsPreview nextDueAt={null} everyMinutes={mins} enabled={false} />
      <div className="job-list" style={{ marginTop: 8 }}>
        {!items.length && (
          <EmptyState
            compact
            icon={<IconClock size={15} />}
            title="Chưa có lịch chạy"
            hint="Chọn kịch bản, máy và khoảng lặp để tạo lịch đầu tiên."
          />
        )}
        {items.map((s) => (
          <article key={s.id} className="job-card">
            <div>
              <strong>{s.name}</strong>
              <span className={`pill ${s.enabled ? "ok" : ""}`}>
                {s.enabled ? "Đang bật" : "Đang tắt"}
              </span>
            </div>
            <p className="hint">
              {s.scriptName} · mỗi {s.everyMinutes} phút · lần tới {s.nextRunAt ?? "chưa lên lịch"}
            </p>
            {/* The schedule's own account of why nothing ran. Before this, a schedule
                whose script had been renamed advanced its timestamps on every tick and
                enqueued nothing, so this card was indistinguishable from a healthy one. */}
            {s.lastError && (
              <p className="hint error" role="alert">
                Lần chạy gần nhất không thực hiện được: {s.lastError}
              </p>
            )}
            <details className="schedule-scope-line">
              <summary>Đã duyệt {s.udids.length} máy</summary>
              <p>{s.udids.map(label).join(", ")}</p>
            </details>
            <NextRunsPreview nextDueAt={s.nextRunAt ?? null} everyMinutes={s.everyMinutes} enabled={s.enabled} compact />
            <button
              type="button"
              className="ghost"
              onClick={async () => {
                try {
                  await saveSchedule({ ...s, enabled: !s.enabled });
                  await reload();
                } catch (error) {
                  setLoadError(describeError(error));
                }
              }}
            >
              {s.enabled ? "Tắt lịch" : "Bật lịch"}
            </button>
            <button
              type="button"
              className="ghost"
              onClick={async () => {
                try {
                  await deleteSchedule(s.id);
                  await reload();
                } catch (error) {
                  setLoadError(describeError(error));
                }
              }}
            >
              Xóa
            </button>
          </article>
        ))}
      </div>
        </>
      )}
    </section>
  );
}
