import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, Check, FolderOpen, ListFilter, Music2, Pencil, Search, Undo2, Zap, X } from "lucide-react";
import type { PublishWizardProps } from "./PublishWizard";
import { PublishMedia } from "./PublishMedia";
import { PublishDialog } from "./PublishDialog";
import { PublishPreflightResult } from "./PublishPreflightResult";
import { MachineChoice } from "../MachineChoice";
import { orderDevicesByNumber, tileName, tileNumber } from "../../deviceNaming";
import { pickDirectory } from "../../pickFile";
import { describeError } from "../../describeError";
import { requestConfirm } from "../../confirmStore";
import { assignDevice } from "./publishAssignments";
import "../../styles/publish-quick.css";
import { allocateQuickPosts } from "./publishQuickAllocation";
import { publishSelectionStatus } from "./publishSelectionStatus";
import { deviceGuardBlock, deviceGuardPending } from "./publishDeviceGuardState";
import type { PublishDeviceGuards } from "../../types";

type QuickProps = PublishWizardProps & {
  blockingReason?: string;
  deviceGuards?: PublishDeviceGuards;
  onPendingPublication?: (campaignId: string) => void;
  onAssignmentChange?: (ids: string[], assignments: Record<string, string>) => void;
};
type CaptionContext = { id: string; source: string; origin: HTMLButtonElement };
type AssignmentSnapshot = { props: QuickProps; activeId?: string; activeVisible: boolean; locked: boolean; key: string };

function canReceive(p: QuickProps, udid: string) {
  return p.eligible.includes(udid) && p.devices.some(d => d.udid === udid && (d.status === "ready" || d.status === "busy" || d.status === "connected"))
    && (p.deviceGuards === undefined || p.deviceGuards[udid] !== undefined);
}
function validOldMachine(p: QuickProps, id: string) {
  const old = p.assignments[id];
  return old && canReceive(p, old) ? old : undefined;
}
function canAssign(snapshot: AssignmentSnapshot, id: string, udid: string, contextual: boolean) {
  const p = snapshot.props;
  return !snapshot.locked && p.active !== false && p.selectedIds.includes(id)
    && !!p.manifest?.bundles.some(b => b.id === id)
    && (!contextual || (snapshot.activeId === id && snapshot.activeVisible))
    && (!udid || canReceive(p, udid));
}

