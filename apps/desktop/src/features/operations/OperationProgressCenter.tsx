import { Activity, useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { Clock3, Inbox, Maximize2, Minimize2, Square, TriangleAlert, X } from "lucide-react";
import { operationGetRun, publishStartStatus, publishCancelUnacceptedStart, nurtureSessionStatus, operationQueryRuns, operationStop, operationStopStatus, type OperationStopResult } from "../../api";
import { ProgressBar } from "../../components/ProgressBar";
import { WorkspaceTabs } from "../../components/WorkspacePrimitives";
import type { OperationRunSummary } from "../../types";
import { activeRun, progressLabel, runOptionLabel, runProgress } from "./operationProgress";
import { useMonitorRead } from "./useMonitorRead";
import { describeError } from "../../describeError";
import { dismissMonitorRecords, monitorRecordKey, readDismissedRecords, writeDismissedRecords, type DismissedRecord } from "./monitorRecords";
import { MonitorReadError, OperationRunDevices } from "./OperationRunDevices";
import { acknowledgePublishStart, publishStageLabel, PUBLISH_START_PENDING, PUBLISH_START_ACKNOWLEDGED, PUBLISH_START_RETIRED, readPendingPublishStart, retirePublishStart, type PendingPublishStart } from "./publishStartBridge";
import { AttentionQueueList } from "./AttentionQueue";
import type { DeviceAttention, QueuePhone } from "./attentionQueueModel";
import { useAttentionQueue } from "./useAttentionQueue";
import { QUEUE_WATCH_RUN } from "./queueBridge";
import "../../styles/operation-progress.css";
import "../../styles/attention-queue.css";

/** Opens and closes the queue from anywhere outside a phone's keyboard and a modal dialog. */
export const QUEUE_SHORTCUT = "Control+Shift+X";
const NOTICE_MS = 6000;
const EMPTY_PHONES: ReadonlyMap<string, QueuePhone> = new Map();
const EMPTY_ATTENTION: readonly DeviceAttention[] = [];
const FOCUSABLE = "a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), summary, [tabindex]:not([tabindex='-1'])";

function focusables(root: ParentNode): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)]
    .filter(element => !element.closest("[hidden]") && (element.checkVisibility?.() ?? true));
}

type QueueTab = "attention" | "running";

/**
 * The "Cần xử lý" queue and the run monitor (PLAN §5), replacing the "Theo dõi tác vụ" panel.
 *
 * In the system bar: a "Cần xử lý N" chip, and while something runs a pill with its progress.
 * Neither ever opens anything on its own: a new item only raises the count and shows a notice
 * that takes no layout space and no focus. The queue opens only from the chip, the pill or
 * Ctrl+Shift+X, as a drawer laid over the wall (never a column that narrows it); opening and
 * closing never touch the wall's selection, its order or the focused element, and closing gives
 * the whole wall back. Tab "Đang chạy" is the run monitor with Stop; a finished run leaves it
 * (Lượt chạy keeps it) unless the operator is watching it.
 *
 * Without a host (unit tests, startup) the bar controls fall back to `document.body`.
 */
