import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, ChevronRight, ChevronUp, Circle, ClipboardPaste, Copy, MoveVertical, RefreshCw, Save, Scan, Square, X } from "lucide-react";
import { createPortal } from "react-dom";

import {
  inspectorConfirmPostcondition,
  inspectorObserve,
  inspectorRecord,
  inspectorRecordClipboard,
  inspectorRecording,
  inspectorSwipe,
  inspectorTap,
} from "../inspectorApi";
import type { InspectorClipboard, InspectorElement, InspectorRecording, InspectorSnapshot, InspectorSwipe } from "../inspectorApi";
import type { FlowNode, InspectorElementSelector, JsonObject } from "../types";
import { flowSaveRevision } from "../api";
import { createFlowNode, newFlowDocument } from "../flow/model";
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

/** "emr · ô thứ 1 trong lưới": the short id, the position and what kind of container holds it. */
function describeOrdinal(selector: InspectorElementSelector) {
  const identity = (selector.resourceId || selector.className || "").split(/[/.]/).at(-1) ?? "";
  const container = selector.scope?.className ?? "";
  const place = /Grid/.test(container) ? `ô thứ ${selector.ordinal} trong lưới`
    : /List|Recycler/.test(container) ? `mục thứ ${selector.ordinal} trong danh sách`
      : `phần tử thứ ${selector.ordinal} trong khung cuộn`;
  return { label: `${identity} · ${place}`, place };
}

type SwipeDirection = "down" | "up" | "left" | "right";
const SWIPE_DIRECTIONS: [SwipeDirection, string][] = [
  ["down", "Kéo xuống (làm mới)"],
  ["up", "Vuốt lên (cuộn xuống)"],
  ["left", "Vuốt sang trái"],
  ["right", "Vuốt sang phải"],
];

