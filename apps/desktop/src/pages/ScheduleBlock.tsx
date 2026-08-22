import { useEffect, useState } from "react";
import {
  deleteSchedule,
  exampleScript,
  listSchedules,
  listScripts,
  saveSchedule,
  saveScript,
} from "../api";
import { SelectionStrip } from "../components/SelectionStrip";
import { flash, flashError } from "../farmToast";
import { targetsOf } from "../selectionTargets";
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
  const [name, setName] = useState("hourly");
  const [scriptName, setScriptName] = useState("");
  const [mins, setMins] = useState(60);
  const targets = targetsOf(selected, devices);

  const reload = async () => {
    setItems(await listSchedules());
    let scriptsList = await listScripts();
    if (!scriptsList.length) {
      const body = await exampleScript();
      await saveScript("example", body);
      scriptsList = await listScripts();
    }
    setScripts(scriptsList);
    if (!scriptName && scriptsList.length) setScriptName(scriptsList[0][0]);
  };
  useEffect(() => {
    reload().catch((e) => flashError(e));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <section style={{ marginTop: 16 }}>
      <h3>Lịch chạy</h3>
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
        Script
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
      <button
        type="button"
        className="primary"
        disabled={!scriptName || !targets.length}
        onClick={async () => {
          try {
            await saveSchedule({
              id: "",
              name,
              scriptName,
              udids: targets,
              everyMinutes: mins,
              enabled: true,
            });
            await reload();
            flash(`Schedule «${name}» mỗi ${mins} phút · ${targets.length} máy`);
          } catch (e) {
            flashError(e);
          }
        }}
      >
        Lưu schedule ({targets.length})
      </button>
      <div className="job-list" style={{ marginTop: 8 }}>
        {items.map((s) => (
          <article key={s.id} className="job-card">
            <div>
              <strong>{s.name}</strong>
              <span className="pill">{s.enabled ? "on" : "off"}</span>
            </div>
            <p className="hint">
              {s.scriptName} · every {s.everyMinutes}m · next {s.nextRunAt ?? "—"}
            </p>
            {/* The schedule's own account of why nothing ran. Before this, a schedule
                whose script had been renamed advanced its timestamps on every tick and
                enqueued nothing, so this card was indistinguishable from a healthy one. */}
            {s.lastError && (
              <p className="hint error" role="alert">
                Lần chạy gần nhất không thực hiện được: {s.lastError}
              </p>
            )}
            <button
              type="button"
              className="ghost"
              onClick={async () => {
                await deleteSchedule(s.id);
                await reload();
              }}
            >
              Xóa
            </button>
          </article>
        ))}
      </div>
    </section>
  );
}
