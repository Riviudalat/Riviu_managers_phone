import { createFlowId, type IdFactory } from "./flow/model";
import { isFlowDocumentV2 } from "./flow/validation";
import { MAX_FLOW_IMPORT_BYTES, parseWorkflowImport } from "./flowImport";
import type { FlowDocumentV2, FlowNode, JsonObject } from "./types";

export interface CompositionDraft {
  kind: "subflow" | "repeat";
  document: FlowDocumentV2;
  count: number;
  inputs: Record<string, string>;
  outputs: Record<string, string>;
  /** Explicit opt-in. Omitting this keeps historical embedded snapshots frozen. */
  library?: { flowId: string; channel: "published" };
}

export function parseCompositionBody(raw: string): FlowDocumentV2 {
  const value = parseWorkflowImport(raw);
  if (!isFlowDocumentV2(value)) throw new Error("Nội dung cần là tài liệu Flow V2 đầy đủ.");
  return value;
}

export function parseCompositionBindings(raw: string): Record<string, string> {
  const value = parseWorkflowImport(raw || "{}");
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Ánh xạ biến phải là đối tượng JSON.");
  for (const [key, source] of Object.entries(value)) if (!/^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(key) || typeof source !== "string" || !/^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(source)) throw new Error("Tên biến gồm chữ, số và gạch dưới, tối đa 64 ký tự; bắt đầu bằng chữ hoặc gạch dưới.");
  return Object.fromEntries(Object.entries(value)) as Record<string, string>;
}

function freezeComposition(document: FlowDocumentV2): FlowDocumentV2 {
  const snapshot = structuredClone(document);
  const freeze = (body: FlowDocumentV2, depth: number) => {
    if (depth > 8) throw new Error("Flow con vượt giới hạn 8 cấp.");
    for (const node of body.nodes) {
      if (node.kind !== "subflow" && node.kind !== "repeat") continue;
      delete node.config.library;
      const nested = node.config.document;
      if (!isFlowDocumentV2(nested)) throw new Error("Nội dung Flow con không hợp lệ.");
      freeze(nested, depth + 1);
    }
  };
  freeze(snapshot, 0);
  return snapshot;
}

export function applyFlowComposition(parent: FlowDocumentV2, draft: CompositionDraft, editingNodeId: string | null = null, idFactory: IdFactory = createFlowId): FlowDocumentV2 {
  if (!isFlowDocumentV2(draft.document)) throw new Error("Nội dung Flow con không hợp lệ.");
  if (draft.kind === "repeat" && (!Number.isInteger(draft.count) || draft.count < 1 || draft.count > 50)) throw new Error("Số lượt lặp phải từ 1 đến 50.");
  if (draft.library && (draft.library.channel !== "published" || draft.library.flowId !== draft.document.id || draft.document.revision < 1)) throw new Error("Flow dùng chung cần đúng phiên bản nguồn đã công bố.");
  if (draft.library?.flowId === parent.id) throw new Error("Flow không thể gọi chính nó qua thư viện.");
  const body = draft.library ? structuredClone(draft.document) : freezeComposition(draft.document);
  const config: JsonObject = { document: body as unknown as JsonObject, inputs: { ...draft.inputs }, outputs: { ...draft.outputs }, ...(draft.kind === "repeat" ? { count: draft.count } : {}), ...(draft.library ? { library: { ...draft.library } } : {}) };
  if (new TextEncoder().encode(JSON.stringify(config)).byteLength > MAX_FLOW_IMPORT_BYTES) throw new Error("Flow con vượt giới hạn 1 MiB.");
  const document = structuredClone(parent);
  if (editingNodeId) {
    const node = document.nodes.find((item) => item.id === editingNodeId);
    if (!node || !["subflow", "repeat"].includes(node.kind)) throw new Error("Bước ghép đã thay đổi. Đóng hộp thoại và chọn lại bước.");
    node.kind = draft.kind;
    node.config = config;
    node.postcondition = null;
    return document;
  }
  const end = document.nodes.find((node) => node.kind === "end");
  if (!end || document.nodes.filter((node) => node.kind === "end").length !== 1) throw new Error("Flow cần một Kết thúc để nối Flow con.");
  const incoming = document.edges.filter((edge) => edge.targetNodeId === end.id);
  if (incoming.length === 0) throw new Error("Nối các bước tới Kết thúc trước khi ghép Flow con.");
  const used = new Set([...document.nodes.map((node) => node.id), ...document.edges.map((edge) => edge.id)]);
  const nextId = () => { const id = idFactory(); if (!id || used.has(id)) throw new Error("ID bước mới bị trùng."); used.add(id); return id; };
  const composition: FlowNode = { id: nextId(), kind: draft.kind, position: { x: end.position.x, y: end.position.y }, config, postcondition: null };
  const join = incoming.length > 1 ? { id: nextId(), kind: "join" as const, position: { x: end.position.x - 120, y: end.position.y }, config: {}, postcondition: null } : null;
  for (const edge of incoming) edge.targetNodeId = join?.id ?? composition.id;
  if (join) {
    document.nodes.push(join);
    document.edges.push({ id: nextId(), sourceNodeId: join.id, sourcePort: "flow", targetNodeId: composition.id, targetPort: "flow" });
  }
  document.nodes.push(composition);
  document.edges.push({ id: nextId(), sourceNodeId: composition.id, sourcePort: "flow", targetNodeId: end.id, targetPort: "flow" });
  end.position.x += 240;
  return document;
}