export function OperationProgressCenter({ deviceLabels, phones = EMPTY_PHONES, attention = EMPTY_ATTENTION, attentionKnown = true, pillHost = null, onDeviceDetails, onOpenRuns }: {
  deviceLabels: ReadonlyMap<string, string>;
  /** Every phone's number, box and current holder. */
  phones?: ReadonlyMap<string, QueuePhone>;
  /** The phones the header counter "Cần xử lý" counts, as far as they are known. */
  attention?: readonly DeviceAttention[];
  /** False while that counter is unknown ("?"): the list may then miss phones. */
  attentionKnown?: boolean;
  pillHost?: HTMLElement | null;
  onDeviceDetails?: (udid: string) => void;
  onOpenRuns?: () => void;
}) {
  const [initialStart] = useState(() => {
    try { return { pending: readPendingPublishStart(), error: null as string | null }; }
    catch (error) { return { pending: null, error: describeError(error) }; }
  });
  const [pendingStart, setPendingStart] = useState(initialStart.pending);
  const [startReadError, setStartReadError] = useState(initialStart.error);
  const [cancellingStart, setCancellingStart] = useState(false);
  const cancelStartInFlight = useRef(false);
  const cancelUnacceptedStart = async () => {
    if (!pendingStart || cancelStartInFlight.current) return;
    const pending = pendingStart;
    cancelStartInFlight.current = true;
    setCancellingStart(true);
    try {
      const status = await publishCancelUnacceptedStart(pending.requestId);
      if (status.requestId !== pending.requestId) throw new Error("Phản hồi không khớp yêu cầu cần hủy; giữ bản ghi để đối chiếu.");
      if (status.state === "failed" && !status.campaignId) {
        retirePublishStart(pending.requestId);
        setStartReadError(status.error?.message ?? "Yêu cầu chưa bắt đầu đã được hủy. Kiểm tra lại để tạo lượt mới.");
      } else {
        acknowledgePublishStart(pending, status);
        setStartReadError("Yêu cầu đã được tiếp nhận hoặc có thể đã bắt đầu; không hủy mã. Theo dõi lượt hiện tại trước khi tạo lượt mới.");
      }
    } catch (error) { setStartReadError(describeError(error)); }
    finally { cancelStartInFlight.current = false; setCancellingStart(false); }
  };
  const [pinnedRun, setPinnedRun] = useState<OperationRunSummary | null>(null);
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<QueueTab>("attention");
  const [maximized, setMaximized] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  /** A finished run the operator is watching stays in "Đang chạy" until they let it go. */
  const [watchId, setWatchId] = useState<string | null>(null);
  const [stopResult,setStopResult]=useState<OperationStopResult|null>(null);
  const [stopError,setStopError]=useState<string|null>(null);
  const [stopping,setStopping]=useState(false);
  const selectionRef=useRef<string|undefined>(undefined);
  const stopTicket=useRef(0);
  const [dismissal, setDismissal] = useState(() => {
    try { return { records: readDismissedRecords(), error: null as string | null }; }
    catch (error) { return { records: [] as DismissedRecord[], error: describeError(error) }; }
  });
  const [undoKeys, setUndoKeys] = useState<string[]>([]);
  /** The notice beside the chip for items that arrived since the last look. */
  const [notice, setNotice] = useState<{ id: number; count: number; text: string } | null>(null);
  const chipRef = useRef<HTMLButtonElement>(null);
  const pillRef = useRef<HTMLButtonElement>(null);
  const openerRef = useRef<HTMLElement | null>(null);
  const drawerRef = useRef<HTMLElement>(null);
  const drawerId = useId();
  const bodyId = useId();
  const read = useCallback(async () => {
    const page = await operationQueryRuns({ limit: 200, offset: 0, since: new Date(Date.now() - 24 * 60 * 60_000).toISOString() });
    const sessions = page.runs.some((run) => run.kind === "nurture" && activeRun(run)) ? await nurtureSessionStatus() : [];
    return { page, sessions };
  }, []);
  const state = useMonitorRead(read, 2000, ["runs", "last24h", 200, 0]);
  const listedRuns = state.value?.page.runs ?? [];
  const allRuns = pinnedRun && !listedRuns.some(run => run.id === pinnedRun.id) ? [pinnedRun, ...listedRuns] : listedRuns;
  // "Đang chạy" holds running runs; a finished one stays only while it is watched.
  const runs = allRuns.filter(run => activeRun(run) || run.id === watchId);
  const selectedRun = pendingStart ? null : runs.find((run) => run.id === selectedId) ?? runs.find(run => run.id === pinnedRun?.id) ?? runs[0] ?? null;
  const queue = useAttentionQueue({
    runs: listedRuns, runsSettled: state.value !== null || state.error !== null, runsError: state.error,
    phones, attention, attentionKnown, labels: deviceLabels, dismissed: dismissal.records,
  });

  const openQueue = useCallback((next: QueueTab, opener: HTMLElement | null) => {
    openerRef.current = opener;
    setTab(next);
    setOpen(true);
    // The operator is looking at the queue now; the notice would only cover it.
    setNotice(null);
  }, []);
  const stopIssue = (id: string | null) => !!id && stopResult?.operationId === id && stopResult.state !== "closed";
  const close = () => {
    setOpen(false);
    setWatchId(current => {
      const run = allRuns.find(item => item.id === current);
      return run && (activeRun(run) || stopIssue(current)) ? current : null;
    });
    // Focus only returns when it was inside the drawer; a focused tile keeps it.
    if (drawerRef.current?.contains(document.activeElement)) (openerRef.current ?? chipRef.current)?.focus();
  };
  const openRef = useRef(open);
  useEffect(() => { openRef.current = open; });
  const latest = useRef({ open, close, openQueue });
  useEffect(() => { latest.current = { open, close, openQueue }; });
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.altKey || !event.shiftKey) return;
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "x") return;
      if (document.querySelector('[aria-modal="true"]')) return;
      event.preventDefault();
      // The shortcut never moves focus: the wall keeps its focused tile.
      if (latest.current.open) latest.current.close();
      else latest.current.openQueue("attention", null);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
  // Lượt chạy asks to show a run's phones: the operator pressed it, so the drawer opens on it.
  useEffect(() => {
    const watchRun = (event: Event) => {
      const run = (event as CustomEvent<OperationRunSummary>).detail;
      setPinnedRun(run); setWatchId(run.id); setSelectedId(run.id);
      latest.current.openQueue("running", null);
    };
    window.addEventListener(QUEUE_WATCH_RUN, watchRun);
    return () => window.removeEventListener(QUEUE_WATCH_RUN, watchRun);
  }, []);

  // The drawer starts under the system bar, whatever height the bar wrapped to.
  const [top, setTop] = useState(48);
  useLayoutEffect(() => {
    if (!open) return;
    const header = (pillHost ?? chipRef.current)?.closest("header");
    if (!header) return;
    const measure = () => setTop(Math.round(header.getBoundingClientRect().bottom));
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(header);
    window.addEventListener("resize", measure);
    return () => { observer?.disconnect(); window.removeEventListener("resize", measure); };
  }, [open, pillHost]);

  useEffect(() => {
    const receive = (event: Event) => {
      const next = (event as CustomEvent<PendingPublishStart>).detail;
      // Shown in the pill and in "Đang chạy"; the queue still never opens on its own.
      setPendingStart(next); setSelectedId(next.status?.operationId ?? null);
      setStartReadError(null);
    };
    // A durable pre-accept rejection retires the marker; the monitor must forget it too,
    // or it keeps reporting a start that the backend says never began.
    const retired = (event: Event) => {
      const requestId = (event as CustomEvent<string>).detail;
      setPendingStart(current => current?.requestId === requestId ? null : current);
      setStartReadError(null);
    };
    window.addEventListener(PUBLISH_START_ACKNOWLEDGED, receive);
    window.addEventListener(PUBLISH_START_PENDING, receive);
    window.addEventListener(PUBLISH_START_RETIRED, retired);
    return () => {
      window.removeEventListener(PUBLISH_START_ACKNOWLEDGED, receive);
      window.removeEventListener(PUBLISH_START_PENDING, receive);
      window.removeEventListener(PUBLISH_START_RETIRED, retired);
    };
  }, []);
  const pendingRequestId = pendingStart?.requestId;
  useEffect(() => {
    if (!pendingRequestId) return;
    let live = true, reading = false;
    let acknowledged = false;
    const readStart = async () => {
      if (reading) return;
      reading = true;
      try {
        const status = await publishStartStatus(pendingRequestId);
        if (!live || cancelStartInFlight.current) return;
        if (!status) { setStartReadError("Chưa tìm thấy xác nhận cho yêu cầu đã gửi. Trở lại Đăng bài để đối chiếu cùng lượt; không tạo lượt mới."); return; }
        if (status.requestId !== pendingRequestId) throw new Error("Phản hồi không khớp yêu cầu đang theo dõi.");
        if (status.state === "failed" && !status.campaignId) {
          // Durable pre-accept rejection: retire everywhere, keep the reason visible.
          retirePublishStart(pendingRequestId);
          setStartReadError(status.error?.message ?? "Lượt đăng bị từ chối trước khi bắt đầu; kiểm tra lại để tạo lượt mới.");
          return;
        }
        if (!acknowledged && status.state !== "failed") {
          acknowledged = true;
          const saved = readPendingPublishStart();
          if (saved?.requestId === pendingRequestId) acknowledgePublishStart(saved, status);
        }
        setStartReadError(null);
        setPendingStart(current => current?.requestId === pendingRequestId && (current.status?.revision ?? -1) <= status.revision ? { ...current, status } : current);
        if (status.campaignId) {
          const run = await operationGetRun(`publish:${status.campaignId}`);
          if (!live || !run) return;
          setPinnedRun(run.summary); setSelectedId(run.summary.id); setWatchId(run.summary.id); setPendingStart(null);
          retirePublishStart(pendingRequestId);
        }
      } catch (error) { if (live) setStartReadError(describeError(error)); }
      finally { reading = false; }
    };
    void readStart();
    const timer = window.setInterval(() => void readStart(), 1000);
    return () => { live = false; clearInterval(timer); };
  }, [pendingRequestId]);
  const resolvedSelectedId = selectedRun?.id;
  const selectedStop=stopResult?.operationId===resolvedSelectedId?stopResult:null;
  const canPausePublish=selectedRun?.kind==="publish" && selectedRun.state!=="succeeded"
    && !(selectedStop?.state==="closed" && selectedStop.stopMarker);
  selectionRef.current=resolvedSelectedId;
  useEffect(()=>{
    setStopResult(null);setStopError(null);
    if(!resolvedSelectedId)return;
    let live=true;
    let lastRead = 0;
    let reading = false;
    const read = async () => {
      if (reading) return;
      reading = true;
      const ticket = stopTicket.current;
      const readId = ++lastRead;
      try {
        const result = await operationStopStatus(resolvedSelectedId);
        // An absent or stale row cannot retract an acknowledged Stop. Only a
        // result for this operation may advance its cleanup status.
        if (live && ticket === stopTicket.current && readId === lastRead && result?.operationId === resolvedSelectedId) {
          setStopResult(result);
        } else if (live && ticket === stopTicket.current && readId === lastRead && result === null) {
          // A completed marker can be cleared by an explicit resume. A pending
          // acknowledgement still requires a real terminal cleanup result.
          setStopResult(current => current?.state === "stopping" ? current : null);
        }
      } catch (error) {
        if (live && ticket === stopTicket.current && readId === lastRead) setStopError(describeError(error));
      } finally {
        reading = false;
      }
    };
    void read();const timer=window.setInterval(()=>void read(),2000);
    return()=>{live=false;clearInterval(timer);};
  },[resolvedSelectedId]);
  const stopSelected=async()=>{
    if(!selectedRun||stopping)return;
    const id=selectedRun.id;++stopTicket.current;setStopping(true);setStopError(null);setWatchId(id);
    try{const result=await operationStop(id);if(selectionRef.current===id){setStopResult(result);state.retry();}}
    catch(error){if(selectionRef.current===id)setStopError(describeError(error));}
    finally{setStopping(false);}
  };
  useEffect(() => {
    // Pin the initial selection too: a newly updated run must not steal the open inspector.
    if (resolvedSelectedId && resolvedSelectedId !== selectedId) setSelectedId(resolvedSelectedId);
  }, [resolvedSelectedId, selectedId]);
  // The run on screen in "Đang chạy" is watched: it does not vanish when it finishes.
  useEffect(() => {
    if (open && tab === "running" && resolvedSelectedId) setWatchId(resolvedSelectedId);
  }, [open, tab, resolvedSelectedId]);
  // Lượt chạy is a page: the drawer closes so it does not cover what was asked for.
  const openRuns = onOpenRuns ? () => { close(); onOpenRuns(); } : undefined;
  const watch = (run: OperationRunSummary) => {
    setWatchId(run.id); setSelectedId(run.id); setTab("running");
  };
  const dismiss = (run: OperationRunSummary) => {
    try {
      const next = dismissMonitorRecords(dismissal.records, [run]);
      writeDismissedRecords(next);
      setDismissal({ records: next, error: null });
      setUndoKeys([monitorRecordKey(run)]);
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
  // First sight of each item (for rows the backend gives no time) and the notice for new ones.
  // The first complete read is the baseline, and so is the moment the header counter becomes
  // known: phones that were only unread are not news.
  const [seenAt, setSeenAt] = useState<ReadonlyMap<string, number>>(() => new Map());
  const items = useMemo(() => queue.groups.flatMap(group => group.items), [queue.groups]);
  const itemsRef = useRef(items);
  useEffect(() => { itemsRef.current = items; });
  const signature = items.map(item => item.key).join("\n");
  const noticedRef = useRef<{ keys: Set<string>; attentionKnown: boolean } | null>(null);
  useEffect(() => {
    const keys = signature ? signature.split("\n") : [];
    setSeenAt(previous => {
      const now = Date.now();
      return new Map(keys.map(key => [key, previous.get(key) ?? now]));
    });
    if (!queue.settled) return;
    const previous = noticedRef.current;
    noticedRef.current = { keys: new Set(keys), attentionKnown };
    if (!previous) return;
    const fresh = keys.filter(key => !previous.keys.has(key) && (previous.attentionKnown || !key.startsWith("device:")));
    if (!fresh.length || openRef.current) return;
    const names = itemsRef.current.filter(item => fresh.includes(item.key)).map(item => `${item.machine} · ${item.reason}`);
    setNotice({ id: Date.now(), count: fresh.length, text: names.slice(0, 3).join("; ") + (names.length > 3 ? `; và ${names.length - 3} mục khác` : "") });
  }, [signature, queue.settled, attentionKnown]);
  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(current => current?.id === notice.id ? null : current), NOTICE_MS);
    return () => window.clearTimeout(timer);
  }, [notice]);
  const [noticeAt, setNoticeAt] = useState<{ top: number; right: number } | null>(null);
  useLayoutEffect(() => {
    if (!notice) return;
    const rect = chipRef.current?.getBoundingClientRect();
    setNoticeAt(rect ? { top: Math.round(rect.bottom + 6), right: Math.max(8, Math.round(window.innerWidth - rect.right)) } : null);
  }, [notice]);

  const active = allRuns.filter(activeRun);
  const progress = active.map((run) => runProgress(run, state.value?.sessions ?? []));
  const fraction = !pendingStart && progress.length && progress.every((value) => value !== null)
    ? progress.reduce((sum, value) => sum + value!, 0) / progress.length : null;
  const runningCount = active.length + (pendingStart ? 1 : 0);
  const showPill = !!pendingStart || !!startReadError || active.length > 0 || !!state.error;
  const pillLabel = state.error ? "Chưa đọc được tiến trình" : pendingStart ? "Lượt đăng đang chuẩn bị"
    : active.length ? `${active.length} tác vụ đang xử lý / chờ` : "Lượt đăng cần kiểm tra";
  const pillAttention = !!startReadError || !!state.error;
  // "≥5": some source is unread, so the count is a floor and never claims there is nothing more.
  const countText = queue.unknown ? `≥${queue.total}` : String(queue.total);
  const chipLabel = queue.unknown
    ? `Hàng đợi Cần xử lý: ít nhất ${queue.total} mục; một số nguồn chưa đọc được`
    : `Hàng đợi Cần xử lý: ${queue.total} mục`;
  const deviceCount = queue.groups.find(group => group.key === "devices")?.items.length ?? 0;
  const chipTitle = `${chipLabel}. ${deviceCount} máy cần xử lý (cùng bộ đếm Cần xử lý), ${queue.total - deviceCount} mục từ lượt chạy: bài Chưa rõ, lỗi ghi Sheet, máy lỗi. Bấm hoặc Ctrl+Shift+X để mở; hàng đợi không tự mở.`;

  // Tab moves from the bar controls into the open drawer and back, as if it sat beside them.
  const enterDrawer = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (!open || event.key !== "Tab" || event.shiftKey || !drawerRef.current) return;
    const first = focusables(drawerRef.current)[0];
    if (!first) return;
    event.preventDefault();
    openerRef.current = event.currentTarget;
    first.focus();
  };
  const leaveDrawer = (event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape") {
      event.stopPropagation();
      close();
      return;
    }
    if (event.key !== "Tab" || !drawerRef.current) return;
    const inside = focusables(drawerRef.current);
    if (event.shiftKey && document.activeElement === inside[0]) {
      event.preventDefault();
      (openerRef.current ?? chipRef.current)?.focus();
    } else if (!event.shiftKey && document.activeElement === inside.at(-1)) {
      const launchers = [chipRef.current, pillRef.current].filter((element): element is HTMLButtonElement => !!element);
      const outside = focusables(document).filter(element => !drawerRef.current?.contains(element));
      const after = outside[outside.indexOf(launchers.at(-1) as HTMLElement) + 1];
      if (after) { event.preventDefault(); after.focus(); }
    }
  };

  const bar = <span className="queue-launchers" role="group" aria-label="Hàng đợi và tiến trình">
    <button ref={chipRef} type="button" className={`queue-chip${queue.total > 0 ? " has-items" : ""}${queue.unknown ? " is-unknown" : ""}`}
      aria-label={chipLabel} title={chipTitle} aria-expanded={open && tab === "attention"} aria-controls={drawerId} aria-keyshortcuts={QUEUE_SHORTCUT}
      onKeyDown={enterDrawer}
      onClick={(event) => open && tab === "attention" ? close() : openQueue("attention", event.currentTarget)}>
      {queue.total > 0 ? <TriangleAlert size={14} aria-hidden="true" /> : <Inbox size={14} aria-hidden="true" />}
      <span className="queue-chip-label">Cần xử lý</span>
      <strong className="queue-chip-count">{countText}</strong>
    </button>
    {showPill && <span className={`queue-run-pill${pillAttention ? " has-attention" : ""}`}>
      <button ref={pillRef} type="button" className="queue-run-toggle" aria-label="Tiến trình công việc" title={`${pillLabel} · ${state.error ? "?" : progressLabel(fraction)}`}
        aria-expanded={open && tab === "running"} aria-controls={drawerId} onKeyDown={enterDrawer}
        onClick={(event) => open && tab === "running" ? close() : openQueue("running", event.currentTarget)}>
        <Clock3 size={14} aria-hidden="true" />
        <strong className="queue-run-count">{state.error ? "?" : runningCount}</strong>
        <span className="queue-run-label">{pendingStart && !active.length ? "đang chuẩn bị" : "đang chạy"}</span>
        <span className="run-percent">{state.error ? "?" : progressLabel(fraction)}</span>
        {pillAttention && <TriangleAlert className="run-attention" size={13} aria-label="Có kết quả cần kiểm tra" />}
      </button>
      <ProgressBar fraction={state.error ? null : fraction} label="Tiến độ các lượt đang chạy" tone={active.length ? "run" : "idle"} className="queue-run-bar" />
    </span>}
  </span>;

  const drawer = <aside ref={drawerRef} id={drawerId} hidden={!open} style={{ top }}
    className={`queue-drawer${maximized ? " is-maximized" : ""}`} aria-label="Hàng đợi Cần xử lý" onKeyDown={leaveDrawer}>
    <header className="queue-drawer-head">
      <WorkspaceTabs label="Hàng đợi" value={tab} onChange={(next) => setTab(next as QueueTab)} tabs={[
        { id: "attention", label: `Cần xử lý (${countText})`, panelId: `${drawerId}-attention` },
        { id: "running", label: `Đang chạy (${runningCount})`, panelId: `${drawerId}-running` },
      ]} />
      <button type="button" className="icon-btn" aria-label={maximized ? "Khôi phục kích thước" : "Phóng rộng tiến trình"}
        title={maximized ? "Khôi phục kích thước" : "Phóng rộng tiến trình"} onClick={() => setMaximized((value) => !value)}>
        {maximized ? <Minimize2 size={16} /> : <Maximize2 size={16} />}
      </button>
      <button type="button" className="icon-btn" aria-label="Đóng hàng đợi" title="Đóng hàng đợi (Esc)" onClick={close}><X size={16} /></button>
    </header>
    {/* Rendered only while shown: a closed queue leaves no second "Máy N" in the page. */}
    {open && tab === "attention" && (
      <div id={`${drawerId}-attention`} role="tabpanel" aria-label="Cần xử lý" className="queue-panel">
        <AttentionQueueList groups={queue.groups} total={queue.total} unknown={queue.unknown} omitted={queue.omitted}
          runsError={state.error} retryRuns={state.retry} attentionUnknown={!attentionKnown}
          seenAt={(key) => seenAt.get(key) ?? null} onWatch={watch} onDeviceDetails={onDeviceDetails}
          onDismiss={dismiss} undoCount={undoKeys.length} onUndo={undo} dismissError={dismissal.error} onOpenRuns={openRuns} />
      </div>
    )}
    <Activity mode={open && tab === "running" ? "visible" : "hidden"}>
      <div id={`${drawerId}-running`} role="tabpanel" aria-label="Đang chạy"
        className={`run-monitor queue-panel ${maximized ? "is-maximized" : "is-compact"}`}>
        <div className="run-monitor-body" id={bodyId}>
          {/* Several runs at once: each one's progress side by side, a press shows its phones. */}
          {active.length > 1 && <ul className="queue-run-list" aria-label="Lượt đang chạy">
            {active.map((run, index) => <li key={run.id}>
              <button type="button" aria-pressed={!pendingStart && run.id === selectedRun?.id} disabled={!!pendingStart}
                onClick={() => { setSelectedId(run.id); setWatchId(run.id); }}>
                <span>{runOptionLabel(run)}</span>
                <span className="run-percent">{progressLabel(progress[index])}</span>
              </button>
              <ProgressBar fraction={progress[index]} label={`Tiến độ ${runOptionLabel(run)}`} />
            </li>)}
          </ul>}
          <div className="run-monitor-tools">
            <label className="run-selector-label" htmlFor={`${bodyId}-run`}>Phiên chạy</label>
            <select id={`${bodyId}-run`} aria-label="Chọn tác vụ theo dõi" title={selectedRun ? runOptionLabel(selectedRun) : undefined} value={pendingStart ? pendingStart.status?.operationId ?? pendingStart.requestId : selectedRun?.id ?? ""} onChange={(event) => { setSelectedId(event.target.value); setWatchId(event.target.value); }} disabled={!!pendingStart || !runs.length}>
              {pendingStart && <option value={pendingStart.status?.operationId ?? pendingStart.requestId}>Lượt đăng đang bắt đầu</option>}
              {!runs.length && !pendingStart && <option value="">Không có lượt đang chạy</option>}
              {runs.map((run) => <option key={run.id} value={run.id}>{activeRun(run) ? runOptionLabel(run) : `${runOptionLabel(run)} · đã kết thúc`}</option>)}
            </select>
            {selectedRun&&(activeRun(selectedRun)||canPausePublish||selectedStop?.state==="needsAttention"||selectedStop?.state==="failed")&&<button type="button" className="run-monitor-stop" title="Dừng toàn bộ máy của tác vụ đang chọn và đóng TikTok" aria-label="Dừng tác vụ và về màn hình chính" disabled={stopping||selectedStop?.state==="stopping"||!!state.error} onClick={()=>void stopSelected()}><Square size={14}/>{stopping||selectedStop?.state==="stopping"?"Đang dừng và nhả máy…":selectedRun.kind==="publish"?"Tạm dừng":"Dừng"}</button>}
          </div>
          {pendingStart && <div className="run-start-bridge" role="status">
            <strong>{pendingStart.status?.state === "failed" ? "Không thể bắt đầu" : pendingStart.status?.state === "uncertain" ? "Đang đối chiếu yêu cầu" : "Đang chuẩn bị lượt đăng"}</strong>
            <span>{pendingStart.status ? publishStageLabel(pendingStart.status.stage) : "Chờ xác nhận từ ứng dụng"}</span>
            <small>Tác vụ: {pendingStart.status?.operationId ?? `publish-start:${pendingStart.requestId}`}</small>
            {pendingStart.status?.error && <p role="alert">{pendingStart.status.error.message}</p>}
            {!pendingStart.status && <div>
              <p>Chỉ hủy mã nếu ứng dụng chưa tiếp nhận yêu cầu. Nếu lượt đã bắt đầu, ứng dụng giữ nguyên để đối chiếu. Mã đã hủy không thể dùng lại.</p>
              <button type="button" disabled={cancellingStart} onClick={() => void cancelUnacceptedStart()}>
                {cancellingStart ? "Đang đối chiếu và hủy…" : "Hủy yêu cầu chưa được tiếp nhận"}
              </button>
            </div>}
          </div>}
          {startReadError && <p role="alert" className="run-stop-feedback is-error">{startReadError}</p>}
          {stopError&&<p className="run-stop-feedback is-error" role="alert">{stopError}</p>}
          {stopResult&&stopResult.operationId===selectedRun?.id&&<div className="run-stop-feedback" role="status">
            <strong>{stopResult.state==="stopping"?"Đang dừng và nhả máy…":stopResult.state==="closed"?`Đã dừng · TikTok đã tắt trên ${stopResult.devices.length} máy`:"Đã yêu cầu dừng · Có máy cần kiểm tra"}</strong>
            {stopResult.state!=="stopping"&&stopResult.devices.filter(d=>!d.closed).map(d=><span key={d.udid}>{deviceLabels.get(d.udid)??d.udid}: {d.message}</span>)}
          </div>}
          {state.error ? <MonitorReadError message={state.error} retry={state.retry} /> : selectedRun
            ? <OperationRunDevices key={selectedRun.id} run={selectedRun} labels={deviceLabels} sessions={state.value?.sessions ?? []} compact={!maximized} />
            : pendingStart ? null : <p className="run-monitor-empty">Không có lượt đang chạy. Lượt đã xong nằm trong Lượt chạy.</p>}
          <p className="run-monitor-history-note">
            Chỉ lượt đang chạy và lượt bạn đang theo dõi. Lượt đã xong nằm trong Lượt chạy.
            {openRuns && <> <button type="button" className="ghost" onClick={openRuns}>Mở Lượt chạy</button></>}
          </p>
        </div>
      </div>
    </Activity>
  </aside>;

  return <>
    {createPortal(bar, pillHost ?? document.body)}
    {createPortal(drawer, document.body)}
    {createPortal(<>
      {/* Read once by assistive technology; never takes focus. */}
      <span className="visually-hidden" role="status" aria-live="polite">{notice ? `Cần xử lý: thêm ${notice.count} mục. ${notice.text}` : ""}</span>
      {notice && noticeAt && <div className="queue-notice" aria-hidden="true" style={{ top: noticeAt.top, right: noticeAt.right }}>
        <TriangleAlert size={14} />
        <span><strong>Thêm {notice.count} mục cần xử lý</strong><small>{notice.text}</small></span>
      </div>}
    </>, document.body)}
  </>;
}