/** Screen fractions from a direction and a distance; the start leaves room for the longest pull. */
function swipeFor(direction: SwipeDirection, distance: number): InspectorSwipe {
  const durationMs = 400;
  switch (direction) {
    case "down": return { from: { x: 0.5, y: 0.3 }, to: { x: 0.5, y: 0.3 + distance }, durationMs };
    case "up": return { from: { x: 0.5, y: 0.75 }, to: { x: 0.5, y: 0.75 - distance }, durationMs };
    case "left": return { from: { x: 0.8, y: 0.5 }, to: { x: 0.8 - distance, y: 0.5 }, durationMs };
    case "right": return { from: { x: 0.2, y: 0.5 }, to: { x: 0.2 + distance, y: 0.5 }, durationMs };
  }
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
  const [swipeDirection, setSwipeDirection] = useState<SwipeDirection>("down");
  const [swipeDistance, setSwipeDistance] = useState(0.4);
  const [clipboardName, setClipboardName] = useState("link");
  const [clipboardPreview, setClipboardPreview] = useState<string | null>(null);
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
  const chosenSelector = element?.selector ?? element?.ordinalSelector ?? null;
  const ordinal = !element?.selector && element?.ordinalSelector ? describeOrdinal(element.ordinalSelector) : null;
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
    if (!chosenSelector) return;
    setBusy(true); setError(""); setClipboardPreview(null);
    try { setSnapshot(await inspectorTap(udid, chosenSelector)); setSelected(null); }
    catch (reason) { setError(describeError(reason)); }
    finally { await refreshRecord(); setBusy(false); }
  };
  const swipe = async () => {
    setBusy(true); setError(""); setClipboardPreview(null);
    try { setSnapshot(await inspectorSwipe(udid, swipeFor(swipeDirection, swipeDistance))); setSelected(null); }
    catch (reason) { setError(describeError(reason)); }
    finally { await refreshRecord(); setBusy(false); }
  };
  // Same-screen is the operator's explicit "this step refreshes in place"; the default stays strict.
  const confirmPostcondition = async (sameScreen: boolean) => {
    if (!snapshot || !chosenSelector || !awaitingPostcondition) return;
    setBusy(true); setError("");
    try {
      setRecord(sameScreen
        ? await inspectorConfirmPostcondition(udid, snapshot.id, chosenSelector, true)
        : await inspectorConfirmPostcondition(udid, snapshot.id, chosenSelector));
    }
    catch (reason) { setError(describeError(reason)); }
    finally { setBusy(false); }
  };
  const recordClipboard = async (step: InspectorClipboard) => {
    setBusy(true); setError(""); setClipboardPreview(null);
    try {
      const result = await inspectorRecordClipboard(udid, step);
      setRecord(result.recording);
      if (step.mode === "read") setClipboardPreview(result.preview ?? "");
    }
    catch (reason) { setError(describeError(reason)); }
    finally { setBusy(false); }
  };
  const save = async () => {
    if (!record || record.active || !record.steps.length) return;
    setBusy(true); setError("");
    try {
      if (record.steps.some((step) => !step.verified || (!step.clipboard && !step.expected))) throw Error("Có bước chưa xác minh phần tử kết quả.");
      const bundleId = record.steps.map((step) => step.selector?.package ?? step.expected?.package).find(Boolean);
      if (!bundleId) throw Error("Cần ít nhất một bước bấm hoặc vuốt đã xác minh để biết ứng dụng.");
      const doc = newFlowDocument(record.name);
      const start = doc.nodes.find((node) => node.kind === "start")!;
      const end = doc.nodes.find((node) => node.kind === "end")!;
      const launch = createFlowNode("launchApp", { x: 220, y: 100 });
      launch.config = { bundleId };
      launch.postcondition = { kind: "activeAppEquals", bundleId };
      const steps = record.steps.map((step, index): FlowNode => {
        const position = { x: 440 + index * 220, y: 100 };
        if (step.clipboard) {
          const node = createFlowNode("readClipboard", position);
          node.config = step.clipboard.mode === "arm" ? { mode: "arm" } : { mode: "read", name: step.clipboard.name };
          return node;
        }
        if (step.swipe) {
          const node = createFlowNode("swipe", position);
          node.config = {
            fromFraction: { x: step.swipe.from.x, y: step.swipe.from.y },
            toFraction: { x: step.swipe.to.x, y: step.swipe.to.y },
            durationMs: step.swipe.durationMs,
          };
          node.postcondition = { kind: "elementVisible", selector: step.expected! };
          return node;
        }
        if (!step.selector) throw Error("Có bước ghi không rõ thao tác; hãy ghi lại phiên này.");
        const node = createFlowNode("tap", position);
        node.config = { selector: step.selector as unknown as JsonObject };
        node.postcondition = { kind: "elementVisible", selector: step.expected! };
        return node;
      });
      end.position = { x: 440 + steps.length * 220, y: 100 };
      doc.nodes = [start, launch, ...steps, end];
      doc.edges = doc.nodes.slice(0, -1).map((node, index) => ({ id: crypto.randomUUID(), sourceNodeId: node.id, sourcePort: "flow", targetNodeId: doc.nodes[index + 1].id, targetPort: "flow" }));
      const revision = await flowSaveRevision(doc, null);
      setSaved(`Đã lưu Flow “${revision.document.name}”. Mở Flow thiết bị để chỉnh và chạy.`);
    } catch (reason) { setError(describeError(reason)); }
    finally { setBusy(false); }
  };

  const actionable = Boolean(element?.clickable || chosenSelector?.actionTarget);
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
              <p>{element.selector ? "Tìm đúng một phần tử bằng thuộc tính" : ordinal ? `Không có thuộc tính riêng; chọn theo thứ tự: ${ordinal.label}` : "Chưa có quy tắc tìm duy nhất; phần tử này chỉ dùng để quan sát."}</p>
              <div className="inspector-action-buttons">
                {element.parent !== null && hierarchy.byIndex.has(element.parent) && <button type="button" onClick={() => selectElement(element.parent)}><ChevronUp size={15}/>Chọn phần tử cha</button>}
                <button type="button" className="primary" disabled={busy || !actionable || !chosenSelector || awaitingPostcondition} onClick={() => void tap()}>{ordinal ? `Bấm ${ordinal.place}${record?.active ? " và kiểm tra kết quả" : ""}` : record?.active ? "Bấm và kiểm tra kết quả" : "Bấm phần tử"}</button>
                {awaitingPostcondition && <button type="button" className="primary" disabled={busy || !chosenSelector} onClick={() => void confirmPostcondition(false)}>Dùng làm kết quả của bước vừa bấm</button>}
                {awaitingPostcondition && <button type="button" disabled={busy || !chosenSelector} onClick={() => void confirmPostcondition(true)}>Bước này làm mới, màn hình giữ nguyên — dùng phần tử vẫn còn làm kết quả</button>}
              </div>
              <details><summary>Selector</summary><pre>{JSON.stringify(chosenSelector, null, 2)}</pre></details>
            </div>}
            <div className="inspector-recorder-tools">
              <div role="group" aria-label="Ghi bước vuốt">
                <select aria-label="Hướng vuốt" value={swipeDirection} onChange={(event) => setSwipeDirection(event.target.value as SwipeDirection)}>
                  {SWIPE_DIRECTIONS.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
                </select>
                <select aria-label="Quãng vuốt" value={swipeDistance} onChange={(event) => setSwipeDistance(Number(event.target.value))}>
                  {[0.25, 0.4, 0.6].map((value) => <option key={value} value={value}>{Math.round(value * 100)}% màn hình</option>)}
                </select>
                <button type="button" disabled={busy || awaitingPostcondition} onClick={() => void swipe()}><MoveVertical size={15}/>{record?.active ? "Vuốt và kiểm tra kết quả" : "Vuốt"}</button>
              </div>
              {record?.active && <div role="group" aria-label="Ghi bước clipboard">
                <button type="button" disabled={busy || awaitingPostcondition} title="Ghi một dấu riêng vào clipboard trước các bước sao chép" onClick={() => void recordClipboard({ mode: "arm" })}>Ghi dấu clipboard</button>
                <input aria-label="Tên biến clipboard" value={clipboardName} maxLength={64} onChange={(event) => setClipboardName(event.target.value)}/>
                <button type="button" disabled={busy || awaitingPostcondition || !clipboardName.trim()} onClick={() => void recordClipboard({ mode: "read", name: clipboardName.trim() })}><ClipboardPaste size={15}/>Ghi bước đọc clipboard vào biến {clipboardName.trim() || "…"}</button>
              </div>}
              {clipboardPreview !== null && <p role="status" aria-label="Clipboard vừa đọc">Clipboard vừa đọc: {clipboardPreview || "(trống)"}</p>}
            </div>
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
      <footer>{busy ? "Đang thao tác…" : `${record?.steps.length ?? 0} bước đã ghi`} · {record?.active ? "Đang ghi" : "Đã dừng"}<span>{awaitingPostcondition ? "Chọn phần tử chỉ xuất hiện sau bước vừa làm để xác minh; bước làm mới thì chọn phần tử vẫn còn." : "Chọn phần tử chỉ xem thuộc tính; bấm là thao tác riêng."}</span></footer>
    </div></div>, document.body);
}
