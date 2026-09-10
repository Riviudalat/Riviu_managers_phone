import { useEffect, useMemo, useRef, useState } from "react";
import { CalendarClock, GripVertical, Search, Undo2, Unlink, X, Zap } from "lucide-react";
import { publishScheduleCreate, publishSchedulePreflight } from "../../api";
import { describeError } from "../../describeError";
import { orderDevicesByNumber, tileNumber, tileName } from "../../deviceNaming";
import type { DeviceInfo, DeviceMeta, PublishBundle, PublishScheduleReport, PublishScheduleRequest, PublishSoundPolicy } from "../../types";
import { PublishMedia } from "./PublishMedia";
import { localDateTime, scheduleDateIssue } from "./publishScheduleTimes";
import { allocateScheduleRows, decodeScheduleDraft, machineHasScheduleConflict, scheduleTime, SCHEDULE_DRAFT_KEY, type ScheduleDraft, type ScheduleRow } from "./publishScheduleAllocation";
import { useScheduleDrag, type ScheduleDropTarget } from "./useScheduleDrag";
import "../../styles/publish-schedule.css";

type Props = {
  sourceRoot: string; bundles: PublishBundle[]; devices: DeviceInfo[]; metas: Map<string, DeviceMeta>;
  captions: Record<string, string>; sound: PublishSoundPolicy; sheet: boolean; cleanup: boolean;
  selectedIds?: string[]; assignments?: Record<string, string>; eligible?: string[];
  active?: boolean; sourceReady?: boolean; blockingReason?: string;
  onCreated: () => void; onSource: () => void; onHistory?: () => void; onSheetSetup?: () => void;
};
const emptyDraft = (sourceRoot: string): ScheduleDraft => ({ version: 2, sourceRoot, date: localDateTime(new Date()).slice(0, 10), commonTime: "", rows: [], selectedMachines: [], requestId: crypto.randomUUID() });
const newRow = (bundleId: string): ScheduleRow => ({ id: crypto.randomUUID(), bundleId, udid: "", time: "", timeMode: "common" });

