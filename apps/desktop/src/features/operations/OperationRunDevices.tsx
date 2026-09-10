import { Activity, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { ArrowDown, ArrowLeft, ArrowUp, CheckCircle2, CircleAlert, Clock3, FileSearch, Info, List, RefreshCw, Search } from "lucide-react";
import { operationDeviceLog, operationGetRun } from "../../api";
import { ProgressBar } from "../../components/ProgressBar";
import { StatusChip, WorkspaceTabs } from "../../components/WorkspacePrimitives";
import type { NurtureSessionStatus, OperationRunSummary } from "../../types";
import { activeRun, compactLogEntries, deviceRows, deviceStateCounts, issueState, logMessage, logTime, monitorDeviceName, progressLabel, runProgress, RUN_STATE_LABEL, timelineEntries } from "./operationProgress";
import { useMonitorRead } from "./useMonitorRead";
import { deviceProgress } from "../../nurtureProgress";
import { useMediaQuery } from "../../useMediaQuery";

export function MonitorReadError({ message, retry }: { message: string; retry: () => void }) {
  return <div className="run-monitor-error" role="alert"><span>{message}</span><button type="button" onClick={retry}><RefreshCw size={14} /> Thử lại</button></div>;
}

function DeviceTimeline({ operationId, udid }: { operationId: string; udid: string }) {
  const read = useCallback(() => operationDeviceLog(operationId, udid), [operationId, udid]);
  const state = useMonitorRead(read);
  const [newestFirst, setNewestFirst] = useState(true);
  const rows = useMemo(() => compactLogEntries(timelineEntries(state.value?.entries ?? [])), [state.value]);
  if (state.error) return <MonitorReadError message={`Chưa đọc được nhật ký: ${state.error}`} retry={state.retry} />;
  if (!state.value) return <p className="run-monitor-empty" role="status">Đang đọc nhật ký…</p>;
  return <>
    <div className="run-log-toolbar"><span>{state.value.truncated ? "500 mốc gần nhất" : `${rows.length} mốc ghi nhận`}</span>
      <button type="button" className="ghost" onClick={() => setNewestFirst((value) => !value)}>
        {newestFirst ? <ArrowDown size={14} /> : <ArrowUp size={14} />}{newestFirst ? "Mới nhất trước" : "Cũ nhất trước"}
      </button>
    </div>
    <div className="run-log-scroll">
      {rows.length === 0 ? <p className="run-monitor-empty">Chưa có nhật ký cho máy này.</p> : <ol className="run-device-log" aria-label="Nhật ký theo thời gian">
        {(newestFirst ? [...rows].reverse() : rows).map(({ entry, count, lastAt }) => {
          const message = logMessage(entry);
          const raw = entry.action !== "nurture" || message !== entry.text || !!entry.detail;
          return <li key={entry.id}>
            <time dateTime={entry.at ?? undefined} title={entry.at && !Number.isNaN(Date.parse(entry.at)) ? new Date(entry.at).toLocaleString("vi-VN") : "Nguồn chưa ghi thời gian"}>{logTime(entry.at)}</time>
            <div className="run-log-message">
              <span>{message}</span>
              {count > 1 && <small title={`Lần cuối ${logTime(lastAt)}`}>Lặp {count} lần · đến {logTime(lastAt)}</small>}
              {raw && <details className="run-log-disclosure"><summary title="Xem bản ghi gốc" aria-label={`Bản ghi gốc lúc ${logTime(entry.at)}`}><Info size={14} aria-hidden="true" /></summary>
                <pre>{entry.text ?? `${entry.action} · ${entry.state}`}{entry.detail ? `\n${entry.detail}` : ""}</pre></details>}
            </div>
          </li>;
        })}
      </ol>}
    </div>
  </>;
}

export function OperationRunDevices({ run, labels, sessions, compact = false }: { run: OperationRunSummary; labels: ReadonlyMap<string, string>; sessions: NurtureSessionStatus[]; compact?: boolean }) {
  const read = useCallback(async () => {
    const next = await operationGetRun(run.id);
    if (!next || next.summary.id !== run.id) throw new Error("Tác vụ không còn trong nguồn dữ liệu.");
    return next;
  }, [run.id]);
  const state = useMonitorRead(read);
  const [selected, setSelected] = useState<string | null>(null);
  const [tab, setTab] = useState("log");
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const narrow = useMediaQuery("(max-width: 600px)");
  const singlePane = compact || narrow;
  const deviceListRef = useRef<HTMLDivElement>(null);
  const backRef = useRef<HTMLButtonElement>(null);
  const panelId = useId();
  const tabs = [{ id: "log", label: "Nhật ký", panelId: `${panelId}-log` }, { id: "evidence", label: "Bằng chứng", panelId: `${panelId}-evidence` }];
  const rows = useMemo(() => deviceRows(state.value?.items ?? []).map((row) => {
    const recorded = row.entries[0]?.label;
    const label = row.udid
      ? recorded && /^Máy \d+(?:\s|$)/.test(recorded) ? recorded
        : labels.get(row.udid) ?? (recorded && recorded !== "Máy trong snapshot" ? recorded : `Thiết bị …${row.udid.slice(-6)}`)
      : "Toàn tác vụ";
    return { ...row, label, ...monitorDeviceName(label) };
  }), [state.value, labels]);
  const shown = rows.filter((row) => row.label.toLocaleLowerCase("vi-VN").includes(search.trim().toLocaleLowerCase("vi-VN"))
    && (filter === "all" || (filter === "issues" ? issueState(row.state) : activeRun(row))));
  const selectedRow = shown.find((row) => row.udid === selected);
  const visibleSelectedId = selectedRow?.udid;
  useEffect(() => {
    // The row moves offscreen in narrow master/detail; keep keyboard focus visible.
    if (singlePane && visibleSelectedId !== undefined) backRef.current?.focus();
  }, [singlePane, visibleSelectedId]);
  if (state.error) return <MonitorReadError message={state.error} retry={state.retry} />;
  if (!state.value) return <p className="run-monitor-empty" role="status">Đang đọc tiến độ từng máy…</p>;
  const summary = state.value.summary;
  const fraction = runProgress(summary, sessions);
  const counts = deviceStateCounts(rows);
  const showDevices = () => {
    setSelected(null);
    queueMicrotask(() => deviceListRef.current?.querySelector<HTMLButtonElement>("button")?.focus());
  };
  return <>
    <div className="run-monitor-summary">
      <div className="run-monitor-result"><StatusChip tone={issueState(summary.state) ? "warning" : activeRun(summary) ? "info" : summary.state === "succeeded" ? "success" : "neutral"}>{RUN_STATE_LABEL[summary.state]}</StatusChip>
        <span className="run-processed-count" title={`${summary.completedItems}/${summary.totalItems} mục đã xử lý`}>{summary.completedItems}/{summary.totalItems}</span>
        <strong className="run-percent">{progressLabel(fraction)}</strong>
      </div>
      <ProgressBar fraction={fraction} label="Tiến độ công việc" tone={activeRun(summary) ? "run" : "idle"} />
      <div className="run-monitor-counts" aria-label="Kết quả từng máy">
        <span><CheckCircle2 size={13} aria-hidden="true" /> {counts.succeeded} hoàn tất</span>
        <span className={counts.issues ? "run-attention" : ""}><CircleAlert size={13} aria-hidden="true" /> {counts.issues} cần kiểm tra</span>
        {counts.active > 0 && <span><Clock3 size={13} aria-hidden="true" /> {counts.active} đang chờ/chạy</span>}
        {counts.stopped > 0 && <span>{counts.stopped} dừng/bỏ qua</span>}
      </div>
    </div>
    <div className="run-monitor-detail">
      <aside className="run-monitor-device-pane" aria-label="Tiến độ từng máy" hidden={singlePane && !!selectedRow}>
        <div className="run-device-filters">
          <label className="run-device-search"><Search size={14} aria-hidden="true" /><input name="monitor-device-search" autoComplete="off" type="search" aria-label="Tìm máy trong tác vụ" placeholder="Tìm máy…" value={search} onChange={(event) => setSearch(event.target.value)} /></label>
          <select aria-label="Lọc trạng thái máy" value={filter} onChange={(event) => setFilter(event.target.value)}><option value="all">{counts.total === rows.length ? `Tất cả máy (${counts.total})` : `Tất cả mục (${rows.length})`}</option><option value="issues">Cần kiểm tra</option><option value="active">Đang chờ/chạy</option></select>
        </div>
        <div ref={deviceListRef} className="run-monitor-devices">
          {!shown.length && <p className="run-monitor-empty">Không có máy phù hợp.</p>}
          {shown.map((row) => {
            const status = sessions.find((session) => session.runId === run.sourceId && session.udid === row.udid);
            const fraction = status && activeRun(row) ? Math.min(.99, deviceProgress(status) ?? 0) : row.fraction;
            const Icon = row.state === "succeeded" ? CheckCircle2 : issueState(row.state) ? CircleAlert : Clock3;
            return <button key={row.udid} type="button" className="run-device-row" aria-pressed={selected === row.udid} title={row.label} onClick={() => {
              if (selected !== row.udid) { setSelected(row.udid); setTab("log"); }
            }}>
              <Icon size={16} className={row.state === "succeeded" ? "run-success" : issueState(row.state) ? "run-attention" : "run-state"} aria-hidden="true" />
              <span className="run-device-copy"><strong>{row.name}</strong><small>{row.reviewPublish ? "Cần kiểm tra bài đăng" : row.pendingPublish ? "Chờ xác minh bài đăng" : RUN_STATE_LABEL[row.state]}</small></span>
              {activeRun(row) && <span className="run-percent">{progressLabel(fraction)}</span>}
            </button>;
          })}
        </div>
      </aside>
      <section className="run-monitor-log" aria-label="Chi tiết máy" hidden={singlePane && !selectedRow}>
        {!selectedRow ? <div className="run-monitor-placeholder"><List size={24} /><strong>Chọn máy để xem nhật ký</strong></div> : <>
          <div className="run-inspector-top"><header className="run-device-heading">{singlePane && <button ref={backRef} type="button" className="icon-btn" onClick={showDevices} title="Về danh sách máy" aria-label="Về danh sách máy"><ArrowLeft size={18} /></button>}<div><strong title={selectedRow.label}>{selectedRow.name}</strong>{!compact && selectedRow.model && <small>{selectedRow.model}</small>}</div>
            <StatusChip tone={issueState(selectedRow.state) ? "warning" : selectedRow.state === "succeeded" ? "success" : "neutral"}>{selectedRow.reviewPublish ? "Cần kiểm tra bài đăng" : selectedRow.pendingPublish ? "Chờ xác minh bài đăng" : RUN_STATE_LABEL[selectedRow.state]}</StatusChip>
          </header>
          <WorkspaceTabs label="Chi tiết hoạt động máy" tabs={tabs} value={tab} onChange={setTab} /></div>
          <Activity mode={tab === "log" ? "visible" : "hidden"}><div role="tabpanel" aria-label="Nhật ký" id={`${panelId}-log`} className="run-timeline-panel">{selectedRow.udid ? <DeviceTimeline key={`${run.id}:${selectedRow.udid}`} operationId={run.id} udid={selectedRow.udid} /> : <p className="run-monitor-empty">Nguồn chưa ghi nhật ký theo máy.</p>}</div></Activity>
          {tab === "evidence" && <div role="tabpanel" aria-label="Bằng chứng" id={`${panelId}-evidence`} className="run-evidence-scroll"><ul className="run-item-statuses">
            {selectedRow.entries.map((item) => <li key={item.id}><strong>{item.label}</strong><span>{RUN_STATE_LABEL[item.state]}</span>
              {item.detail && <p>{item.detail}</p>}
              {(item.errorCode || item.evidence) ? <details><summary><FileSearch size={14} /> Dữ liệu xác minh</summary>{item.errorCode && <code>{item.errorCode}</code>}{item.evidence && <pre>{item.evidence}</pre>}</details> : !item.detail && <p>Chưa có bằng chứng được lưu.</p>}
            </li>)}
          </ul></div>}
        </>}
      </section>
    </div>
  </>;
}
