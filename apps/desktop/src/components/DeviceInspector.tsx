import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronUp, Circle, Copy, RefreshCw, Save, Scan, Square, X } from "lucide-react";
import { createPortal } from "react-dom";

import {
  inspectorConfirmPostcondition,
  inspectorObserve,
  inspectorRecord,
  inspectorRecording,
  inspectorTap,
} from "../inspectorApi";
import type { InspectorElement, InspectorRecording, InspectorSnapshot } from "../inspectorApi";
import { flowSaveRevision } from "../api";
import { createFlowNode, newFlowDocument } from "../flow/model";
import type { JsonObject } from "../types";
import { describeError } from "../describeError";
import { useClosingTransition } from "./useClosingTransition";
import { useModalFocus } from "./useModalFocus";
import "./device-inspector.css";

function elementLabel(element: InspectorElement) {
  return element.text || element.description || element.resourceId || element.className || `Phần tử ${element.index}`;
}

function elementDepth(elements: InspectorElement[], index: number) {
  const byIndex = new Map(elements.map((element) => [element.index, element]));
  let current = byIndex.get(index)?.parent ?? null;
  let depth = 1;
  const seen = new Set<number>();
  while (current !== null && depth < 32 && !seen.has(current)) {
    seen.add(current);
    depth += 1;
    current = byIndex.get(current)?.parent ?? null;
  }
  return depth;
}