export function PublishSchedulePlanner(p: Props) {
  const [draft, setDraft] = useState(() => emptyDraft(p.sourceRoot));
  const draftRef = useRef(draft);
  const initialized = useRef(false);
  const mounted = useRef(true);
  const [postQuery, setPostQuery] = useState("");
  const [machineQuery, setMachineQuery] = useState("");
  const [individualTimes, setIndividualTimes] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [storageError, setStorageError] = useState<string | null>(null);
  const [hoveredPost, setHoveredPost] = useState<string | null>(null);
  const [focusedPost, setFocusedPost] = useState<string | null>(null);
  const activePost = hoveredPost ?? focusedPost;
  const [focusTarget, setFocusTarget] = useState<{ field: "posts" | "machines" | "date" | "commonTime" | "rowMachine" | "rowTime" | "verdict" | "check" | "confirm"; bundleId?: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);
  const generation = useRef(0);
  const [history, setHistory] = useState<ScheduleDraft[]>([]);
  const [review, setReview] = useState<{ key: string; generation: number; request: PublishScheduleRequest; report: PublishScheduleReport } | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (p.active === false) return;
    const refresh = () => setNow(Date.now());
    const timer = setInterval(refresh, 1000);
    window.addEventListener("focus", refresh);
    return () => { clearInterval(timer); window.removeEventListener("focus", refresh); };
  }, [p.active]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; generation.current += 1; }; }, []);
  const ordered = useMemo(() => orderDevicesByNumber(p.devices, p.metas), [p.devices, p.metas]);
  const inScope = ordered.filter(d => !p.eligible || p.eligible.includes(d.udid));
  const readyIds = inScope.filter(d => d.status === "ready").map(d => d.udid);
  const label = (udid: string) => {
    const index = ordered.findIndex(d => d.udid === udid);
    return index >= 0 ? `Máy ${tileNumber(index + 1, p.metas.get(udid))}` : "Máy đã ngắt kết nối";
  };
  const name = (id: string) => p.bundles.find(b => b.id === id)?.name ?? "Bài không còn trong nguồn";
  const persist = (next: ScheduleDraft) => {
    try { localStorage.setItem(SCHEDULE_DRAFT_KEY, JSON.stringify(next)); setStorageError(null); }
    catch { setStorageError("Chưa lưu được bản nháp lịch. Giữ trang này mở để tiếp tục."); }
  };
  // Delay seeding until the tab is first opened and source restoration has completed.
  useEffect(() => {
    if (initialized.current || p.active === false || p.sourceReady === false) return;
    let saved: ScheduleDraft | null = null;
    try { saved = decodeScheduleDraft(localStorage.getItem(SCHEDULE_DRAFT_KEY), p.sourceRoot); } catch { /* Start with the selected source if storage is unreadable. */ }
    const next = saved ?? { ...emptyDraft(p.sourceRoot), rows: p.bundles.filter(b => p.selectedIds?.includes(b.id)).slice(0, 100).map(b => ({ ...newRow(b.id), udid: p.assignments?.[b.id] ?? "" })) };
    if (!saved) next.selectedMachines = [...new Set(next.rows.map(r => r.udid).filter(Boolean))];
    initialized.current = !!saved || next.rows.length > 0;
    draftRef.current = next; setDraft(next);
    setIndividualTimes(next.rows.some(r => r.timeMode === "custom"));
    if (!saved && next.rows.length) persist(next);
  }, [p.active, p.sourceReady, p.sourceRoot, p.bundles, p.selectedIds, p.assignments]);
  const edit = (change: (current: ScheduleDraft) => ScheduleDraft, remember = true) => {
    if (inFlight.current) return;
    const before = draftRef.current;
    const value = change(before);
    if (JSON.stringify(value) === JSON.stringify(before)) return;
    const next = { ...value, requestId: crypto.randomUUID() };
    initialized.current = true;
    if (remember) setHistory(h => [...h.slice(-29), before]);
    generation.current += 1;
    draftRef.current = next; setDraft(next); setReview(null); setConfirmed(false); setNotice(null); persist(next);
  };
  const request: PublishScheduleRequest = { requestId: draft.requestId, sourceRoot: p.sourceRoot,
    slots: draft.rows.map(r => ({ bundleId: r.bundleId, udid: r.udid, runAt: `${draft.date}T${scheduleTime(r, draft.commonTime)}` })),
    captionOverrides: Object.fromEntries(draft.rows.filter(r => p.captions[r.bundleId] !== undefined).map(r => [r.bundleId, p.captions[r.bundleId]])),
    soundPolicy: p.sound, sheetEnabled: p.sheet, deleteAfterPublish: p.cleanup };
  const key = JSON.stringify({ request,
    readiness: draft.rows.map(row => [row.udid, readyIds.includes(row.udid)]),
    source: draft.rows.map(row => p.bundles.find(bundle => bundle.id === row.bundleId) ?? null),
    sourceReady: p.sourceReady !== false, blockingReason: p.blockingReason ?? "" });
  const latest = useRef(key);
  if (latest.current !== key) { latest.current = key; generation.current += 1; }
  const reviewed = review?.key === key && review.generation === generation.current && !p.blockingReason ? review : null;
  const locked = busy || p.active === false || p.sourceReady === false;
  const chosen = p.bundles.filter(b => draft.rows.some(r => r.bundleId === b.id));
  const addRows = (current: ScheduleDraft, ids: string[]) => {
    const missing = p.bundles.filter(b => ids.includes(b.id) && !current.rows.some(r => r.bundleId === b.id));
    const added = missing.slice(0, Math.max(0, 100 - current.rows.length)).map(b => newRow(b.id));
    return { ...current, rows: [...current.rows, ...added] };
  };
  const selection = (id: string, checked: boolean) => edit(d => checked ? addRows(d, [id]) : { ...d, rows: d.rows.filter(r => r.bundleId !== id) });
  const orderedIds = (ids: string[]) => p.bundles.filter(b => ids.includes(b.id)).map(b => b.id);
  const allocation = (ids: string[], target: ScheduleDropTarget) => {
    const before = draftRef.current;
    const next = addRows(before, ids);
    const singleTarget = ids.length === 1 ? target.machine : undefined;
    const candidates = singleTarget ? readyIds.filter(id => id === singleTarget) : readyIds.filter(id => next.selectedMachines.includes(id));
    const result = allocateScheduleRows(next.rows, orderedIds(ids), candidates, next.commonTime, singleTarget);
    return { ...result, next: { ...next, rows: result.rows, selectedMachines: singleTarget && result.assigned.length ? [...new Set([...next.selectedMachines, singleTarget])] : next.selectedMachines } };
  };
  const drop = (ids: string[], target: ScheduleDropTarget) => {
    if (locked || inFlight.current) return;
    const result = allocation(ids, target);
    edit(() => result.next);
    const overLimit = ids.filter(id => !result.rows.some(r => r.bundleId === id)).length;
    setNotice(overLimit ? `Mỗi lịch tối đa 100 bài. Còn ${overLimit} bài chưa được thêm.`
      : result.missing.length && target.machine ? "Máy này chưa sẵn sàng hoặc đã nhận bài cùng giờ. Phân công cũ được giữ nguyên."
      : result.missing.length ? `Đã gán ${result.assigned.length} bài. Còn ${result.missing.length} bài chưa có máy trống cùng giờ.`
      : result.assigned.length ? `Đã gán ${result.assigned.length} bài vào máy.` : "Các bài đã có máy; giữ nguyên phân công hiện tại.");
  };
  const quickAssign = () => {
    if (locked || inFlight.current) return;
    const before = draftRef.current;
    const ids = before.rows.length ? orderedIds(before.rows.map(r => r.bundleId)) : p.bundles.slice(0, 100).map(b => b.id);
    const next = addRows(before, ids);
    const candidates = before.selectedMachines.length ? readyIds.filter(id => before.selectedMachines.includes(id)) : readyIds;
    const result = allocateScheduleRows(next.rows, ids, candidates, next.commonTime);
    const rows = result.rows.filter(r => r.udid && readyIds.includes(r.udid));
    const assignedMachines = [...new Set(rows.map(r => r.udid))];
    edit(() => ({ ...next, rows, selectedMachines: rows.length ? assignedMachines : before.selectedMachines }));
    setNotice(rows.length ? `Đã gán ${rows.length}/${rows.length} bài · ${p.bundles.length > rows.length ? `${p.bundles.length - rows.length} bài còn lại để đợt sau. Hoàn tác để khôi phục lựa chọn trước.` : "mỗi bài đã có máy"}` : "Chưa có máy sẵn sàng trong lựa chọn. Chọn máy trước khi gán nhanh.");
  };
  const drag = useScheduleDrag({ disabled: locked, getIds: id => draftRef.current.rows.some(r => r.bundleId === id) ? orderedIds(draftRef.current.rows.map(r => r.bundleId)) : [id], onDrop: drop });
  useEffect(() => {
    if (!focusTarget) return;
    const root = drag.root.current;
    const isRowTarget = focusTarget.field === "rowMachine" || focusTarget.field === "rowTime" || focusTarget.field === "verdict";
    const row = isRowTarget && focusTarget.bundleId ? Array.from(root?.querySelectorAll<HTMLElement>("[data-schedule-row]") ?? []).find(r => r.dataset.scheduleRow === focusTarget.bundleId) : undefined;
    const target = (row ?? root)?.querySelector<HTMLElement>(`[data-schedule-field="${focusTarget.field}"]`);
    if (target) { target.scrollIntoView?.({ block: "nearest", inline: "nearest" }); target.focus({ preventScroll: true }); }
    setFocusTarget(null);
  }, [focusTarget, drag.root]);
  const preview = drag.view?.target ? allocation(drag.view.ids, drag.view.target) : null;
  const setMachine = (row: ScheduleRow, udid: string) => {
    if (udid && (!readyIds.includes(udid) || machineHasScheduleConflict(draftRef.current.rows, row.bundleId, udid, draftRef.current.commonTime))) {
      setNotice("Máy này chưa sẵn sàng hoặc đã nhận bài khác cùng giờ. Chọn máy trống hay đổi giờ bài."); return;
    }
    edit(d => ({ ...d, rows: d.rows.map(r => r.id === row.id ? { ...r, udid } : r), selectedMachines: udid ? [...new Set([...d.selectedMachines, udid])] : d.selectedMachines }));
  };
  const rowIssue = (row: ScheduleRow) => {
    if (!p.bundles.some(b => b.id === row.bundleId)) return "Bài không còn trong nguồn";
    if (!row.udid) return "Chưa có máy";
    if (!readyIds.includes(row.udid)) return "Máy chưa sẵn sàng trong phạm vi";
    if (machineHasScheduleConflict(draft.rows, row.bundleId, row.udid, draft.commonTime)) return "Trùng máy và giờ";
    const time = scheduleTime(row, draft.commonTime);
    return scheduleDateIssue(draft.date, time, now);
  };
  const valid = draft.rows.length > 0 && draft.rows.every(r => !rowIssue(r));
  const check = async () => {
    if (inFlight.current || locked || p.blockingReason || !valid) return;
    inFlight.current = true; setBusy(true); setReview(null); setConfirmed(false); setNotice(null);
    const at = key, revision = generation.current;
    try {
      const report = await publishSchedulePreflight(request);
      if (mounted.current && latest.current === at && generation.current === revision) {
        if (report.slots.length !== request.slots.length) throw new Error("Kết quả kiểm tra không khớp số bài. Kiểm tra lại lịch.");
        setReview({ key: at, generation: revision, request, report });
      }
    } catch (e) { if (mounted.current && latest.current === at && generation.current === revision) setNotice(describeError(e)); }
    finally { inFlight.current = false; if (mounted.current) setBusy(false); }
  };
  const save = async () => {
    if (inFlight.current || locked || !valid || !confirmed || !reviewed?.report.canExecute || p.blockingReason) return;
    if (draft.rows.some(row => scheduleDateIssue(draft.date, scheduleTime(row, draft.commonTime), Date.now()))) {
      setNow(Date.now()); setConfirmed(false); setReview(null); setNotice("Giờ đã qua. Chọn giờ mới rồi kiểm tra lại lịch."); return;
    }
    inFlight.current = true; setBusy(true); setNotice(null);
    try {
      const records = await publishScheduleCreate(reviewed.request, reviewed.report.inputDigest, true);
      if (records.length !== reviewed.request.slots.length) throw new Error("Kết quả lưu chưa khớp số bài. Giữ nguyên lịch và thử lại để đối chiếu, không tạo lịch mới.");
      p.onCreated();
      if (mounted.current) {
        const next = { ...draftRef.current, rows: [], requestId: crypto.randomUUID() };
        generation.current += 1; draftRef.current = next; setDraft(next); setReview(null); setConfirmed(false); setHistory([]); persist(next);
        setNotice(`Đã lưu lịch ${records.length} bài. Xem từng lượt trong Theo dõi.`);
      }
    } catch (e) { if (mounted.current) setNotice(describeError(e)); }
    finally { inFlight.current = false; if (mounted.current) setBusy(false); }
  };
  const missing = draft.rows.filter(r => !r.udid || !readyIds.includes(r.udid)).length;
  const assignmentsCount = draft.rows.length - missing;
  const rowStatus = (row: ScheduleRow, index: number) => {
    const issue = rowIssue(row);
    if (issue === "Chọn ngày và giờ đăng") return "Chờ đặt giờ";
    return issue || (reviewed ? reviewed.report.slots[index].canExecute ? "Sẵn sàng" : reviewed.report.slots[index].issues.map(i => i.message).join("; ") || "Cần kiểm tra lại" : "Chưa kiểm tra");
  };
  const filteredMachines = inScope.filter(d => `${label(d.udid)} ${tileName(d, p.metas.get(d.udid))}`.toLocaleLowerCase().includes(machineQuery.toLocaleLowerCase()));
  type NextStep = { text: string; action: string; target?: NonNullable<typeof focusTarget>; sheet?: boolean; source?: boolean };
  const nextStep = (): NextStep | null => {
    if (!draft.rows.length) return p.bundles.length ? { text: "Chọn bài hoặc bấm Chọn nhanh để phân công", action: "Chọn bài", target: { field: "posts" } } : { text: "Quét thư mục bài trước khi lập lịch", action: "Chọn nguồn bài", source: true };
    const sourceMissing = draft.rows.find(r => !p.bundles.some(b => b.id === r.bundleId));
    if (sourceMissing) return { text: `${name(sourceMissing.bundleId)}: kiểm tra lại thư mục`, action: "Kiểm tra nguồn", source: true };
    const missingMachine = draft.rows.find(r => !r.udid || !readyIds.includes(r.udid));
    if (missingMachine) return { text: `${missing} bài chưa có máy sẵn sàng`, action: "Bổ sung máy", target: { field: "rowMachine", bundleId: missingMachine.bundleId } };
    const invalid = draft.rows.find(r => !!rowIssue(r));
    if (invalid) {
      const issue = rowIssue(invalid);
      if (issue === "Trùng máy và giờ") return { text: `${name(invalid.bundleId)}: trùng máy và giờ`, action: "Đổi máy", target: { field: "rowMachine", bundleId: invalid.bundleId } };
      const badDate = !/^\d{4}-\d{2}-\d{2}$/.test(draft.date) || draft.date < localDateTime(new Date()).slice(0, 10);
      return { text: issue, action: "Đặt giờ", target: { field: badDate ? "date" : invalid.timeMode === "custom" ? "rowTime" : "commonTime", bundleId: invalid.bundleId } };
    }
    if (p.blockingReason) return { text: "Xác minh kết nối Sheet trước khi lưu lịch", action: "Kiểm tra Sheet", sheet: true };
    if (reviewed && !reviewed.report.canExecute) {
      const index = reviewed.report.slots.findIndex(s => !s.canExecute);
      return { text: "Có bài chưa đạt kiểm tra", action: "Xem lỗi", target: { field: "verdict", bundleId: draft.rows[Math.max(0, index)]?.bundleId } };
    }
    if (!reviewed) return { text: "Kiểm tra lịch trước khi xác nhận", action: "Tới kiểm tra", target: { field: "check" } };
    if (!confirmed) return { text: "Xác nhận các bài và giờ đã kiểm tra", action: "Tới xác nhận", target: { field: "confirm" } };
    return null;
  };
  const next = nextStep();
  const goToNext = () => {
    if (!next || locked) return;
    if (next.source) { p.onSource(); return; }
    if (next.sheet) { (p.onSheetSetup ?? p.onSource)(); return; }
    if (next.target?.field === "rowTime") setIndividualTimes(true);
    if (next.target) setFocusTarget(next.target);
  };
  return <section ref={drag.root} {...drag.bindings} className="publish-daily-schedule ps-workspace" aria-label="Lịch đăng nhiều khung giờ">
    <header className="ps-heading"><h2><CalendarClock size={18} aria-hidden="true"/> Lịch đăng</h2>{p.onHistory && <button type="button" disabled={busy} onClick={p.onHistory}>Theo dõi lịch đã lưu</button>}</header>
    <div className="ps-time-controls"><label>Ngày đăng<input data-schedule-field="date" type="date" aria-label="Ngày đăng" min={localDateTime(new Date(now)).slice(0, 10)} value={draft.date} disabled={locked} onChange={e => edit(d => ({ ...d, date: e.target.value }))}/></label><label>Giờ chung<input data-schedule-field="commonTime" type="time" aria-label="Giờ chung" value={draft.commonTime} disabled={locked} onChange={e => edit(d => ({ ...d, commonTime: e.target.value }))}/></label><label className="ps-inline-check"><input type="checkbox" checked={individualTimes} disabled={locked} onChange={e => setIndividualTimes(e.target.checked)}/>Chỉnh giờ từng bài</label><button type="button" disabled={locked || !history.length} onClick={() => { const previous = history.at(-1)!; setHistory(h => h.slice(0, -1)); edit(() => previous, false); }}> <Undo2 size={15} aria-hidden="true"/>Hoàn tác</button></div>
    {storageError && <div className="ps-storage-error" role="alert"><span>{storageError}</span><button type="button" disabled={locked} onClick={() => persist(draftRef.current)}>Thử lưu nháp lại</button></div>}
    <div className="ps-content" data-schedule-scroll>
      <div className="ps-assignment-board">
        <section className="ps-posts" aria-label="Bài để hẹn giờ">
          <header><h3>Bài đăng</h3><span>{p.bundles.length} bài</span></header>
          <label className="ps-search"><Search size={15} aria-hidden="true"/><input aria-label="Tìm bài hẹn giờ" placeholder="Tìm bài đăng" value={postQuery} onChange={e => setPostQuery(e.target.value)}/></label>
          <div className="ps-selection-tools"><button data-schedule-field="posts" type="button" disabled={locked || !p.bundles.length} onClick={() => { edit(d => addRows(d, p.bundles.map(b => b.id))); if (p.bundles.length > 100) setNotice("Đã chọn 100 bài đầu tiên; mỗi lịch tối đa 100 bài."); }}>Chọn tất cả bài</button><button type="button" disabled={locked || !draft.rows.length} onClick={() => edit(d => ({ ...d, rows: [] }))}>Bỏ chọn bài</button><span>{draft.rows.length} đã chọn</span></div>
          <div className="ps-post-list" data-schedule-scroll>{p.bundles.filter(b => `${b.name} ${p.captions[b.id] ?? b.caption}`.toLocaleLowerCase().includes(postQuery.toLocaleLowerCase())).map(b => {
            const row = draft.rows.find(r => r.bundleId === b.id);
            return <div key={b.id} data-schedule-post={b.id} className={`ps-post ${row ? "is-picked" : ""} ${activePost === b.id ? "is-linked" : ""}`} onMouseEnter={() => setHoveredPost(b.id)} onMouseLeave={() => setHoveredPost(null)} onFocusCapture={() => setFocusedPost(b.id)} onBlurCapture={e => { if (!e.currentTarget.contains(e.relatedTarget)) setFocusedPost(null); }}>
              <input type="checkbox" aria-label={`Chọn bài ${b.name}`} checked={!!row} disabled={locked || (!row && draft.rows.length >= 100)} onChange={e => selection(b.id, e.target.checked)}/>
              <button type="button" className="ps-drag-handle" data-schedule-drag={b.id} aria-label={`Kéo bài ${b.name}`} aria-describedby="schedule-drag-help" title={b.name} disabled={locked} onClick={() => selection(b.id, !row)}><GripVertical size={15} aria-hidden="true"/><PublishMedia bundle={b}/><span><strong>{b.name}</strong><small>{row?.udid ? label(row.udid) : `${b.video ? "Video" : `${b.images.length} ảnh`} · Chưa có máy`}</small></span></button>
            </div>;
          })}
          {!p.bundles.length && <p className="ps-empty">Quét thư mục bài trong Thiết lập để bắt đầu. <button type="button" onClick={p.onSource}>Về Thiết lập</button></p>}
          {p.bundles.length > 0 && !p.bundles.some(b => `${b.name} ${p.captions[b.id] ?? b.caption}`.toLocaleLowerCase().includes(postQuery.toLocaleLowerCase())) && <p className="ps-empty">Không có bài khớp từ khóa.</p>}
          </div>
        </section>
        <section className={`ps-machines ${preview ? "is-drag-over" : ""}`} data-schedule-drop aria-label="Vùng máy nhận bài">
          <header><h3>Thả bài vào máy</h3><span>{draft.selectedMachines.filter(id => readyIds.includes(id)).length} máy đã chọn</span></header>
          <label className="ps-search"><Search size={15} aria-hidden="true"/><input aria-label="Tìm máy hẹn giờ" placeholder="Tìm số máy" value={machineQuery} onChange={e => setMachineQuery(e.target.value)}/></label>
          <div className="ps-selection-tools"><button type="button" className="ps-quick-select" title="Tự chọn bài và máy phù hợp, rồi gán ngay" disabled={locked || !p.bundles.length} onClick={quickAssign}><Zap size={16} aria-hidden="true"/>Chọn nhanh</button><button type="button" disabled={locked || !readyIds.length} onClick={() => edit(d => ({ ...d, selectedMachines: readyIds }))}>Chọn máy sẵn sàng</button><button type="button" disabled={locked || !draft.selectedMachines.length} onClick={() => edit(d => ({ ...d, selectedMachines: [] }))}>Bỏ chọn máy</button></div>
          <div className="ps-machine-list" data-schedule-scroll>{filteredMachines.map(d => {
            const assigned = draft.rows.filter(r => r.udid === d.udid);
            const willReceive = preview?.assigned.filter(a => a.udid === d.udid) ?? [];
            return <div key={d.udid} data-schedule-device={d.udid} className={`ps-machine ${draft.selectedMachines.includes(d.udid) ? "is-picked" : ""} ${willReceive.length ? "is-drop-target" : ""} ${assigned.some(r => r.bundleId === activePost) ? "is-linked" : ""} ${d.status !== "ready" ? "is-unavailable" : ""}`}>
              <label><input type="checkbox" checked={draft.selectedMachines.includes(d.udid)} disabled={locked || d.status !== "ready"} aria-label={`Chọn máy hẹn giờ ${label(d.udid)}`} onChange={e => edit(current => ({ ...current, selectedMachines: e.target.checked ? [...new Set([...current.selectedMachines, d.udid])] : current.selectedMachines.filter(id => id !== d.udid) }))}/><strong>{label(d.udid)}</strong><span>{d.status === "ready" ? assigned.length ? `${assigned.length} bài` : "Trống" : "Chưa sẵn sàng"}</span></label>
              {assigned.length ? <p className="ps-machine-post" title={assigned.map(r => name(r.bundleId)).join(", ")}>{assigned.map(r => name(r.bundleId)).join(", ")}</p> : <p className="ps-machine-empty">{d.status === "ready" ? "Thả để gán bài" : "Không nhận bài"}</p>}
              {willReceive.length > 0 && <p className="ps-drop-preview">Nhận {willReceive.map(a => name(a.bundleId)).join(", ")}</p>}
            </div>;
          })}{!inScope.length ? <p className="ps-empty">Chọn phạm vi máy trong Thiết lập để phân công.</p> : !filteredMachines.length && <div className="ps-empty ps-empty-machines"><p>Không có máy khớp từ khóa.</p><button type="button" onClick={() => { setMachineQuery(""); drag.root.current?.querySelector<HTMLInputElement>('[aria-label="Tìm máy hẹn giờ"]')?.focus(); }}>Xóa tìm kiếm</button></div>}</div>
          <div className="ps-drop-bar"><span id="schedule-drag-help">Thả vào một máy để gán riêng; thả vào vùng chung để gán cả nhóm.</span><button type="button" disabled={locked || !chosen.length || !draft.selectedMachines.some(id => readyIds.includes(id))} onClick={() => drop(chosen.map(b => b.id), {})}>Gán bài đã chọn</button></div>
        </section>
      </div>
      <section className="ps-plan" aria-label="Bảng phân công lịch đăng">
        <div className="ps-plan-heading"><h3>Phân công</h3><span>{assignmentsCount}/{draft.rows.length} bài có máy</span></div>
        <div className="ps-table-scroll"><table className="ps-plan-table" aria-label="Bảng phân công"><thead><tr><th>Bài đăng</th><th>Máy nhận</th><th>Giờ đăng</th><th>Kết quả kiểm tra</th><th><span className="ps-sr-only">Thao tác</span></th></tr></thead><tbody>
          {draft.rows.map((row, index) => <tr key={row.id} data-schedule-row={row.bundleId} className={activePost === row.bundleId ? "is-linked" : ""} onMouseEnter={() => setHoveredPost(row.bundleId)} onMouseLeave={() => setHoveredPost(null)} onFocusCapture={() => setFocusedPost(row.bundleId)} onBlurCapture={e => { if (!e.currentTarget.contains(e.relatedTarget)) setFocusedPost(null); }}>
            <td data-schedule-field="verdict" tabIndex={-1}><strong title={name(row.bundleId)}>{name(row.bundleId)}</strong><small className="ps-mobile-result">{rowStatus(row, index)}</small></td>
            <td><select data-schedule-field="rowMachine" aria-label={`Máy nhận ${name(row.bundleId)}`} value={row.udid} disabled={locked} onChange={e => setMachine(row, e.target.value)}><option value="">Chưa có máy</option>{row.udid && !inScope.some(d => d.udid === row.udid) && <option value={row.udid}>Máy ngoài phạm vi / mất kết nối</option>}{inScope.map(d => <option key={d.udid} value={d.udid} disabled={d.status !== "ready"}>{label(d.udid)}{d.status !== "ready" ? " · Chưa sẵn sàng" : ""}</option>)}</select></td>
            <td>{individualTimes ? <div className="ps-row-time"><input data-schedule-field="rowTime" type="time" aria-label={`Giờ bài ${name(row.bundleId)}`} disabled={locked} value={scheduleTime(row, draft.commonTime)} onChange={e => edit(d => ({ ...d, rows: d.rows.map(r => r.id === row.id ? { ...r, time: e.target.value, timeMode: "custom" } : r) }))}/>{row.timeMode === "custom" ? <button type="button" disabled={locked} aria-label={`Dùng giờ chung cho ${name(row.bundleId)}`} onClick={() => edit(d => ({ ...d, rows: d.rows.map(r => r.id === row.id ? { ...r, timeMode: "common" } : r) }))}>Dùng giờ chung</button> : <small>Giờ chung</small>}</div> : <span>{scheduleTime(row, draft.commonTime) || "Chưa đặt giờ"}{row.timeMode === "custom" && <small>Giờ riêng</small>}</span>}</td>
            <td className={rowIssue(row) && rowIssue(row) !== "Chọn ngày và giờ đăng" ? "ps-row-issue" : "ps-row-status"}>{rowStatus(row, index)}</td>
            <td><div className="ps-row-actions"><button type="button" disabled={locked || !row.udid} aria-label={`Gỡ gán ${name(row.bundleId)}`} onClick={() => setMachine(row, "")}><Unlink size={15} aria-hidden="true"/></button><button type="button" disabled={locked} aria-label={`Bỏ bài ${name(row.bundleId)}`} onClick={() => selection(row.bundleId, false)}><X size={15} aria-hidden="true"/></button></div></td>
          </tr>)}
        </tbody></table>{!draft.rows.length && <p className="ps-empty">Chọn hoặc kéo bài vào vùng máy để tạo bảng phân công.</p>}</div>
        <p className="ps-time-note">Giờ máy tính: {Intl.DateTimeFormat().resolvedOptions().timeZone}. Lịch bắt đầu xử lý lúc đã đặt; các bài cùng máy chạy lần lượt. Giữ Riviu mở và máy kết nối. Nếu mở Riviu sau giờ hẹn, lịch được đánh dấu lỡ lịch.</p>
        {(p.blockingReason || notice) && <p className="ps-notice" role="status">{p.blockingReason || notice}</p>}
        {reviewed && <div className="ps-review"><strong>{reviewed.report.canExecute ? "Các bài đã đạt kiểm tra" : "Có bài cần xử lý trong bảng phân công"}</strong>{reviewed.report.canExecute && <label className="ps-inline-check"><input data-schedule-field="confirm" type="checkbox" checked={confirmed} disabled={locked} onChange={e => setConfirmed(e.target.checked)}/>Tôi xác nhận đăng công khai các bài đúng lịch trên</label>}</div>}
      </section>
    </div>
    <footer className="ps-footer"><div><strong>{draft.rows.length} bài · {new Set(draft.rows.filter(r => r.udid).map(r => r.udid)).size} máy</strong><span aria-live="polite">{busy ? "Đang xử lý lịch…" : next?.text ?? "Lịch đã sẵn sàng để lưu"}</span></div><div>{next && next.target?.field !== "check" && <button className="ps-next-step" type="button" disabled={locked} onClick={goToNext}>{next.action}</button>}<button data-schedule-field="check" type="button" disabled={locked || !valid || !!p.blockingReason} onClick={() => void check()}>{busy ? "Đang xử lý…" : "Kiểm tra lịch"}</button><button type="button" className="primary" disabled={locked || !valid || !reviewed?.report.canExecute || !confirmed} onClick={() => void save()}>Lưu lịch {draft.rows.length} bài</button></div></footer>
    {drag.view && <div className="ps-drag-ghost" aria-hidden="true" style={{ left: Math.min(drag.view.x + 14, window.innerWidth - 205), top: Math.max(8, drag.view.y - 42) }}><GripVertical size={16}/>{drag.view.ids.length} bài{preview && <small>{preview.assigned.length} bài sẽ được gán{preview.missing.length ? ` · thiếu ${preview.missing.length} chỗ` : ""}</small>}</div>}
  </section>;
}
