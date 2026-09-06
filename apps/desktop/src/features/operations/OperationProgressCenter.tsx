import { Activity, useCallback, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Clock3, Maximize2, Minimize2, Minus, MoreHorizontal, Trash2, TriangleAlert, Undo2 } from "lucide-react";
import { nurtureSessionStatus, operationQueryRuns } from "../../api";
import { ProgressBar } from "../../components/ProgressBar";
import type { OperationRunSummary } from "../../types";
import { activeRun, issueState, progressLabel, runOptionLabel, runProgress } from "./operationProgress";
import { useMonitorRead } from "./useMonitorRead";
import { describeError } from "../../describeError";
import { dismissMonitorRecords, monitorRecordKey, readDismissedRecords, visibleMonitorRuns, writeDismissedRecords, type DismissedRecord } from "./monitorRecords";
import { useFloatingMonitor } from "./useFloatingMonitor";
import { MonitorReadError, OperationRunDevices } from "./OperationRunDevices";
import "../../styles/operation-progress.css";

/** Read-only floating monitor. Dismissal never mutates the source run or its audit. */
export function OperationProgressCenter({ deviceLabels }: { deviceLabels: ReadonlyMap<string, string> }) {
  const [expanded, setExpanded] = useState(false);
  const [maximized, setMaximized] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [dismissal, setDismissal] = useState(() => {
    try { return { records: readDismissedRecords(), error: null as string | null }; }
    catch (error) { return { records: [] as DismissedRecord[], error: describeError(error) }; }
  });
  const [undoKeys, setUndoKeys] = useState<string[]>([]);
  const toggleRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDetailsElement>(null);
  const bodyId = useId();
  const floating = useFloatingMonitor(expanded, expanded && maximized);
  const read = useCallback(async () => {
    const page = await operationQueryRuns({ limit: 200, offset: 0, since: new Date(Date.now() - 24 * 60 * 60_000).toISOString() });
    const sessions = page.runs.some((run) => run.kind === "nurture" && activeRun(run)) ? await nurtureSessionStatus() : [];
    return { page, sessions };
  }, []);
  const state = useMonitorRead(read);
  const runs = visibleMonitorRuns(state.value?.page.runs ?? [], dismissal.records);
  const selectedRun = runs.find((run) => run.id === selectedId) ?? runs[0] ?? null;
  const resolvedSelectedId = selectedRun?.id;
  useEffect(() => {
    // Pin the initial selection too: a newly updated run must not steal the open inspector.
    if (resolvedSelectedId && resolvedSelectedId !== selectedId) setSelectedId(resolvedSelectedId);
  }, [resolvedSelectedId, selectedId]);
  useEffect(() => {
    if (!expanded) return;
    const closeMenu = (event: PointerEvent) => {
      if (event.target instanceof Node && !menuRef.current?.contains(event.target) && menuRef.current) menuRef.current.open = false;
    };
    document.addEventListener("pointerdown", closeMenu);
    return () => document.removeEventListener("pointerdown", closeMenu);
  }, [expanded]);
  const dismiss = (selected: OperationRunSummary[]) => {
    if (menuRef.current) menuRef.current.open = false;
    const settled = selected.filter((run) => !activeRun(run));
    try {
      const next = dismissMonitorRecords(dismissal.records, settled);
      writeDismissedRecords(next);
      setDismissal({ records: next, error: null });
      setUndoKeys(settled.map(monitorRecordKey));
      toggleRef.current?.focus();
    } catch (error) { setDismissal((current) => ({ ...current, error: describeError(error) })); }
  };
  const undo = () => {
    try {
      const next = dismissal.records.filter((record) => !undoKeys.includes(record.key));
      writeDismissedRecords(next);
      setDismissal({ records: next, error: null });
      setUndoKeys([]);
    } catch (error) { setDismissal((current) => ({ ...current, error: describeError(error) })); }
  };
  const active = runs.filter(activeRun);
  const shown = active.length ? active : runs.slice(0, 1);
  const progress = shown.map((run) => runProgress(run, state.value?.sessions ?? []));
  const fraction = progress.length && progress.every((value) => value !== null)
    ? progress.reduce((sum, value) => sum + value!, 0) / progress.length : null;
  if (!runs.length && !state.error && !dismissal.records.length && !dismissal.error) return null;
  const minimize = () => { setExpanded(false); toggleRef.current?.focus(); };
  return createPortal(<section ref={floating.ref} style={floating.style}
    className={`run-monitor is-floating${expanded ? " is-expanded" : " is-minimized"}${expanded && maximized ? " is-maximized" : " is-compact"}`}
    role={expanded ? "dialog" : "region"} aria-modal={expanded ? false : undefined} aria-label="Cửa sổ tiến trình"
    onKeyDown={(event) => {
      if (event.key !== "Escape" || !expanded) return;
      event.stopPropagation();
      if (menuRef.current?.open) { menuRef.current.open = false; menuRef.current.querySelector("summary")?.focus(); }
      else minimize();
    }}>
    <header className="run-monitor-titlebar" {...floating.handle}>
      <button ref={toggleRef} type="button" className="run-monitor-toggle" aria-label="Tiến trình công việc"
        aria-expanded={expanded} aria-controls={bodyId} onClick={() => setExpanded((value) => !value)}>
        <Clock3 size={16} aria-hidden="true" /><strong>{expanded ? "Theo dõi tác vụ" : state.error ? "Chưa đọc được tiến trình" : active.length ? `${active.length} tác vụ đang chạy` : runs.length ? `${runs.length} tác vụ đã kết thúc` : "Không còn bản ghi"}</strong>
      </button>
      {expanded && <span className="run-monitor-total">{runs.length} bản ghi</span>}
      {!expanded && <span className="run-percent">{!runs.length && !state.error ? "—" : state.error ? "?" : progressLabel(fraction)}</span>}
      {!expanded && shown.some((run) => issueState(run.state) || run.issueCount > 0) && <span className="run-attention" title="Có kết quả cần kiểm tra" aria-label="Có kết quả cần kiểm tra"><TriangleAlert size={14} /></span>}
      {expanded && <button type="button" className="icon-btn" data-monitor-no-drag
        aria-label={maximized ? "Khôi phục kích thước" : "Phóng rộng tiến trình"} title={maximized ? "Khôi phục kích thước" : "Phóng rộng tiến trình"}
        onClick={() => setMaximized((value) => !value)}>{maximized ? <Minimize2 size={16} /> : <Maximize2 size={16} />}</button>}
      <button type="button" className="icon-btn" data-monitor-no-drag aria-label={expanded ? "Thu nhỏ tiến trình" : "Mở rộng tiến trình"}
        title={expanded ? "Thu nhỏ tiến trình" : "Mở rộng tiến trình"} onClick={() => expanded ? minimize() : setExpanded(true)}>
        {expanded ? <Minus size={16} /> : <Maximize2 size={15} />}
      </button>
    </header>
    {!expanded && <div className="run-monitor-overview">
      <ProgressBar fraction={state.error ? null : fraction} label="Tiến độ công việc" tone={active.length ? "run" : "idle"} />
    </div>}
    <Activity mode={expanded ? "visible" : "hidden"}><div className="run-monitor-body" id={bodyId}>
      <div className="run-monitor-tools">
        <label className="run-selector-label" htmlFor={`${bodyId}-run`}>Phiên chạy</label>
        <select id={`${bodyId}-run`} aria-label="Chọn tác vụ theo dõi" title={selectedRun ? runOptionLabel(selectedRun) : undefined} value={selectedRun?.id ?? ""} onChange={(event) => setSelectedId(event.target.value)} disabled={!runs.length}>
          {!runs.length && <option value="">Không còn bản ghi</option>}
          {runs.map((run) => <option key={run.id} value={run.id}>{runOptionLabel(run)}</option>)}
        </select>
        {selectedRun && <button type="button" className="icon-btn" title="Xoá bản ghi đang xem khỏi cửa sổ theo dõi" aria-label={`Xoá bản ghi ${selectedRun.title}`}
          disabled={!!state.error || activeRun(selectedRun)} onClick={() => dismiss([selectedRun])}><Trash2 size={16} /></button>}
        <details ref={menuRef} className="run-monitor-history-menu"><summary aria-label="Tuỳ chọn bản ghi" title="Tuỳ chọn bản ghi"><MoreHorizontal size={18} aria-hidden="true" /></summary>
          <div><button type="button" disabled={!!state.error || !runs.some((run) => !activeRun(run))} onClick={() => dismiss(runs)}>Xoá các bản ghi đã kết thúc</button></div>
        </details>
      </div>
      {undoKeys.length > 0 && <div className="run-monitor-undo" role="status"><span>Đã xoá {undoKeys.length} bản ghi khỏi cửa sổ.</span><button type="button" className="ghost" onClick={undo}><Undo2 size={14} /> Hoàn tác xoá</button></div>}
      {dismissal.error && <MonitorReadError message={`Chưa lưu được danh sách bản ghi: ${dismissal.error}`} retry={() => {
        try { setDismissal({ records: readDismissedRecords(), error: null }); }
        catch (error) { setDismissal((current) => ({ ...current, error: describeError(error) })); }
      }} />}
      {state.error ? <MonitorReadError message={state.error} retry={state.retry} /> : selectedRun
        ? <OperationRunDevices key={selectedRun.id} run={selectedRun} labels={deviceLabels} sessions={state.value?.sessions ?? []} compact={!maximized} />
        : <p className="run-monitor-empty">Không còn bản ghi trong cửa sổ này.</p>}
      {state.value?.page.hasMore && <p className="run-monitor-history-note">Các tác vụ khác nằm trong trang Tác vụ.</p>}
    </div></Activity>
  </section>, document.body);
}
