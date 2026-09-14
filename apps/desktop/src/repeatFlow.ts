import type { FlowDocumentV2, FlowNode } from "./types";
import { createFlowId, type IdFactory } from "./flow/model";

export const MAX_FLOW_REPEAT_COUNT = 50;
export const MAX_REPEATED_FLOW_NODES = 500;

export interface RepeatFlowAnalysis {
  body: FlowNode[];
  totalNodeCount: number;
  waitDurationMs: number;
  error: string | null;
}

/** Follow actual edges. Array/canvas order is not execution order. */
export function analyzeFlowRepeat(document: FlowDocumentV2, count: number): RepeatFlowAnalysis {
  const fail = (error: string): RepeatFlowAnalysis => ({ body: [], totalNodeCount: 0, waitDurationMs: 0, error });
  if (!Number.isSafeInteger(count) || count < 1 || count > MAX_FLOW_REPEAT_COUNT) return fail("Số lượt phải là số nguyên từ 1 đến 50.");
  const nodes = new Map(document.nodes.map((item) => [item.id, item]));
  if (nodes.size !== document.nodes.length) return fail("Flow có ID node bị trùng.");
  const start = nodes.get(document.entryNodeId);
  if (!start || start.kind !== "start" || document.nodes.filter((item) => item.kind === "start").length !== 1 || document.nodes.filter((item) => item.kind === "end").length !== 1) return fail("Flow cần đúng một Bắt đầu và một Kết thúc.");
  if (document.nodes.some((item) => ["ifVision", "ifVisible", "ifValue"].includes(item.kind))) return fail("Flow có nhánh điều kiện. Hãy tách chuỗi hành động tuyến tính trước khi tạo lượt lặp.");
  const outgoing = new Map<string, string>();
  const incoming = new Map<string, number>();
  const edgeIds = new Set<string>();
  for (const edge of document.edges) {
    if (edgeIds.has(edge.id) || !nodes.has(edge.sourceNodeId) || !nodes.has(edge.targetNodeId) || edge.sourcePort !== "flow" || edge.targetPort !== "flow" || outgoing.has(edge.sourceNodeId)) return fail("Flow phải có một đường chạy liên tục, dùng cổng flow và các đích node hợp lệ.");
    edgeIds.add(edge.id);
    outgoing.set(edge.sourceNodeId, edge.targetNodeId);
    incoming.set(edge.targetNodeId, (incoming.get(edge.targetNodeId) ?? 0) + 1);
  }
  if (incoming.has(start.id)) return fail("Node Bắt đầu có cạnh đi vào.");
  for (const item of document.nodes) {
    if (item.id !== start.id && incoming.get(item.id) !== 1) return fail("Mỗi bước phải có đúng một cạnh đi vào; hãy xử lý node rời hoặc nhánh gộp.");
    if (item.kind === "end" ? outgoing.has(item.id) : !outgoing.has(item.id)) return fail("Đường chạy phải kết thúc ở node Kết thúc.");
  }
  const visited = new Set<string>();
  const body: FlowNode[] = [];
  let current: FlowNode | undefined = start;
  while (current && !visited.has(current.id)) {
    visited.add(current.id);
    if (current.kind !== "start" && current.kind !== "end") body.push(current);
    current = nodes.get(outgoing.get(current.id) ?? "");
  }
  if (current || visited.size !== nodes.size) return fail("Flow có vòng lặp hoặc node không nằm trên đường chạy.");
  if (body.length === 0) return fail("Thêm ít nhất một hành động giữa Bắt đầu và Kết thúc.");
  const totalNodeCount = 2 + body.length * count;
  if (totalNodeCount > MAX_REPEATED_FLOW_NODES) return fail(`Tổng ${totalNodeCount} node vượt giới hạn 500; hãy giảm số lượt hoặc chia Flow.`);
  const waitDurationMs = body.reduce((total, item) => total + (item.kind === "wait" && typeof item.config.durationMs === "number" ? item.config.durationMs : 0), 0) * count;
  return { body, totalNodeCount, waitDurationMs, error: null };
}

/** Expand to a bounded DAG. Every copy owns its action ID and therefore its own attempt ledger. */
export function repeatLinearFlow(document: FlowDocumentV2, count: number, idFactory: IdFactory = createFlowId): FlowDocumentV2 {
  const analysis = analyzeFlowRepeat(document, count);
  if (analysis.error) throw new Error(analysis.error);
  if (count === 1) return structuredClone(document);
  const used = new Set([...document.nodes.map((item) => item.id), ...document.edges.map((item) => item.id)]);
  const freshId = () => {
    const id = idFactory();
    if (!id || used.has(id)) throw new Error("Không tạo được ID riêng cho bước lặp.");
    used.add(id);
    return id;
  };
  const start = structuredClone(document.nodes.find((item) => item.id === document.entryNodeId)!);
  const end = structuredClone(document.nodes.find((item) => item.kind === "end")!);
  const nodes = [start];
  for (let iteration = 0; iteration < count; iteration++) {
    for (const source of analysis.body) {
      const copy = structuredClone(source);
      if (iteration > 0) copy.id = freshId();
      // Editor layout only; these pixels are not device coordinates.
      copy.position = { x: start.position.x + nodes.length * 240, y: start.position.y };
      nodes.push(copy);
    }
  }
  end.position = { x: start.position.x + nodes.length * 240, y: start.position.y };
  nodes.push(end);
  const edges = nodes.slice(1).map((item, index) => ({ id: freshId(), sourceNodeId: nodes[index].id, sourcePort: "flow", targetNodeId: item.id, targetPort: "flow" }));
  return { ...structuredClone(document), nodes, edges };
}
