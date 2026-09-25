import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { flowGet, flowLibraryGet, flowLibraryPublish, flowLibraryUnpublish, flowValidate } from "../../api";
import { newFlowDocument } from "../../flow/model";
import { normalizeFlowIssues } from "../../flow/validation";
import { applyFlowComposition, parseCompositionBindings, parseCompositionBody } from "../../flowComposition";
import { MAX_FLOW_IMPORT_BYTES } from "../../flowImport";
import type { FlowDocumentV2, FlowNode, FlowRevisionRecord, FlowSummary } from "../../types";
import { useModalFocus } from "../useModalFocus";

export function FlowCompositionDialog({ document, flows, editingNode, onApply, onClose }: { document: FlowDocumentV2; flows: FlowSummary[]; editingNode: FlowNode | null; onApply: (document: FlowDocumentV2) => void; onClose: () => void }) {
  const initialBody = editingNode?.config.document ?? newFlowDocument("Flow con");
  const initialLibrary = editingNode?.config.library;
  const initialSourceId = initialLibrary && typeof initialLibrary === "object" && !Array.isArray(initialLibrary) && typeof initialLibrary.flowId === "string" ? initialLibrary.flowId : "";
  const [sourceMode, setSourceMode] = useState<"snapshot" | "published">(initialSourceId ? "published" : "snapshot");
  const [raw, setRaw] = useState(() => JSON.stringify(initialBody, null, 2));
  const [kind, setKind] = useState<"subflow" | "repeat">(editingNode?.kind === "repeat" ? "repeat" : "subflow");
  const [count, setCount] = useState(String(editingNode?.config.count ?? 2));
  const [inputs, setInputs] = useState(() => JSON.stringify(editingNode?.config.inputs ?? {}, null, 2));
  const [outputs, setOutputs] = useState(() => JSON.stringify(editingNode?.config.outputs ?? {}, null, 2));
  const [sourceId, setSourceId] = useState(initialSourceId);
  const [revision, setRevision] = useState("");
  const [loadedSource, setLoadedSource] = useState<FlowRevisionRecord | null>(null);
  const [publishedSource, setPublishedSource] = useState<FlowRevisionRecord | null>(null);
  const [publicationLoaded, setPublicationLoaded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sequence = useRef(0);
  const live = useRef(true);
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  const currentDocument = useRef(document);
  currentDocument.current = document;
  const close = () => { live.current = false; sequence.current++; onClose(); };
  const ref = useModalFocus<HTMLElement>(close);
  const changed = () => { sequence.current++; setBusy(false); setError(null); };
  const run = async (action: () => Promise<void>) => { const generation = ++sequence.current; setBusy(true); setError(null); try { await action(); } catch (reason) { if (live.current && generation === sequence.current) setError(normalizeFlowIssues(reason).map((issue) => issue.message).join(" · ")); } finally { if (live.current && generation === sequence.current) setBusy(false); } };
  const load = () => {
    if (!sourceId || !Number.isSafeInteger(Number(revision)) || Number(revision) < 1) { setError("Chọn Flow và phiên bản đã lưu từ 1 trở lên."); return; }
    void run(async () => { const generation = sequence.current; const [record, published] = await Promise.all([flowGet(sourceId, Number(revision)), flowLibraryGet(sourceId)]); if (!live.current || generation !== sequence.current) return; if (!record || record.document.id !== sourceId || record.document.revision !== Number(revision)) throw new Error("Phiên bản đã chọn không tồn tại."); setRaw(JSON.stringify(record.document, null, 2)); setLoadedSource(record); setPublishedSource(published); setPublicationLoaded(true); });
  };
  const loadPublished = () => void run(async () => {
    const generation = sequence.current;
    const record = await flowLibraryGet(sourceId);
    if (!live.current || generation !== sequence.current) return;
    setPublishedSource(record); setPublicationLoaded(true);
    if (!record || record.document.id !== sourceId) throw new Error("Flow này chưa có phiên bản được công bố.");
    setLoadedSource(record); setRevision(String(record.document.revision)); setRaw(JSON.stringify(record.document, null, 2));
  });
  const publish = () => void run(async () => {
    const generation = sequence.current;
    if (!loadedSource || loadedSource.document.id !== sourceId || !publicationLoaded || raw !== JSON.stringify(loadedSource.document, null, 2)) throw new Error("Nạp đúng phiên bản đã lưu trước khi công bố.");
    const published = await flowLibraryPublish(sourceId, loadedSource.document.revision, publishedSource?.document.revision ?? null);
    if (live.current && generation === sequence.current) { setPublishedSource(published); setLoadedSource(published); setRaw(JSON.stringify(published.document, null, 2)); setRevision(String(published.document.revision)); }
  });
  const unpublish = () => void run(async () => {
    const generation = sequence.current;
    if (!publishedSource || publishedSource.document.id !== sourceId) throw new Error("Flow nguồn chưa được công bố.");
    await flowLibraryUnpublish(sourceId, publishedSource.document.revision);
    if (live.current && generation === sequence.current) setPublishedSource(null);
  });
  const apply = () => void run(async () => {
    const generation = sequence.current;
    const body = parseCompositionBody(raw);
    if (sourceMode === "published" && (!sourceId || body.id !== sourceId || !publishedSource || publishedSource.document.revision !== body.revision || JSON.stringify(publishedSource.document) !== JSON.stringify(body))) throw new Error("Nạp bản đã công bố trước khi liên kết Flow dùng chung.");
    const next = applyFlowComposition(document, { kind, document: body, count: Number(count), inputs: parseCompositionBindings(inputs), outputs: parseCompositionBindings(outputs), ...(sourceMode === "published" ? { library: { flowId: sourceId, channel: "published" as const } } : {}) }, editingNode?.id ?? null);
    await flowValidate(next);
    if (live.current && generation === sequence.current) {
      if (JSON.stringify(currentDocument.current) !== JSON.stringify(document)) throw new Error("Flow cha đã thay đổi. Kiểm tra lại nội dung rồi áp dụng.");
      onApply(next);
    }
  });
  return <section ref={ref} tabIndex={-1} role="dialog" aria-modal="true" aria-label="Ghép Flow con" className="flow-dialog">
    <header><strong>{editingNode ? "Chỉnh Flow con" : "Ghép Flow con"}</strong><button type="button" aria-label="Đóng ghép Flow" onClick={close}><X size={16} /></button></header>
    <label className="flow-field"><span>Nguồn Flow con</span><select value={sourceMode} onChange={(event) => { changed(); setSourceMode(event.currentTarget.value as typeof sourceMode); }}><option value="snapshot">Bản sao cố định</option><option value="published">Bản công bố dùng chung</option></select></label>
    <label className="flow-field"><span>Cách thực hiện</span><select value={kind} onChange={(event) => { changed(); setKind(event.currentTarget.value as typeof kind); }}><option value="subflow">Chạy Flow con một lần</option><option value="repeat">Lặp Flow con</option></select></label>
    {kind === "repeat" && <label className="flow-field"><span>Số lượt Flow con</span><input type="number" min={1} max={50} step={1} value={count} onChange={(event) => { changed(); setCount(event.currentTarget.value); }} /></label>}
    <label className="flow-field"><span>Flow đã lưu</span><select value={sourceId} onChange={(event) => { changed(); setSourceId(event.currentTarget.value); setRevision(String(flows.find((flow) => flow.id === event.currentTarget.value)?.latestRevision ?? "")); setLoadedSource(null); setPublishedSource(null); setPublicationLoaded(false); }}><option value="">Chọn Flow</option>{flows.filter((flow) => flow.id !== document.id && !flow.archived).map((flow) => <option value={flow.id} key={flow.id}>{flow.name}</option>)}</select></label>
    <label className="flow-field"><span>Phiên bản nguồn</span><input type="number" min={1} step={1} value={revision} onChange={(event) => { changed(); setRevision(event.currentTarget.value); }} /></label>
    <button type="button" disabled={busy || !sourceId} onClick={load}>Nạp đúng phiên bản</button>
    <button type="button" disabled={busy || !sourceId} onClick={loadPublished}>Nạp bản đã công bố</button>
    <button type="button" disabled={busy || !publicationLoaded || !loadedSource || loadedSource.document.id !== sourceId || raw !== JSON.stringify(loadedSource.document, null, 2) || loadedSource.document.revision === publishedSource?.document.revision} onClick={publish}>Công bố phiên bản đã nạp</button>
    <button type="button" disabled={busy || !publishedSource || publishedSource.document.id !== sourceId} onClick={unpublish}>Bỏ công bố</button>
    {publicationLoaded && <p role="status">{publishedSource ? `Bản công bố hiện tại: ${publishedSource.document.revision}.` : "Flow nguồn chưa được công bố."} Công bố cập nhật các lượt chạy mới có liên kết tới Flow này; lượt đang chạy giữ nguyên phiên bản.</p>}
    <label className="flow-field"><span>JSON Flow con</span><textarea rows={10} spellCheck={false} readOnly={sourceMode === "published"} value={raw} onChange={(event) => { changed(); setRaw(event.currentTarget.value); }} /></label>
    {sourceMode === "snapshot" && <label className="flow-field"><span>Nạp tệp Flow con (tối đa 1 MiB)</span><input type="file" accept=".json,application/json" onChange={(event) => { const file = event.currentTarget.files?.[0]; event.currentTarget.value = ""; if (!file) return; void run(async () => { const generation = sequence.current; if (file.size > MAX_FLOW_IMPORT_BYTES) throw new Error("Tệp vượt 1 MiB."); const text = await file.text(); if (live.current && generation === sequence.current) setRaw(text); }); }} /></label>}
    <p>{sourceMode === "published" ? "Lượt chạy mới lấy bản công bố của Flow nguồn và chốt toàn bộ các bước trước khi chạy. Muốn sửa nguồn, mở Flow đó, lưu phiên bản mới rồi công bố." : "Nội dung trên được lưu nguyên trong bước này. Đổi Flow nguồn sau đó sẽ không thay phiên bản đang dùng. Có thể chỉnh nội dung JSON ngay tại đây."}</p>
    <label className="flow-field"><span>Biến đầu vào (biến con: biến cha)</span><textarea rows={3} spellCheck={false} value={inputs} onChange={(event) => { changed(); setInputs(event.currentTarget.value); }} /></label>
    <label className="flow-field"><span>Biến đầu ra (biến cha: biến con)</span><textarea rows={3} spellCheck={false} value={outputs} onChange={(event) => { changed(); setOutputs(event.currentTarget.value); }} /></label>
    <p>Mỗi lần gọi có biến riêng; ánh xạ đầu vào và đầu ra truyền dữ liệu giữa các lượt. Tối đa 8 cấp và 2.000 bước sau khi mở rộng.</p>
    {error && <p role="alert">{error}</p>}
    <footer><button type="button" onClick={close}>Hủy</button><button type="button" disabled={busy} onClick={apply}>{busy ? "Đang kiểm tra…" : "Áp dụng Flow con"}</button></footer>
  </section>;
}