/** Ba khung chỉ sửa bản nháp; PublishPage tiếp tục sở hữu preflight và mọi tác vụ thật. */
export function PublishQuickSetup(p: QuickProps) {
  const [query, setQuery] = useState("");
  const [deviceQuery, setDeviceQuery] = useState("");
  const [onlyFree, setOnlyFree] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [activeId, setActiveId] = useState<string>();
  const [photo, setPhoto] = useState(0);
  const [captionContext, setCaptionContext] = useState<CaptionContext | null>(null);
  const [dialog, setDialog] = useState<"check" | null>(null);
  const [error, setError] = useState("");
  const [picked, setPicked] = useState<string[]>([]);
  const [notice, setNotice] = useState("");
  const [undo, setUndo] = useState<{ source: string; ids: string[]; assignments: Record<string, string>; picked: string[]; after: string } | null>(null);
  const [reportPage, setReportPage] = useState(0);
  const [confirming, setConfirming] = useState(false);
  const latest = useRef<AssignmentSnapshot | null>(null);
  const pendingAssignment = useRef(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const linksRef = useRef<HTMLDivElement>(null);
  const postRefs = useRef(new Map<string, HTMLButtonElement>());
  const revealAfterCommit = useRef<string | null>(null);
  const bundles = p.manifest?.bundles ?? [];
  const visibleBundles = bundles.filter(b => `${b.name} ${p.captions[b.id] ?? b.caption}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
  const selected = bundles.filter(b => p.selectedIds.includes(b.id));
  const captionCounts = new Map<string, number>();
  for (const bundle of selected) {
    const caption = (p.captions[bundle.id] ?? bundle.caption).trim().replace(/\s+/g, " ");
    if (caption) captionCounts.set(caption, (captionCounts.get(caption) ?? 0) + 1);
  }
  const duplicateCaptions = [...captionCounts.values()].filter(count => count > 1).reduce((total, count) => total + count, 0);
  const sourceWarnings = p.manifest?.notices ?? [];
  const active = bundles.find(b => b.id === activeId) ?? selected[0] ?? bundles[0];
  const activeVisible = !!active && visibleBundles.some(b => b.id === active.id);
  const captionBundle = captionContext?.source === p.sourceRoot && p.active !== false ? bundles.find(b => b.id === captionContext.id) : undefined;
  const devices = useMemo(() => orderDevicesByNumber(p.devices, p.metas), [p.devices, p.metas]);
  const selectable = devices.filter(d => (d.status === "ready" || d.status === "busy" || d.status === "connected") && p.eligible.includes(d.udid));
  const ready = selectable.filter(d => canReceive(p, d.udid));
  const pendingBlock = selected.map(bundle => p.assignments[bundle.id]).filter(Boolean)
    .map(udid => deviceGuardBlock(p.deviceGuards, udid)).find(Boolean);
  const unknownGuard = selected.some(bundle => {
    const id = p.assignments[bundle.id];
    return id && p.deviceGuards !== undefined && !p.deviceGuards[id];
  }) ? "Chưa kiểm tra được bài đang chờ" : undefined;
  const selection = publishSelectionStatus({ selectedIds: p.selectedIds, bundles, assignments: p.assignments,
    captions: p.captions, eligible: p.eligible, ready: selectable.map(d => d.udid), blockingReason: unknownGuard ?? p.blockingReason });
  const mapped = selection.mapped;
  const locked = p.busy || p.scanning || p.preflightLoading;
  const complete = selection.ready;
  const checkReason = p.scanning ? "Đang quét và xác nhận nội dung…" : p.preflightLoading ? "Đang kiểm tra đợt đăng…" : p.busy ? "Đang xử lý tác vụ…" : selection.reason;
  const mappingKey = JSON.stringify([p.selectedIds, p.selectedIds.map(id => p.assignments[id] ?? "")]);
  // Chỉ snapshot đã commit được phép cấp quyền cho handler/continuation sau await.
  const assignmentKey = JSON.stringify([p.sourceRoot, p.manifest, p.selectedIds, p.assignments, p.eligible,
    p.devices.map(d => [d.udid, d.status]), p.deviceGuards]);
  useLayoutEffect(() => {
    latest.current = { props: p, activeId: active?.id, activeVisible, locked, key: assignmentKey };
    if (revealAfterCommit.current) {
      const button = postRefs.current.get(revealAfterCommit.current);
      button?.focus({ preventScroll: true });
      button?.scrollIntoView?.({ block: "nearest" });
      revealAfterCommit.current = null;
    }
  });
  useLayoutEffect(() => () => { latest.current = null; }, []);
  useEffect(() => { if (p.active === false) { setDialog(null); setCaptionContext(null); } }, [p.active]);
  useEffect(() => { if (captionContext && !captionBundle) setCaptionContext(null); }, [captionContext, captionBundle]);

  const select = (id: string, checked: boolean) => {
    if (locked) return;
    p.onSelect(checked ? [...new Set([...p.selectedIds, id])] : p.selectedIds.filter(x => x !== id));
  };
  const applyAssignment = (ids: string[], assignments: Record<string, string>) => {
    if (p.onAssignmentChange) p.onAssignmentChange(ids, assignments);
    else { p.onSelect(ids); p.onAssign(assignments); }
  };
  const autoAssign = () => {
    if (locked || confirming || !bundles.length) return;
    const next = allocateQuickPosts({ sourceIds: bundles.map(b => b.id), selectedIds: p.selectedIds,
      assignments: p.assignments, eligibleIds: p.eligible, readyIds: ready.map(d => d.udid), picked });
    const after = JSON.stringify([next.ids, next.ids.map(id => next.assignments[id] ?? "")]);
    if (after !== mappingKey) {
      setUndo({ source: p.sourceRoot, ids: p.selectedIds, assignments: p.assignments, picked, after });
      applyAssignment(next.ids, next.assignments);
    }
    setPicked(next.picked);
    setError("");
    const leftover = bundles.length - next.ids.length;
    const freeReady = ready.length - next.picked.length;
    setNotice(!next.ids.length
      ? "Chưa có máy sẵn sàng trong phạm vi. Kiểm tra kết nối hoặc chọn lại nhóm máy."
      : leftover
        ? `Đã gán ${next.ids.length} bài cho ${next.ids.length} máy. ${leftover} bài còn lại vì hết máy sẵn sàng hoặc đạt giới hạn 100 bài.`
        : freeReady
          ? `Đã gán ${next.ids.length} bài cho ${next.ids.length} máy. Còn ${freeReady} máy sẵn sàng nhưng hết bài trong nguồn.`
          : `Đã gán ${next.ids.length} bài cho ${next.ids.length} máy.`);
  };
  const undoAssignment = () => {
    if (locked || confirming || !undo || undo.source !== p.sourceRoot || undo.after !== mappingKey) return;
    applyAssignment(undo.ids, undo.assignments); setPicked(undo.picked); setUndo(null); setNotice("Đã hoàn tác gán nhanh.");
  };
  const chooseFolder = async () => {
    if (p.busy) return;
    const source = p.sourceRoot;
    try {
      const path = await pickDirectory(), current = latest.current;
      if (path && current && !current.props.busy && current.props.active !== false && current.props.sourceRoot === source) current.props.onSource(path);
    } catch (e) { if (latest.current) setError(describeError(e)); }
  };
  const deviceIndex = new Map(devices.map((d, i) => [d.udid, { device: d, index: i }]));
  const assignedByDevice = new Map(selected.filter(b => p.assignments[b.id]).map(b => [p.assignments[b.id], b]));
  const label = (udid: string) => {
    const found = deviceIndex.get(udid);
    return found ? `Máy ${tileNumber(found.index + 1, p.metas.get(udid))} · ${tileName(found.device, p.metas.get(udid))}` : `Máy không còn trong danh sách · ${udid}`;
  };
  const assign = async (id: string, udid: string, contextual = false) => {
    const start = latest.current;
    if (!start || pendingAssignment.current || !canAssign(start, id, udid, contextual)) return;
    if (start.props.assignments[id] === udid) return;
    const old = validOldMachine(start.props, id);
    const other = Object.keys(start.props.assignments).find(key => key !== id && start.props.assignments[key] === udid);
    if (udid && other && !old) {
      pendingAssignment.current = true; setConfirming(true);
      const name = (key: string) => start.props.manifest?.bundles.find(b => b.id === key)?.name ?? key;
      const accepted = await requestConfirm({ title: `Thay bài trên ${label(udid)}?`,
        message: `${name(id)} sẽ nhận máy này. ${name(other)} sẽ trở về chờ ghép máy. Đây không phải đổi chỗ vì bài mới chưa có máy cũ hợp lệ. Chỉ sửa bản nháp, chưa đăng bài.`,
        confirmLabel: "Thay bài trong bản nháp", cancelLabel: "Hủy" });
      pendingAssignment.current = false;
      if (!latest.current) return;
      setConfirming(false);
      if (!accepted) return;
    }
    const current = latest.current;
    if (!current || current.key !== start.key || !canAssign(current, id, udid, contextual)) {
      if (current) setNotice("Phân công, nguồn hoặc trạng thái máy đã đổi. Chọn lại bài và máy trước khi gán.");
      return;
    }
    // Máy cũ không hợp lệ không được truyền sang bài khác bởi semantics đổi chỗ của assignDevice.
    const base = { ...current.props.assignments };
    if (!udid || !validOldMachine(current.props, id)) delete base[id];
    current.props.onAssign(udid ? assignDevice(base, id, udid) : base);
    setNotice(udid ? other ? old ? "Đã đổi chỗ hai bài trong bản nháp. Chưa đăng bài." : "Đã thay bài; bài cũ đang chờ ghép máy khác. Chưa đăng bài." : "Đã ghép bài trong bản nháp. Chưa đăng bài." : "Đã bỏ ghép bài, vẫn giữ lựa chọn nội dung.");
  };
  const clearDevice = (udid: string) => {
    if (locked || confirming) return;
    p.onAssign(Object.fromEntries(Object.entries(p.assignments).filter(([, id]) => id !== udid)));
  };
  const openCaption = (id: string, origin: HTMLButtonElement) => {
    setPhoto(0); setCaptionContext({ id, source: p.sourceRoot, origin });
  };
  const captionReturnTarget = () => {
    const current = latest.current;
    if (current?.props.active === false) return rootRef.current?.closest(".publish-page")?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]') ?? null;
    if (captionContext && current?.props.sourceRoot === captionContext.source
      && current.props.manifest?.bundles.some(b => b.id === captionContext.id)
      && captionContext.origin.isConnected && !captionContext.origin.closest("[hidden]")) return captionContext.origin;
    return linksRef.current;
  };
  const selectedBlockedDevice = selected.map(bundle => p.assignments[bundle.id]).find(udid => !!udid && !!deviceGuardBlock(p.deviceGuards, udid));
  const selectedPending = selectedBlockedDevice ? deviceGuardPending(p.deviceGuards?.[selectedBlockedDevice]) : undefined;
  const filteredDevices = devices.filter(d => `${label(d.udid)} ${p.metas.get(d.udid)?.handle ?? ""}`.toLocaleLowerCase().includes(deviceQuery.toLocaleLowerCase())
    && (!onlyFree || (canReceive(p, d.udid) && !assignedByDevice.has(d.udid))));
  return <div ref={rootRef} className="publish-quick" hidden={p.active === false}>
    <div className="pq-setup-scroll">
      <div className="pq-setup-tools">
        <div className="pq-source"><label htmlFor="publish-source-folder">Thư mục bài đăng</label><div className="pq-source-controls"><input id="publish-source-folder" aria-label="Thư mục nguồn" value={p.sourceRoot} onChange={e => { if (!p.busy) p.onSource(e.target.value); }} disabled={p.busy} placeholder="Đường dẫn thư mục chứa bài đăng"/><button type="button" disabled={p.busy} onClick={() => void chooseFolder()}><FolderOpen size={16}/>Chọn thư mục</button><button type="button" disabled={locked || !p.sourceRoot} onClick={() => { if (!locked && p.sourceRoot) void p.onScan(p.sourceRoot); }}>{p.scanning ? "Đang quét…" : "Quét"}</button></div></div>
        <div className={`pq-settings${settingsOpen ? " is-open" : ""}`} onKeyDown={e => { if (e.key === "Escape") { setSettingsOpen(false); e.currentTarget.querySelector<HTMLButtonElement>(".pq-settings-toggle")?.focus(); } }}>
          <button type="button" className="pq-settings-toggle" aria-expanded={settingsOpen} aria-controls="publish-google-settings" onClick={() => setSettingsOpen(open => !open)}>Thiết lập Google Sheet</button>
          <div id="publish-google-settings" className="pq-settings-content">{p.settings}</div>
        </div>
      </div>
      {(sourceWarnings.length > 0 || duplicateCaptions > 0 || p.blockingReason || error || p.notices) && <div className="pq-messages" tabIndex={0} aria-label="Thông báo thiết lập đăng bài">
        {p.blockingReason && !pendingBlock && <p className="pq-blocking-reason" role="status">{p.blockingReason}</p>}
        {error && <div className="pq-error" role="alert">{error}<button type="button" aria-label="Đóng lỗi" onClick={() => setError("")}><X size={14}/></button></div>}
        {sourceWarnings.length > 0 && <details className="pq-hint"><summary>Cảnh báo nguồn ({sourceWarnings.length})</summary>
          <ul>{sourceWarnings.map((warning, index) => <li key={`${warning.path}:${index}`}><span>{warning.message}</span><br/><small>{warning.path}</small></li>)}</ul>
          <p>Cảnh báo không sửa dữ liệu nguồn. Bài thiếu đối tác vẫn có thể đăng và ghi link; tên đối tác tương ứng sẽ trống.</p>
        </details>}
        {duplicateCaptions > 0 && <p className="pq-hint" role="status">{duplicateCaptions} bài đã chọn có caption trùng. Nếu cùng tài khoản đã đăng nội dung tương tự, việc xác minh có thể lâu hơn; nội dung giữ nguyên và không tự sửa.</p>}
        {p.notices}
      </div>}
      <div className="pq-columns">
        <section className="pq-library pq-panel" aria-label="Nội dung đăng">
          <header><h2>Chọn bài đăng</h2><span role={p.scanning ? "status" : undefined}>{p.scanning ? "Đang đọc nội dung…" : `${selected.length}/${bundles.length}`}</span></header>
          <div className="pq-panel-tools"><label className="pq-search"><Search size={15}/><input aria-label="Tìm bài đăng" placeholder="Tìm bài đăng" value={query} onChange={e => setQuery(e.target.value)}/></label>
            <div className="pq-tools"><button type="button" className="ghost" title="Chọn toàn bộ bài trong nguồn, kể cả bài bị bộ lọc ẩn" disabled={locked || !bundles.length} onClick={() => { if (!locked) p.onSelect(bundles.map(b => b.id)); }}>Chọn tất cả bài</button><button type="button" className="ghost" disabled={locked || !selected.length} onClick={() => { if (!locked) p.onSelect([]); }}>Bỏ chọn toàn bộ bài</button></div>
          </div>
          <div className="pq-posts" tabIndex={0} aria-label="Danh sách bài đăng">{visibleBundles.map(b => <article key={b.id} className={b.id === active?.id ? "is-active" : ""}>
            <input type="checkbox" aria-label={`Chọn ${b.name}`} checked={p.selectedIds.includes(b.id)} disabled={locked} onChange={e => select(b.id, e.target.checked)}/>
            <button type="button" className="pq-post-pick" ref={node => { if (node) postRefs.current.set(b.id, node); else postRefs.current.delete(b.id); }} aria-label={`Chọn bài đang gán · ${b.name}`} aria-pressed={b.id === active?.id} onClick={() => setActiveId(b.id)}><PublishMedia bundle={b}/><span><strong>{b.name}</strong><small>{b.mediaKind === "video" ? "Video MP4" : `${b.images.length} ảnh`} · {p.assignments[b.id] ? label(p.assignments[b.id]) : p.selectedIds.includes(b.id) ? "Chờ ghép" : "Chưa chọn"}</small></span></button>
            <button type="button" className="pq-edit-button ghost" aria-label={`Xem ảnh và sửa caption · ${b.name}`} title="Xem ảnh và sửa caption" onClick={e => openCaption(b.id, e.currentTarget)}><Pencil size={14}/></button>
          </article>)}
            {!visibleBundles.length && <p className="pq-list-empty">{bundles.length ? "Không có bài khớp với từ khóa." : "Chọn thư mục ở trên, rồi bấm Quét để xem bài đăng."}</p>}
          </div>
        </section>
        <section className="pq-mapping pq-panel" aria-label="Liên kết bài và máy">
          <header><h2>Bài ↔ máy</h2><span>{mapped}/{selected.length} ghép</span></header>
          <div className="pq-links" ref={linksRef} tabIndex={0} aria-label="Các cặp bài và máy">{selected.map((b, index) => {
            const udid = p.assignments[b.id], valid = !!validOldMachine(p, b.id), handle = udid ? p.metas.get(udid)?.handle : undefined;
            return <article key={b.id} className={`pq-link${b.id === active?.id ? " is-active" : ""}${valid ? "" : " needs-machine"}`}>
              <button type="button" className="pq-link-post" title={b.name} aria-label={`Chọn cặp bài · ${b.name}`} aria-pressed={b.id === active?.id} onClick={() => setActiveId(b.id)}><span>{String(index + 1).padStart(2, "0")}</span><strong>{b.name}</strong></button>
              <label className="pq-link-target"><span aria-hidden="true">→</span><select aria-label={`Máy nhận bài ${b.name}`} disabled={locked || confirming} value={udid ?? ""} onChange={e => void assign(b.id, e.target.value)}><option value="">Chưa ghép</option>
                {udid && !selectable.some(d => d.udid === udid) && <option value={udid}>{label(udid)} · {p.eligible.includes(udid) ? "Chưa sẵn sàng" : "Ngoài phạm vi"}</option>}
                {selectable.map(d => <option key={d.udid} value={d.udid} disabled={!canReceive(p, d.udid)}>{label(d.udid)}{deviceGuardBlock(p.deviceGuards, d.udid) ? ` · ${deviceGuardBlock(p.deviceGuards, d.udid)}` : ""}</option>)}
              </select></label>
              <div className="pq-link-detail"><span>{b.mediaKind === "video" ? "Video MP4" : `${b.images.length} ảnh`}</span><button type="button" className="ghost" aria-label={`Sửa caption · ${b.name}`} onClick={e => openCaption(b.id, e.currentTarget)}>Sửa caption</button></div>
              <div className="pq-link-account"><span title={handle || (udid ? label(udid) : "Cần chọn máy")}>{handle || (udid ? valid ? "Đã ghép máy" : "Máy chưa hợp lệ" : "Cần chọn máy")}</span><button type="button" className="pq-unlink ghost" aria-label={`Bỏ ghép bài ${b.name}`} title="Bỏ ghép bài này" disabled={locked || confirming || !udid} onClick={() => void assign(b.id, "")}><X size={14}/></button></div>
            </article>;
          })}{!selected.length && <div className="pq-list-empty"><strong>Chưa chọn bài nào</strong><p>Tick bài bên trái hoặc dùng Chọn nhanh để ghép với máy sẵn sàng.</p></div>}</div>
        </section>
        <section className="pq-devices pq-panel" aria-label="Máy thực hiện">
          <header className="pq-devices-header"><div><h2>Thiết bị</h2><div className={`pq-active-post${active && !activeVisible ? " is-hidden" : ""}`} role="status">{active && !activeVisible ? <><span>Bài đang gán bị lọc ẩn.</span><button type="button" className="ghost" onClick={() => { revealAfterCommit.current = active.id; setQuery(""); }}>Hiện bài</button></> : <span title={active?.name}>Gán: <strong>{active?.name ?? "Chưa chọn bài"}</strong>{active && !p.selectedIds.includes(active.id) ? " · Tick bài trước" : ""}</span>}</div></div>{p.scopeControl ?? <span>{ready.length} sẵn sàng</span>}</header>
          <div className="pq-panel-tools pq-device-tools">
            <div className="pq-quick-actions"><button type="button" className="pq-quick-button" disabled={locked || confirming || !bundles.length} onClick={autoAssign}><Zap size={15} aria-hidden="true"/>Chọn nhanh</button><button type="button" aria-label="Hoàn tác gán nhanh" disabled={locked || confirming || !undo || undo.source !== p.sourceRoot || undo.after !== mappingKey} onClick={undoAssignment}><Undo2 size={15} aria-hidden="true"/>Hoàn tác</button></div>
            <div className="pq-device-searchbar"><label className="pq-search"><Search size={15}/><input aria-label="Tìm số máy" value={deviceQuery} onChange={e => setDeviceQuery(e.target.value)} placeholder="Tìm máy / tài khoản"/></label>
            <details className="pq-device-filters" onKeyDown={e => { if (e.key === "Escape") { e.currentTarget.open = false; e.currentTarget.querySelector("summary")?.focus(); } }}><summary aria-label="Bộ lọc thiết bị" title="Bộ lọc thiết bị"><ListFilter size={15}/><span>Bộ lọc thiết bị</span></summary><div>
              <label><input type="checkbox" checked={onlyFree} onChange={e => setOnlyFree(e.target.checked)}/>Chỉ hiện máy sẵn sàng chưa có bài</label>
              <button type="button" className="ghost" title="Chọn tất cả máy sẵn sàng trong phạm vi, kể cả ngoài kết quả tìm kiếm" disabled={locked || confirming || !ready.length} onClick={() => { if (!locked && !confirming) setPicked(ready.map(d => d.udid)); }}>Chọn tất cả sẵn sàng</button>
              <button type="button" className="ghost" disabled={locked || confirming || (!picked.length && !selected.some(b => p.assignments[b.id]))} onClick={() => { if (!locked && !confirming) { setPicked([]); p.onAssign({}); } }}>Bỏ chọn toàn bộ máy</button>
              <small>{mapped} bài đã ghép · {ready.length} máy sẵn sàng / {devices.length} tổng. Bộ lọc chỉ đổi danh sách đang xem, không đổi Chọn nhanh.</small>
            </div></details></div>
          </div>
          {!p.eligible.length && <p className="pq-scope-hint">Chọn Toàn bộ máy hoặc một nhóm để ghép bài.</p>}
          <div className="pq-machine-grid machine-choice-grid" tabIndex={0} aria-label="Danh sách thiết bị để gán bài">{filteredDevices.map(d => {
            const i = deviceIndex.get(d.udid)!.index, assigned = assignedByDevice.get(d.udid), checked = picked.includes(d.udid) || Boolean(assigned);
            const block = deviceGuardBlock(p.deviceGuards, d.udid), pending = deviceGuardPending(p.deviceGuards?.[d.udid]);
            const current = assigned?.id === active?.id && !!assigned;
            const action = current ? "Đã gán" : assigned ? active && validOldMachine(p, active.id) ? "Đổi chỗ" : "Thay bài" : "Gán";
            const assignable = !!active && activeVisible && p.selectedIds.includes(active.id) && canReceive(p, d.udid);
            return <div className="pq-device-row" key={d.udid}>
              <MachineChoice number={tileNumber(i + 1, p.metas.get(d.udid))} name={tileName(d, p.metas.get(d.udid))} status={d.status} reason={d.lastError} label={`Chọn ${label(d.udid)}`} checked={checked} disabled={locked || confirming || (!checked && !canReceive(p, d.udid))} onChange={value => {
                if (locked || confirming || (value && !canReceive(p, d.udid))) return;
                setPicked(value ? [...new Set([...picked, d.udid])] : picked.filter(id => id !== d.udid)); if (!value) clearDevice(d.udid);
              }} detail={<>
                <span title={assigned?.name ?? p.metas.get(d.udid)?.handle ?? ""}>{assigned?.name ?? p.metas.get(d.udid)?.handle ?? (p.eligible.includes(d.udid) ? "Chưa ghép bài" : "Ngoài phạm vi")}</span>
                {(block || pending) && <div className="publish-device-pending" role="status"><strong>{block ?? "Bài cũ còn cần kiểm tra link"}</strong>
                  {pending && <><span>{pending.reason}</span>{p.onPendingPublication && <button type="button" disabled={locked} aria-label={`Xem bài đang chờ · ${label(d.udid)}`} onClick={() => { if (!locked) p.onPendingPublication?.(pending.campaignId); }}>Xem bài đang chờ</button>}</>}
                </div>}
              </>}/>
              <button type="button" className="pq-assign-button" aria-label={`${action} ${active?.name ?? "bài"} · ${label(d.udid)}`} disabled={locked || confirming || !assignable || current} title={!activeVisible ? "Hiện lại bài đang gán trước khi phân công" : block ?? (!p.eligible.includes(d.udid) ? "Máy ngoài phạm vi" : action)} onClick={() => { if (active) void assign(active.id, d.udid, true); }}>{action}</button>
            </div>;
          })}{!filteredDevices.length && <p className="pq-list-empty">{devices.length ? "Không có máy khớp bộ lọc." : "Chưa có thiết bị. Kết nối máy để ghép bài."}</p>}</div>
        </section>
      </div>
    </div>
    <footer className="pq-footer"><div><strong>{selected.length} bài đã chọn</strong><span>{mapped}/{selected.length} bài có máy · mỗi máy một bài · Sheet {p.sheet ? "bật" : "tắt"}</span><details className="pq-run-options" onKeyDown={e => { if (e.key === "Escape") { e.currentTarget.open = false; e.currentTarget.querySelector("summary")?.focus(); } }}><summary>Tùy chọn · Sheet {p.sheet ? "bật" : "tắt"} · {p.cleanup ? "Xóa bản chuyển" : "Giữ bản chuyển"}</summary><div className="pq-options"><label><input type="checkbox" checked={p.sheet} disabled={locked} onChange={e => { if (!locked) p.onSheet(e.target.checked); }}/> Ghi kết quả lên Sheet</label><label><input type="checkbox" checked={p.cleanup} disabled={locked} onChange={e => { if (!locked) p.onCleanup(e.target.checked); }}/> Xóa bản chuyển sau khi đăng thành công</label></div></details>{notice && <details className="pq-assignment-notice"><summary title={notice}><span role="status">{notice}</span></summary><p>{notice}</p></details>}{checkReason && <small id="publish-check-reason" role="status">{selectedBlockedDevice ? `${label(selectedBlockedDevice)}: ${checkReason}` : checkReason}</small>}{selectedPending && p.onPendingPublication && <button type="button" className="pq-pending-action" disabled={locked} onClick={() => { if (!locked) p.onPendingPublication?.(selectedPending.campaignId); }}>Xem bài đang chờ</button>}</div><button type="button" className="primary" aria-describedby={checkReason ? "publish-check-reason" : undefined} disabled={locked || !complete} onClick={() => { if (!locked && complete) { setReportPage(0); setDialog("check"); void p.onPreflight(); } }}>{p.preflightLoading ? "Đang kiểm tra…" : "Kiểm tra & đăng"}<ArrowRight size={16}/></button></footer>
    {captionContext && captionBundle && <PublishDialog title={`Ảnh & caption · ${captionBundle.name}`} wide returnFocus={captionReturnTarget} fallbackFocus={() => linksRef.current} onClose={() => setCaptionContext(null)} actions={<button type="button" onClick={() => setCaptionContext(null)}>Đóng · giữ bản nháp</button>}>
      <div className="pq-caption-editor"><div className="pq-caption-media"><div className="pq-large-preview"><PublishMedia bundle={captionBundle} index={Math.min(photo, Math.max(0, captionBundle.images.length - 1))} expanded/></div>
        {captionBundle.mediaKind === "image" && captionBundle.images.length > 0 && <div className="pq-photo-nav"><button type="button" aria-label="Ảnh trước" disabled={photo === 0} onClick={() => setPhoto(n => n - 1)}><ArrowLeft size={16}/></button><span>{Math.min(photo + 1, captionBundle.images.length)} / {captionBundle.images.length}</span><button type="button" aria-label="Ảnh tiếp" disabled={photo + 1 >= captionBundle.images.length} onClick={() => setPhoto(n => n + 1)}><ArrowRight size={16}/></button></div>}
        {captionBundle.mediaKind === "video" && <p className="pq-hint">{captionBundle.video?.fileName ?? "Video MP4"}{captionBundle.video ? ` · ${(captionBundle.video.durationMs / 1000).toFixed(1)} giây` : ""}. Video giữ nguyên trong nguồn; khung này không phát video.</p>}
      </div><div><label className="pq-field"><span>Nội dung bài đăng</span><textarea aria-label="Nội dung bài đăng" rows={5} value={p.captions[captionBundle.id] ?? captionBundle.caption} disabled={locked} onChange={e => {
        const current = latest.current;
        if (current && !current.locked && current.props.active !== false && current.props.sourceRoot === captionContext.source && current.props.manifest?.bundles.some(b => b.id === captionContext.id)) current.props.onCaption(captionContext.id, e.target.value);
      }}/></label><small className="pq-char-count">{(p.captions[captionBundle.id] ?? captionBundle.caption).length} ký tự · lưu trong bản nháp, không sửa file nguồn</small>
        <div className="pq-partners"><strong>Đối tác của bài</strong><p>{captionBundle.partners?.length ? captionBundle.partners.join(" · ") : "Không có thông tin đối tác trong file nguồn"}</p><small>Người đăng trên Sheet: bot</small></div>
        <div className="pq-sound"><Music2 size={20}/><div><strong>Nhạc thịnh hành</strong><small>Chọn và xác nhận nhạc trong TikTok khi đăng</small></div></div>
        {duplicateCaptions > 0 && <p className="pq-hint">{duplicateCaptions} bài đã chọn có caption trùng; nội dung giữ nguyên, không tự sửa.</p>}
      </div></div>
    </PublishDialog>}
    {dialog === "check" && <PublishDialog title="Kiểm tra đợt đăng" wide onClose={() => setDialog(null)} actions={<><button type="button" onClick={() => setDialog(null)}>Quay lại</button><button type="button" className="primary" disabled={locked || !p.preflight?.canExecute || !complete} onClick={() => void p.onExecute()}><Check size={16}/> Xác nhận đăng {selected.length} bài</button></>}>
      {p.preflightLoading ? <p>Đang kiểm tra nội dung và máy thực hiện…</p> : p.preflightError ? <p role="alert">{p.preflightError}</p> : p.preflight ? <PublishPreflightResult report={p.preflight} machineName={label} bundleName={id => bundles.find(bundle => bundle.id === id)?.name ?? id} page={reportPage} onPage={setReportPage} onRetry={() => void p.onPreflight()} busy={locked}/> : <p>Chưa có kết quả kiểm tra.</p>}
      <p className="pq-hint">Nhạc được chọn sau khi mở TikTok. {p.sheet ? "Ghi Sheet đang bật; link được ghi sau khi xác nhận bài đăng thành công." : "Ghi Sheet đang tắt; kết quả chỉ lưu trong ứng dụng."}</p>
    </PublishDialog>}
  </div>;
}
