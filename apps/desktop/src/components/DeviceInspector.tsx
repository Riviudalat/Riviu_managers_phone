import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, ChevronRight, ChevronUp, Circle, Copy, RefreshCw, Save, Scan, Square, X } from "lucide-react";
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

function describeBoolean(value: boolean | null | undefined) {
  return value == null ? "—" : value ? "true" : "false";
}

export function DeviceInspector({ udid, onClose }: { udid: string; onClose: () => void }) {
  const { closing, close } = useClosingTransition(onClose);
  const ref = useModalFocus<HTMLDivElement>(close);
  const [snapshot, setSnapshot] = useState<InspectorSnapshot | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const [collapsed, setCollapsed] = useState<Set<number>>(() => new Set());
  const [record, setRecord] = useState<InspectorRecording | null>(null);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [name, setName] = useState("Quy trình mới");
  const [saved, setSaved] = useState("");
  const treeRef = useRef<HTMLDivElement>(null);

  const refreshRecord = useCallback(async () => setRecord(await inspectorRecording(udid)), [udid]);
  const observe = useCallback(async () => {
    setBusy(true); setError("");
    try {
      setSnapshot(await inspectorObserve(udid));
      setSelected(null); setHover(null); setCollapsed(new Set());
      await refreshRecord();
    } catch (reason) { setError(describeError(reason)); }
    finally { setBusy(false); }
  }, [refreshRecord, udid]);
  useEffect(() => { void observe(); }, [observe]);

  const hierarchy = useMemo(() => {
    const elements = snapshot?.elements ?? [];
    const byIndex = new Map(elements.map((candidate) => [candidate.index, candidate]));
    const children = new Map<number, number>();
    const depths = new Map<number, number>();
    for (const candidate of elements) {
      if (candidate.parent !== null && byIndex.has(candidate.parent)) {
        children.set(candidate.parent, (children.get(candidate.parent) ?? 0) + 1);
      }
    }
    const depthOf = (index: number, visited = new Set<number>()): number => {
      const cached = depths.get(index);
      if (cached !== undefined) return cached;
      const parent = byIndex.get(index)?.parent;
      if (parent == null || !byIndex.has(parent) || visited.has(index) || visited.size >= 32) return 1;
      visited.add(index);
      const depth = Math.min(32, depthOf(parent, visited) + 1);
      depths.set(index, depth);
      return depth;
    };
    for (const candidate of elements) depthOf(candidate.index);
    return { elements, byIndex, children, depths };
  }, [snapshot]);
  const element = selected === null ? undefined : hierarchy.byIndex.get(selected);
  const outlined = hierarchy.byIndex.get(hover ?? selected ?? -1);
  useEffect(() => {
    if (selected !== null && !query.trim()) treeRef.current?.querySelector<HTMLElement>(`[data-element-index="${selected}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [selected, query, collapsed]);
  const pending = record?.steps.at(-1);
  const awaitingPostcondition = Boolean(
    pending && !pending.verified && pending.afterId === snapshot?.id && pending.error === "Chờ chọn phần tử kết quả",
  );
  const visibleElements = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    if (needle) {
      const matching = new Set<number>();
      for (const candidate of hierarchy.elements) {
        if (![candidate.text, candidate.description, candidate.resourceId, candidate.className]
          .some((value) => value.toLocaleLowerCase().includes(needle))) continue;
        let current: InspectorElement | undefined = candidate;
        const visited = new Set<number>();
        while (current && !visited.has(current.index)) {
          matching.add(current.index);
          visited.add(current.index);
          current = current.parent === null ? undefined : hierarchy.byIndex.get(current.parent);
        }
      }
      return hierarchy.elements.filter((candidate) => matching.has(candidate.index));
    }
    return hierarchy.elements.filter((candidate) => {
      let parent = candidate.parent;
      const visited = new Set<number>();
      while (parent !== null && !visited.has(parent)) {
        if (collapsed.has(parent)) return false;
        visited.add(parent);
        parent = hierarchy.byIndex.get(parent)?.parent ?? null;
      }
      return true;
    });
  }, [query, hierarchy, collapsed]);

  const hit = (event: { currentTarget: SVGSVGElement; clientX: number; clientY: number }) => {
    if (!snapshot) return null;
    const rect = event.currentTarget.getBoundingClientRect();
    const scale = Math.min(rect.width / snapshot.width, rect.height / snapshot.height);
    if (!Number.isFinite(scale) || scale <= 0) return null;
    const imageWidth = snapshot.width * scale;
    const imageHeight = snapshot.height * scale;
    const localX = event.clientX - rect.left - (rect.width - imageWidth) / 2;
    const localY = event.clientY - rect.top - (rect.height - imageHeight) / 2;
    if (localX < 0 || localY < 0 || localX > imageWidth || localY > imageHeight) return null;
    const x = localX / scale;
    const y = localY / scale;
    let hitElement: InspectorElement | undefined;
    let area = Number.POSITIVE_INFINITY;
    for (const candidate of hierarchy.elements) {
      const candidateArea = candidate.width * candidate.height;
      if (candidateArea > 0 && candidateArea < area && x >= candidate.x && x <= candidate.x + candidate.width && y >= candidate.y && y <= candidate.y + candidate.height) {
        hitElement = candidate;
        area = candidateArea;
      }
    }
    return hitElement?.index ?? null;
  };
  const selectElement = (index: number | null) => {
    if (index !== null) {
      const next = new Set(collapsed);
      let parent = hierarchy.byIndex.get(index)?.parent ?? null;
      const visited = new Set<number>();
      while (parent !== null && !visited.has(parent)) {
        next.delete(parent);
        visited.add(parent);
        parent = hierarchy.byIndex.get(parent)?.parent ?? null;
      }
      setCollapsed(next);
    }
    setSelected(index);
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
  const properties: [string, string][] = element ? [
    ["index", String(element.index)],
    ["text", element.text],
    ["resource-id", element.resourceId],
    ["class", element.className],
    ["description", element.description],
    ["package", element.package ?? snapshot?.package ?? ""],
    ["bounds", `[${element.x},${element.y}][${element.x + element.width},${element.y + element.height}]`],
    ["enabled", describeBoolean(element.enabled)],
    ["clickable", describeBoolean(element.clickable)],
    ["checkable", describeBoolean(element.checkable)],
    ["checked", describeBoolean(element.checked)],
    ["selected", describeBoolean(element.selected)],
    ["focusable", describeBoolean(element.focusable)],
    ["focused", describeBoolean(element.focused)],
    ["scrollable", describeBoolean(element.scrollable)],
    ["long-clickable", describeBoolean(element.longClickable)],
    ["password", describeBoolean(element.password)],
  ] : [];
  return createPortal(<div className={`modal-backdrop inspector-backdrop${closing ? " is-closing" : ""}`}>
    <div ref={ref} tabIndex={-1} className="modal device-inspector" role="dialog" aria-label="Bắt thuộc tính và ghi Flow" aria-modal="true">
      <header><div><h2><Scan size={18}/>Bắt thuộc tính & ghi Flow</h2><small>{snapshot ? `${snapshot.package} · ${snapshot.version} · ${snapshot.locale}` : udid}</small></div><button className="icon-btn" aria-label="Đóng Inspector" onClick={close}><X size={18}/></button></header>
      <div className="inspector-toolbar"><input aria-label="Tên quy trình" value={name} disabled={record?.active} onChange={(event) => setName(event.target.value)}/><button disabled={busy} onClick={() => void observe()}><RefreshCw size={15}/>Đọc lại</button><button disabled={busy} onClick={() => void toggleRecord()}>{record?.active ? <Square size={14}/> : <Circle size={14}/>} {record?.active ? "Dừng ghi" : "Bắt đầu ghi"}</button><button disabled={busy || !record || record.active || !record.steps.length} onClick={() => void save()}><Save size={15}/>Lưu thành Flow</button></div>
      {error && <p role="alert">{error}</p>}{saved && <p role="status">{saved}</p>}
      <div className="inspector-body">
        <section className="inspector-screen" aria-label="Ảnh màn hình thiết bị">
          {snapshot ? <svg role="img" viewBox={`0 0 ${snapshot.width} ${snapshot.height}`} onPointerMove={(event) => setHover(hit(event))} onPointerLeave={() => setHover(null)} onClick={(event) => selectElement(hit(event))} aria-label="Màn hình thiết bị">
            <image href={`data:image/png;base64,${snapshot.pngBase64}`} width={snapshot.width} height={snapshot.height}/>
            {outlined && <rect x={outlined.x} y={outlined.y} width={outlined.width} height={outlined.height} fill="rgba(194,65,12,.15)" stroke="#c2410c" strokeWidth={4} pointerEvents="none"/>}
          </svg> : <p>{busy ? "Đang đọc màn hình…" : "Chưa có ảnh"}</p>}
        </section>
        <section className="inspector-properties" aria-label="Thuộc tính">
          <header><h3>Thuộc tính</h3><span>{element ? `#${element.index}` : "Chưa chọn"}</span></header>
          <div className="inspector-properties-scroll">
            <table aria-label="Thuộc tính phần tử"><thead><tr><th scope="col">Prop</th><th scope="col">Value</th></tr></thead><tbody>
              {element ? properties.map(([label, value]) => <tr key={label}><th scope="row">{label}</th><td title={value}>{value || "—"}</td></tr>) : <tr><td colSpan={2}>Chưa chọn phần tử.</td></tr>}
            </tbody></table>
            {element && <div className="inspector-actions">
              <p>{element.selector ? "Tìm đúng một phần tử bằng thuộc tính" : "Chưa có quy tắc tìm duy nhất; phần tử này chỉ dùng để quan sát."}</p>
              <div className="inspector-action-buttons">
                {element.parent !== null && hierarchy.byIndex.has(element.parent) && <button type="button" onClick={() => selectElement(element.parent)}><ChevronUp size={15}/>Chọn phần tử cha</button>}
                <button type="button" className="primary" disabled={busy || !actionable || !element.selector || awaitingPostcondition} onClick={() => void tap()}>{record?.active ? "Bấm và kiểm tra kết quả" : "Bấm phần tử"}</button>
                {awaitingPostcondition && <button type="button" className="primary" disabled={busy || !element.selector} onClick={() => void confirmPostcondition()}>Dùng làm kết quả của bước vừa bấm</button>}
              </div>
              <details><summary>Selector</summary><pre>{JSON.stringify(element.selector, null, 2)}</pre></details>
            </div>}
          </div>
        </section>
        <section className="inspector-tree-pane" aria-label="Cây giao diện">
          <header><h3>Cây phần tử</h3><span>{hierarchy.elements.length} phần tử</span></header>
          <input className="inspector-search" aria-label="Tìm phần tử" placeholder="Tìm chữ, mô tả, ID hoặc loại" value={query} onChange={(event) => setQuery(event.target.value)}/>
          <div ref={treeRef} className="inspector-elements" role="tree" aria-label="Cây phần tử">{visibleElements.map((candidate) => {
            const hasChildren = (hierarchy.children.get(candidate.index) ?? 0) > 0;
            const expanded = !collapsed.has(candidate.index) || !!query.trim();
            return <div className="inspector-tree-row" key={candidate.index} style={{ paddingLeft: `${Math.min(31, (hierarchy.depths.get(candidate.index) ?? 1) - 1) * 12}px` }}>
              {hasChildren ? <button type="button" className="inspector-tree-toggle" aria-label={`${expanded ? "Thu gọn" : "Mở rộng"} ${elementLabel(candidate)}`} onClick={() => setCollapsed((current) => { const next = new Set(current); if (next.has(candidate.index)) next.delete(candidate.index); else next.add(candidate.index); return next; })}>{expanded ? <ChevronDown size={14}/> : <ChevronRight size={14}/>}</button> : <span className="inspector-tree-spacer"/>}
              <button type="button" role="treeitem" data-element-index={candidate.index} aria-level={hierarchy.depths.get(candidate.index) ?? 1} aria-expanded={hasChildren ? expanded : undefined} aria-selected={selected === candidate.index} className={selected === candidate.index ? "active" : ""} onClick={() => selectElement(candidate.index)}><span>{elementLabel(candidate)}</span><small>{candidate.clickable ? "Có thể bấm" : candidate.selector?.actionTarget ? "Bấm qua phần tử cha" : "Nội dung"}</small></button>
            </div>;
          })}</div>
          {snapshot?.hierarchyXml && <div className="inspector-xml-tools"><button type="button" onClick={() => void navigator.clipboard.writeText(snapshot.hierarchyXml ?? "").catch((reason) => setError(describeError(reason)))}><Copy size={14}/>Sao chép XML</button><details><summary>XML giao diện</summary><pre className="inspector-xml">{snapshot.hierarchyXml}</pre></details></div>}
        </section>
      </div>
      <footer>{busy ? "Đang thao tác…" : `${record?.steps.length ?? 0} bước đã ghi`} · {record?.active ? "Đang ghi" : "Đã dừng"}<span>{awaitingPostcondition ? "Chọn phần tử chỉ xuất hiện sau bước vừa bấm để xác minh." : "Chọn phần tử chỉ xem thuộc tính; bấm là thao tác riêng."}</span></footer>
    </div></div>, document.body);
}
