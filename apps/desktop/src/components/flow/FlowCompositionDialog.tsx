import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { flowGet, flowValidate } from "../../api";
import { newFlowDocument } from "../../flow/model";
import { normalizeFlowIssues } from "../../flow/validation";
import { applyFlowComposition, parseCompositionBindings, parseCompositionBody } from "../../flowComposition";
import { MAX_FLOW_IMPORT_BYTES } from "../../flowImport";
import type { FlowDocumentV2, FlowNode, FlowSummary } from "../../types";
import { useModalFocus } from "../useModalFocus";

export function FlowCompositionDialog({ document, flows, editingNode, onApply, onClose }: { document: FlowDocumentV2; flows: FlowSummary[]; editingNode: FlowNode | null; onApply: (document: FlowDocumentV2) => void; onClose: () => void }) {
  const initialBody = editingNode?.config.document ?? newFlowDocument("Flow con");
  const [raw, setRaw] = useState(() => JSON.stringify(initialBody, null, 2));
  const [kind, setKind] = useState<"subflow" | "repeat">(editingNode?.kind === "repeat" ? "repeat" : "subflow");
  const [count, setCount] = useState(String(editingNode?.config.count ?? 2));
  const [inputs, setInputs] = useState(() => JSON.stringify(editingNode?.config.inputs ?? {}, null, 2));
  const [outputs, setOutputs] = useState(() => JSON.stringify(editingNode?.config.outputs ?? {}, null, 2));
  const [sourceId, setSourceId] = useState("");
  const [revision, setRevision] = useState("");
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
    void run(async () => { const generation = sequence.current; const record = await flowGet(sourceId, Number(revision)); if (!live.current || generation !== sequence.current) return; if (!record || record.document.id !== sourceId || record.document.revision !== Number(revision)) throw new Error("Phiên bản đã chọn không tồn tại."); setRaw(JSON.stringify(record.document, null, 2)); });
  };
  const apply = () => void run(async () => {
    const generation = sequence.current;
    const next = applyFlowComposition(document, { kind, document: parseCompositionBody(raw), count: Number(count), inputs: parseCompositionBindings(inputs), outputs: parseCompositionBindings(outputs) }, editingNode?.id ?? null);
    await flowValidate(next);
    if (live.current && generation === sequence.current) {
      if (JSON.stringify(currentDocument.current) !== JSON.stringify(document)) throw new Error("Flow cha đã thay đổi. Kiểm tra lại nội dung rồi áp dụng.");
      onApply(next);
    }
  });
  return <section ref={ref} tabIndex={-1} role="dialog" aria-modal="true" aria-label="Ghép Flow con" className="flow-dialog">
    <header><strong>{editingNode ? "Chỉnh Flow con" : "Ghép Flow con"}</strong><button type="button" aria-label="Đóng ghép Flow" onClick={close}><X size={16} /></button></header>
    <label className="flow-field"><span>Cách thực hiện</span><select value={kind} onChange={(event) => { changed(); setKind(event.currentTarget.value as typeof kind); }}><option value="subflow">Chạy Flow con một lần</option><option value="repeat">Lặp Flow con</option></select></label>
    {kind === "repeat" && <label className="flow-field"><span>Số lượt Flow con</span><input type="number" min={1} max={50} step={1} value={count} onChange={(event) => { changed(); setCount(event.currentTarget.value); }} /></label>}
    <label className="flow-field"><span>Flow đã lưu</span><select value={sourceId} onChange={(event) => { changed(); setSourceId(event.currentTarget.value); setRevision(String(flows.find((flow) => flow.id === event.currentTarget.value)?.latestRevision ?? "")); }}><option value="">Chọn Flow</option>{flows.map((flow) => <option value={flow.id} key={flow.id}>{flow.name}</option>)}</select></label>
    <label className="flow-field"><span>Phiên bản nguồn</span><input type="number" min={1} step={1} value={revision} onChange={(event) => { changed(); setRevision(event.currentTarget.value); }} /></label>
    <button type="button" disabled={busy || !sourceId} onClick={load}>Nạp đúng phiên bản</button>
    <label className="flow-field"><span>JSON Flow con</span><textarea rows={10} spellCheck={false} value={raw} onChange={(event) => { changed(); setRaw(event.currentTarget.value); }} /></label>
    <label className="flow-field"><span>Nạp tệp Flow con (tối đa 1 MiB)</span><input type="file" accept=".json,application/json" onChange={(event) => { const file = event.currentTarget.files?.[0]; event.currentTarget.value = ""; if (!file) return; void run(async () => { const generation = sequence.current; if (file.size > MAX_FLOW_IMPORT_BYTES) throw new Error("Tệp vượt 1 MiB."); const text = await file.text(); if (live.current && generation === sequence.current) setRaw(text); }); }} /></label>
    <p>Nội dung trên được lưu nguyên trong bước này. Đổi Flow nguồn sau đó sẽ không thay phiên bản đang dùng. Có thể chỉnh nội dung JSON ngay tại đây.</p>
    <label className="flow-field"><span>Biến đầu vào (biến con: biến cha)</span><textarea rows={3} spellCheck={false} value={inputs} onChange={(event) => { changed(); setInputs(event.currentTarget.value); }} /></label>
    <label className="flow-field"><span>Biến đầu ra (biến cha: biến con)</span><textarea rows={3} spellCheck={false} value={outputs} onChange={(event) => { changed(); setOutputs(event.currentTarget.value); }} /></label>
    <p>Mỗi lần gọi có biến riêng; ánh xạ đầu vào và đầu ra truyền dữ liệu giữa các lượt. Tối đa 8 cấp và 2.000 bước sau khi mở rộng.</p>
    {error && <p role="alert">{error}</p>}
    <footer><button type="button" onClick={close}>Hủy</button><button type="button" disabled={busy} onClick={apply}>{busy ? "Đang kiểm tra…" : "Áp dụng Flow con"}</button></footer>
  </section>;
}
