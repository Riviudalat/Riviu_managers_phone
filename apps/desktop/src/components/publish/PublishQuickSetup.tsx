import { useEffect, useMemo, useState } from "react";
import { ArrowLeft, ArrowRight, Check, FolderOpen, Image, Music2, Search, Undo2, Zap, X } from "lucide-react";
import type { PublishWizardProps } from "./PublishWizard";
import { PublishMedia } from "./PublishMedia";
import { PublishDialog } from "./PublishDialog";
import { PublishPreflightResult } from "./PublishPreflightResult";
import { MachineChoice } from "../MachineChoice";
import { orderDevicesByNumber, tileName, tileNumber } from "../../deviceNaming";
import { pickDirectory } from "../../pickFile";
import { describeError } from "../../describeError";
import { assignDevice } from "./publishAssignments";
import "../../styles/publish-quick.css";
import { allocateQuickPosts } from "./publishQuickAllocation";
import { publishSelectionStatus } from "./publishSelectionStatus";

/** Concept 01: source, caption and devices share one screen; PublishPage owns effects. */
export function PublishQuickSetup(p: PublishWizardProps & { blockingReason?: string; onAssignmentChange?: (ids: string[], assignments: Record<string, string>) => void }) {
  const [query, setQuery] = useState("");
  const [deviceQuery, setDeviceQuery] = useState("");
  const [activeId, setActiveId] = useState<string>();
  const [photo, setPhoto] = useState(0);
  const [dialog, setDialog] = useState<"preview" | "check" | null>(null);
  const [error, setError] = useState("");
  const [picked, setPicked] = useState<string[]>([]);
  const [notice, setNotice] = useState("");
  const [undo, setUndo] = useState<{ source: string; ids: string[]; assignments: Record<string, string>; picked: string[]; after: string } | null>(null);
  const [reportPage, setReportPage] = useState(0);
  const bundles = p.manifest?.bundles ?? [];
  const visibleBundles = bundles.filter(b => `${b.name} ${p.captions[b.id] ?? b.caption}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
  const selected = bundles.filter(b => p.selectedIds.includes(b.id));
  const active = bundles.find(b => b.id === activeId) ?? selected[0] ?? bundles[0];
  const devices = useMemo(() => orderDevicesByNumber(p.devices, p.metas), [p.devices, p.metas]);
  const ready = devices.filter(d => d.status === "ready" && p.eligible.includes(d.udid));
  const selection = publishSelectionStatus({ selectedIds: p.selectedIds, bundles, assignments: p.assignments,
    captions: p.captions, eligible: p.eligible, ready: ready.map(d => d.udid), blockingReason: p.blockingReason });
  const mapped = selection.mapped;
  const locked = p.busy || p.scanning || p.preflightLoading;
  const complete = selection.ready;
  const captionsValid = selection.ready;
  const checkReason = p.scanning ? "Đang quét và xác nhận nội dung…" : p.preflightLoading ? "Đang kiểm tra đợt đăng…" : p.busy ? "Đang xử lý tác vụ…" : selection.reason;
  useEffect(() => { if (p.active === false) setDialog(null); }, [p.active]);
  const select = (id: string, checked: boolean) => p.onSelect(checked ? [...p.selectedIds, id] : p.selectedIds.filter(x => x !== id));
  const mappingKey = JSON.stringify([p.selectedIds, p.selectedIds.map(id => p.assignments[id] ?? "")]);
  const applyAssignment = (ids: string[], assignments: Record<string, string>) => {
    if (p.onAssignmentChange) p.onAssignmentChange(ids, assignments);
    else { p.onSelect(ids); p.onAssign(assignments); }
  };
  const autoAssign = () => {
    const next = allocateQuickPosts({ sourceIds: bundles.map(b => b.id), selectedIds: p.selectedIds,
      assignments: p.assignments, eligibleIds: p.eligible, readyIds: ready.map(d => d.udid), picked });
    const after = JSON.stringify([next.ids, next.ids.map(id => next.assignments[id] ?? "")]);
    if (after !== mappingKey) {
      setUndo({ source: p.sourceRoot, ids: p.selectedIds, assignments: p.assignments, picked, after });
      applyAssignment(next.ids, next.assignments);
    }
    setPicked(next.picked);
    setError("");
    setNotice(next.ids.length ? `Đã gán ${next.ids.length} bài cho ${next.ids.length} máy. ${bundles.length - next.ids.length} bài còn lại để đợt sau.` : "Chưa có máy sẵn sàng trong lựa chọn. Kiểm tra kết nối hoặc chọn lại máy.");
  };
  const undoAssignment = () => {
    if (!undo || undo.source !== p.sourceRoot || undo.after !== mappingKey) return;
    applyAssignment(undo.ids, undo.assignments); setPicked(undo.picked); setUndo(null); setNotice("Đã hoàn tác gán nhanh.");
  };
  const chooseFolder = async () => { try { const path = await pickDirectory(); if (path) p.onSource(path); } catch (e) { setError(describeError(e)); } };
  const deviceIndex = new Map(devices.map((d, i) => [d.udid, { device: d, index: i }]));
  const assignedByDevice = new Map(selected.map(b => [p.assignments[b.id], b]));
  const readyIds = new Set(ready.map(d => d.udid));
  const label = (udid: string) => {
    const found = deviceIndex.get(udid);
    return found ? `Máy ${tileNumber(found.index + 1, p.metas.get(udid))} · ${tileName(found.device, p.metas.get(udid))}` : "Chưa ghép máy";
  };
  const clearDevice = (udid: string) => p.onAssign(Object.fromEntries(Object.entries(p.assignments).filter(([,id]) => id !== udid)));
  return <div className="publish-quick" hidden={p.active === false}>
    <div className="pq-setup-tools">
      <div className="pq-source"><label htmlFor="publish-source-folder">Thư mục bài đăng</label><div className="pq-source-controls"><input id="publish-source-folder" aria-label="Thư mục nguồn" value={p.sourceRoot} onChange={e => p.onSource(e.target.value)} disabled={p.busy} placeholder="Đường dẫn thư mục chứa bài đăng"/><button type="button" disabled={p.busy} onClick={() => void chooseFolder()}><FolderOpen size={16}/>Chọn thư mục</button><button type="button" disabled={locked || !p.sourceRoot} onClick={() => void p.onScan(p.sourceRoot)}>{p.scanning ? "Đang quét…" : "Quét"}</button></div></div>
      {p.settings}
    </div>
    {p.blockingReason && <p className="pq-blocking-reason" role="status">{p.blockingReason}</p>}
    {error && <div className="pq-error" role="alert">{error}<button type="button" aria-label="Đóng lỗi" onClick={() => setError("")}><X size={14}/></button></div>}
    <div className="pq-columns">
      <section className="pq-library pq-panel" aria-label="Nội dung đăng">
        <header><h2>Nội dung</h2><span>{bundles.length} bài</span></header>
        <label className="pq-search"><Search size={15}/><input aria-label="Tìm bài đăng" placeholder="Tìm bài đăng" value={query} onChange={e => setQuery(e.target.value)}/></label>
        <div className="pq-tools"><button type="button" className="ghost" disabled={locked || !bundles.length} onClick={() => p.onSelect(bundles.map(b => b.id))}>Chọn tất cả bài</button><button type="button" className="ghost" disabled={locked} onClick={() => p.onSelect([])}>Bỏ chọn</button><small>{selected.length} đã chọn</small></div>
        <div className="pq-posts">{visibleBundles.map(b => <article key={b.id} className={b.id === active?.id ? "is-active" : ""}>
          <input type="checkbox" aria-label={`Chọn ${b.name}`} checked={p.selectedIds.includes(b.id)} disabled={locked} onChange={e => select(b.id,e.target.checked)}/>
          <button type="button" className="pq-post-pick" onClick={() => {setActiveId(b.id);setPhoto(0);}}><PublishMedia bundle={b}/><span><strong>{b.name}</strong><small>{b.mediaKind === "video" ? "Video MP4" : `${b.images.length} ảnh`} · {p.assignments[b.id] ? label(p.assignments[b.id]) : "Chưa ghép máy"}</small></span></button>
        </article>)}
          {!visibleBundles.length && <p className="pq-list-empty">{bundles.length ? "Không có bài khớp với từ khóa." : "Chọn thư mục ở trên, rồi bấm Quét để xem bài đăng."}</p>}
        </div>
        <footer>{p.scanning ? "Đang đọc nội dung…" : `${bundles.length} bài trong thư mục`}</footer>
      </section>
      <section className="pq-editor pq-panel" aria-label="Bài đang chỉnh">
        <header><h2>Bài đang chỉnh</h2><span>{active?.mediaKind === "video" ? "Video" : "Bộ ảnh"}</span></header>
        {active ? <div className="pq-editor-scroll">
          <div className="pq-active-title"><PublishMedia bundle={active}/><div><h3>{active.name}</h3><small>{active.images.length} ảnh · {(active.totalBytes/1048576).toFixed(1)} MB</small></div><button type="button" className="ghost" aria-label="Phóng to ảnh" onClick={() => setDialog("preview")}><Image size={18}/></button></div>
          <label className="pq-field"><span>Nội dung bài đăng</span><textarea aria-label="Nội dung bài đăng" rows={5} value={p.captions[active.id] ?? active.caption} disabled={locked} onChange={e => p.onCaption(active.id,e.target.value)}/></label>
          <small className="pq-char-count">{(p.captions[active.id] ?? active.caption).length} ký tự · lưu trong bản nháp</small>
          <div className="pq-sound"><Music2 size={20}/><div><strong>Nhạc thịnh hành</strong><small>Chọn và xác nhận nhạc trong TikTok khi đăng</small></div></div>
          <label className="pq-field"><span>Máy nhận bài này</span><select aria-label="Máy nhận bài đang chỉnh" disabled={locked || !p.selectedIds.includes(active.id)} value={p.assignments[active.id] ?? ""} onChange={e => { if (e.target.value) p.onAssign(assignDevice(p.assignments,active.id,e.target.value)); else p.onAssign(Object.fromEntries(Object.entries(p.assignments).filter(([id]) => id !== active.id))); }}><option value="">Chọn máy</option>{ready.map(d => <option key={d.udid} value={d.udid}>{label(d.udid)}</option>)}</select></label>
          <div className="pq-partners"><strong>Đối tác của bài</strong><p>{active.partners?.length ? active.partners.join(" · ") : "Không có thông tin đối tác trong file nguồn"}</p><small>Người đăng trên Sheet: bot</small></div>
          <div className="pq-options"><label><input type="checkbox" checked={p.sheet} disabled={locked} onChange={e => p.onSheet(e.target.checked)}/> Ghi kết quả lên Sheet</label><label><input type="checkbox" checked={p.cleanup} disabled={locked} onChange={e => p.onCleanup(e.target.checked)}/> Xóa bản chuyển sau khi đăng thành công</label></div>
        </div> : <div className="pq-empty"><Image size={34}/><strong>Nội dung và đối tác hiện tại đây</strong><p>Chọn một bài trong danh sách bên trái.</p></div>}
      </section>
      <section className="pq-devices pq-panel" aria-label="Máy thực hiện">
        <header><h2>Máy thực hiện</h2><span>{new Set([...picked,...selected.map(b=>p.assignments[b.id]).filter(Boolean)]).size} / {devices.length}</span></header>
        <div className="pq-quick-actions"><button type="button" className="pq-quick-button" disabled={locked || !bundles.length} onClick={autoAssign}><Zap size={16} aria-hidden="true"/>Chọn nhanh</button><button type="button" aria-label="Hoàn tác gán nhanh" disabled={locked || !undo || undo.source !== p.sourceRoot || undo.after !== mappingKey} onClick={undoAssignment}><Undo2 size={16} aria-hidden="true"/>Hoàn tác</button></div>
        {notice && <p className="pq-hint" role="status">{notice}</p>}
        <label className="pq-search"><Search size={15}/><input aria-label="Tìm số máy" value={deviceQuery} onChange={e => setDeviceQuery(e.target.value)} placeholder="Tìm số máy"/></label>
        <div className="pq-tools"><button type="button" className="ghost" disabled={locked} onClick={() => setPicked(ready.map(d => d.udid))}>Chọn tất cả</button><button type="button" className="ghost" disabled={locked} onClick={() => {setPicked([]);p.onAssign({});}}>Bỏ chọn</button>{p.scopeControl}</div>
        {!p.eligible.length && <p className="pq-hint">Chọn Toàn bộ máy hoặc một nhóm để ghép bài.</p>}
        <div className="pq-machine-grid machine-choice-grid">{devices.filter(d => `${label(d.udid)} ${p.metas.get(d.udid)?.handle ?? ""}`.toLocaleLowerCase().includes(deviceQuery.toLocaleLowerCase())).map(d => {
          const i=deviceIndex.get(d.udid)!.index, assigned=assignedByDevice.get(d.udid),checked=picked.includes(d.udid)||Boolean(assigned);
          return <MachineChoice key={d.udid} number={tileNumber(i+1,p.metas.get(d.udid))} name={tileName(d,p.metas.get(d.udid))} status={d.status} label={`Chọn ${label(d.udid)}`} checked={checked} disabled={locked || (!checked && !readyIds.has(d.udid))} onChange={value=>{setPicked(value?[...new Set([...picked,d.udid])]:picked.filter(id=>id!==d.udid));if(!value)clearDevice(d.udid);}} detail={assigned?<span title={assigned.name}>{assigned.name}</span>:undefined}/>;
        })}</div>
        <footer><small>{ready.length} máy sẵn sàng trong phạm vi</small></footer>
      </section>
    </div>
    <footer className="pq-footer"><div><strong>{selected.length} bài đã chọn</strong><span>{mapped}/{selected.length} bài có máy · mỗi máy một bài · Sheet {p.sheet ? "bật" : "tắt"}</span>{checkReason && <small id="publish-check-reason" role="status">{checkReason}</small>}</div><button type="button" className="primary" aria-describedby={checkReason ? "publish-check-reason" : undefined} disabled={locked || !complete || !captionsValid} onClick={()=>{setReportPage(0);setDialog("check");void p.onPreflight();}}>{p.preflightLoading?"Đang kiểm tra…":"Kiểm tra & đăng"}<ArrowRight size={16}/></button></footer>
    {dialog==="preview" && active && <PublishDialog title={`Xem trước · ${active.name}`} onClose={()=>setDialog(null)}><div className="pq-large-preview"><PublishMedia bundle={active} index={Math.min(photo,Math.max(0,active.images.length-1))} expanded/></div><div className="pq-photo-nav"><button type="button" aria-label="Ảnh trước" disabled={photo===0} onClick={()=>setPhoto(photo-1)}><ArrowLeft size={16}/></button><span>{photo+1} / {active.images.length}</span><button type="button" aria-label="Ảnh tiếp" disabled={photo+1>=active.images.length} onClick={()=>setPhoto(photo+1)}><ArrowRight size={16}/></button></div></PublishDialog>}
    {dialog==="check" && <PublishDialog title="Kiểm tra đợt đăng" wide onClose={()=>setDialog(null)} actions={<><button type="button" onClick={()=>setDialog(null)}>Quay lại</button><button type="button" className="primary" disabled={locked || !p.preflight?.canExecute || !complete} onClick={()=>void p.onExecute()}><Check size={16}/> Xác nhận đăng {selected.length} bài</button></>}>
      {p.preflightLoading?<p>Đang kiểm tra nội dung và máy thực hiện…</p>:p.preflightError?<p role="alert">{p.preflightError}</p>:p.preflight?<PublishPreflightResult report={p.preflight} machineName={label} bundleName={id => bundles.find(bundle => bundle.id === id)?.name ?? id} page={reportPage} onPage={setReportPage} onRetry={() => void p.onPreflight()} busy={locked}/>:<p>Chưa có kết quả kiểm tra.</p>}
      <p className="pq-hint">Nhạc được chọn sau khi mở TikTok. {p.sheet ? "Ghi Sheet đang bật; link được ghi sau khi xác nhận bài đăng thành công." : "Ghi Sheet đang tắt; kết quả chỉ lưu trong ứng dụng."}</p>
    </PublishDialog>}
  </div>;
}