export function DeviceInspector({ udid, onClose }: { udid: string; onClose: () => void }) {
  const { closing, close } = useClosingTransition(onClose);
  const ref = useModalFocus<HTMLDivElement>(close);
  const [snapshot, setSnapshot] = useState<InspectorSnapshot | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const [record, setRecord] = useState<InspectorRecording | null>(null);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [name, setName] = useState("Quy trình mới");
  const [saved, setSaved] = useState("");

  const refreshRecord = useCallback(async () => setRecord(await inspectorRecording(udid)), [udid]);
  const observe = useCallback(async () => {
    setBusy(true); setError("");
    try {
      setSnapshot(await inspectorObserve(udid));
      setSelected(null); setHover(null);
      await refreshRecord();
    } catch (reason) { setError(describeError(reason)); }
    finally { setBusy(false); }
  }, [refreshRecord, udid]);
  useEffect(() => { void observe(); }, [observe]);

  const element = snapshot?.elements.find((candidate) => candidate.index === selected);
  const outlined = snapshot?.elements.find((candidate) => candidate.index === (hover ?? selected));
  const pending = record?.steps.at(-1);
  const awaitingPostcondition = Boolean(
    pending && !pending.verified && pending.afterId === snapshot?.id && pending.error === "Chờ chọn phần tử kết quả",
  );
  const visibleElements = useMemo(() => {
    if (!snapshot) return [];
    const needle = query.trim().toLocaleLowerCase();
    return snapshot.elements.filter((candidate) => {
      if (!candidate.selector && !candidate.text && !candidate.description && !candidate.clickable) return false;
      if (!needle) return true;
      return [candidate.text, candidate.description, candidate.resourceId, candidate.className]
        .some((value) => value.toLocaleLowerCase().includes(needle));
    });
  }, [query, snapshot]);

  const hit = (event: React.PointerEvent<SVGSVGElement>) => {
    if (!snapshot) return null;
    const rect = event.currentTarget.getBoundingClientRect();
    const x = (event.clientX - rect.left) * snapshot.width / rect.width;
    const y = (event.clientY - rect.top) * snapshot.height / rect.height;
    return snapshot.elements
      .filter((candidate) => x >= candidate.x && x <= candidate.x + candidate.width && y >= candidate.y && y <= candidate.y + candidate.height)
      .sort((left, right) => (left.width * left.height) - (right.width * right.height))[0]?.index ?? null;
  };
  const toggleRecord = async () => {
    setError("");
    try { setRecord(await inspectorRecord(udid, name, !record?.active)); setSaved(""); }
    catch (reason) { setError(describeError(reason)); }
  };
  const tap = async () => {
    if (!element?.selector) return;
    setBusy(true); setError("");
    try { setSnapshot(await inspectorTap(udid, element.selector)); setSelected(null); }
    catch (reason) { setError(describeError(reason)); }
    finally { await refreshRecord(); setBusy(false); }
  };
  const confirmPostcondition = async () => {
    if (!snapshot || !element?.selector || !awaitingPostcondition) return;
    setBusy(true); setError("");
    try { setRecord(await inspectorConfirmPostcondition(udid, snapshot.id, element.selector)); }
    catch (reason) { setError(describeError(reason)); }
    finally { setBusy(false); }
  };
  const save = async () => {
    if (!record || record.active || !record.steps.length) return;
    setBusy(true); setError("");
    try {
      if (record.steps.some((step) => !step.verified || !step.expected)) throw Error("Có bước chưa xác minh phần tử kết quả.");
      const doc = newFlowDocument(record.name);
      const start = doc.nodes.find((node) => node.kind === "start")!;
      const end = doc.nodes.find((node) => node.kind === "end")!;
      const launch = createFlowNode("launchApp", { x: 220, y: 100 });
      launch.config = { bundleId: record.steps[0].selector.package };
      launch.postcondition = { kind: "activeAppEquals", bundleId: record.steps[0].selector.package };
      const taps = record.steps.map((step, index) => {
        const node = createFlowNode("tap", { x: 440 + index * 220, y: 100 });
        node.config = { selector: step.selector as unknown as JsonObject };
        node.postcondition = { kind: "elementVisible", selector: step.expected! };
        return node;
      });
      end.position = { x: 440 + taps.length * 220, y: 100 };
      doc.nodes = [start, launch, ...taps, end];
      doc.edges = doc.nodes.slice(0, -1).map((node, index) => ({ id: crypto.randomUUID(), sourceNodeId: node.id, sourcePort: "flow", targetNodeId: doc.nodes[index + 1].id, targetPort: "flow" }));
      const revision = await flowSaveRevision(doc, null);
      setSaved(`Đã lưu Flow “${revision.document.name}”. Mở Flow thiết bị để chỉnh và chạy.`);
    } catch (reason) { setError(describeError(reason)); }
    finally { setBusy(false); }
  };

  const actionable = Boolean(element?.clickable || element?.selector?.actionTarget);
  return createPortal(<div className={`modal-backdrop inspector-backdrop${closing ? " is-closing" : ""}`}>
    <div ref={ref} tabIndex={-1} className="modal device-inspector" role="dialog" aria-label="Bắt thuộc tính và ghi Flow" aria-modal="true">
      <header><div><h2><Scan size={18}/>Bắt thuộc tính & ghi Flow</h2><small>{snapshot ? `${snapshot.package} · ${snapshot.version} · ${snapshot.locale}` : udid}</small></div><button className="icon-btn" aria-label="Đóng Inspector" onClick={close}><X size={18}/></button></header>
      <div className="inspector-toolbar"><input aria-label="Tên quy trình" value={name} disabled={record?.active} onChange={(event) => setName(event.target.value)}/><button disabled={busy} onClick={() => void observe()}><RefreshCw size={15}/>Đọc lại</button><button disabled={busy} onClick={() => void toggleRecord()}>{record?.active ? <Square size={14}/> : <Circle size={14}/>} {record?.active ? "Dừng ghi" : "Bắt đầu ghi"}</button><button disabled={busy || !record || record.active || !record.steps.length} onClick={() => void save()}><Save size={15}/>Lưu thành Flow</button></div>
      {error && <p role="alert">{error}</p>}{saved && <p role="status">{saved}</p>}
      <div className="inspector-body"><div className="inspector-screen">{snapshot ? <svg viewBox={`0 0 ${snapshot.width} ${snapshot.height}`} onPointerMove={(event) => setHover(hit(event))} onPointerLeave={() => setHover(null)} onClick={() => setSelected(hover)} aria-label="Chọn phần tử trên ảnh">
        <image href={`data:image/png;base64,${snapshot.pngBase64}`} width={snapshot.width} height={snapshot.height}/>
        {outlined && <rect x={outlined.x} y={outlined.y} width={outlined.width} height={outlined.height} fill="rgba(194,65,12,.15)" stroke="#c2410c" strokeWidth={4}/>}</svg> : <p>{busy ? "Đang đọc màn hình…" : "Chưa có ảnh"}</p>}</div>
        <div className="inspector-properties">
          <h3>Cây phần tử</h3><input className="inspector-search" aria-label="Tìm phần tử" placeholder="Tìm chữ, mô tả, ID hoặc loại" value={query} onChange={(event) => setQuery(event.target.value)}/>
          <div className="inspector-elements" role="tree">{visibleElements.map((candidate) => <button type="button" role="treeitem" aria-level={elementDepth(snapshot?.elements ?? [], candidate.index)} key={candidate.index} className={selected === candidate.index ? "active" : ""} style={{ paddingLeft: `${8 + elementDepth(snapshot?.elements ?? [], candidate.index) * 10}px` }} onClick={() => setSelected(candidate.index)}><span>{elementLabel(candidate)}</span><small>{candidate.clickable ? "Có thể bấm" : candidate.selector?.actionTarget ? "Bấm qua phần tử cha" : "Nội dung"}</small></button>)}</div>
          {element && <><dl>{[["Chữ", element.text], ["Mô tả", element.description], ["ID", element.resourceId], ["Loại", element.className]].map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value || "—"}</dd></div>)}</dl>
            <p>{element.selector ? "Tìm đúng một phần tử bằng thuộc tính" : "Chưa có quy tắc tìm duy nhất; phần tử này chỉ dùng để quan sát."}</p>
            {element.parent !== null && <button type="button" onClick={() => setSelected(element.parent)}><ChevronUp size={15}/>Chọn phần tử cha</button>}
            <button className="primary" disabled={busy || !actionable || !element.selector || awaitingPostcondition} onClick={() => void tap()}>{record?.active ? "Bấm và ghi bước" : "Bấm phần tử"}</button>
            {awaitingPostcondition && <button type="button" className="primary" disabled={busy || !element.selector} onClick={() => void confirmPostcondition()}>Dùng làm kết quả của bước vừa bấm</button>}
            <details><summary>Selector</summary><pre>{JSON.stringify(element.selector, null, 2)}</pre></details></>}
          {snapshot?.hierarchyXml && <details><summary>XML giao diện</summary><button type="button" onClick={() => void navigator.clipboard.writeText(snapshot.hierarchyXml ?? "")}><Copy size={14}/>Sao chép XML</button><pre className="inspector-xml">{snapshot.hierarchyXml}</pre></details>}
        </div></div>
      <footer>{busy ? "Đang thao tác…" : `${record?.steps.length ?? 0} bước đã ghi`} · {record?.active ? "Đang ghi" : "Đã dừng"}<span>{awaitingPostcondition ? "Chọn phần tử chỉ xuất hiện sau bước vừa bấm để xác minh." : "Chọn phần tử chỉ xem thuộc tính; bấm là thao tác riêng."}</span></footer>
    </div></div>, document.body);
}
