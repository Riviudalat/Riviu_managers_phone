import type {
  ActionKind,
  EvidenceSpec,
  FlowDocumentV2,
  FlowEdge,
  FlowNode,
  JsonObject,
} from "../types";

export type IdFactory = () => string;

export const createFlowId: IdFactory = () => crypto.randomUUID();

export function cloneFlowDocument(document: FlowDocumentV2): FlowDocumentV2 {
  return structuredClone(document);
}

export function defaultConfigForAction(kind: ActionKind): JsonObject {
  switch (kind) {
    case "start":
    case "end":
    case "home":
      return {};
    case "launchApp":
    case "terminateApp":
      return { bundleId: "" };
    case "wait":
      return { durationMs: 1000 };
    case "tap":
      return { accessibilityId: "" };
    case "swipe":
      return { durationMs: 280 };
    case "autoSwipe":
      return {
        preset: "tiktokFeed",
        count: 10,
        from: { x: 0.5, y: 0.78 },
        to: { x: 0.5, y: 0.28 },
        gestureDurationMs: 350,
        pauseMinMs: 1_200,
        pauseMaxMs: 2_500,
        jitterPercent: 2,
      };
    case "typeText":
      return {
        text: "",
        readBackLocator: { strategy: "accessibilityId", value: "" },
      };
    case "screenshot":
      return { label: "screenshot", format: "jpeg" };
    case "assertVisible":
      return { accessibilityId: "" };
    case "tapVision":
    case "ifVision":
      return { templatePngBase64: "", threshold: 0.85 };
    case "rawHttp":
    case "rawWda":
    case "shell":
      return {};
  }
}

export function createFlowNode(
  kind: ActionKind,
  position: { x: number; y: number },
  idFactory: IdFactory = createFlowId,
): FlowNode {
  return {
    id: idFactory(),
    kind,
    position: { ...position },
    config: defaultConfigForAction(kind),
    postcondition: null,
  };
}

export function newFlowDocument(
  name = "Untitled Flow",
  idFactory: IdFactory = createFlowId,
): FlowDocumentV2 {
  const flowId = idFactory();
  const start = createFlowNode("start", { x: 0, y: 80 }, idFactory);
  const end = createFlowNode("end", { x: 320, y: 80 }, idFactory);
  const edge: FlowEdge = {
    id: idFactory(),
    sourceNodeId: start.id,
    sourcePort: "flow",
    targetNodeId: end.id,
    targetPort: "flow",
  };

  return {
    schemaVersion: 2,
    id: flowId,
    name,
    revision: 0,
    entryNodeId: start.id,
    nodes: [start, end],
    edges: [edge],
    viewport: { x: 0, y: 0, zoom: 1 },
  };
}

export function duplicateDocument(
  source: FlowDocumentV2,
  name = `${source.name} Copy`,
  idFactory: IdFactory = createFlowId,
): FlowDocumentV2 {
  const nodeIds = new Map(source.nodes.map((node) => [node.id, idFactory()]));
  const nodes = source.nodes.map<FlowNode>((node) => ({
    ...structuredClone(node),
    id: requireMappedId(nodeIds, node.id),
  }));
  const edges = source.edges.map<FlowEdge>((edge) => ({
    ...edge,
    id: idFactory(),
    sourceNodeId: requireMappedId(nodeIds, edge.sourceNodeId),
    targetNodeId: requireMappedId(nodeIds, edge.targetNodeId),
  }));

  return {
    ...cloneFlowDocument(source),
    id: idFactory(),
    name,
    revision: 0,
    entryNodeId: requireMappedId(nodeIds, source.entryNodeId),
    nodes,
    edges,
  };
}

export function withNodeConfig(
  document: FlowDocumentV2,
  nodeId: string,
  config: JsonObject,
): FlowDocumentV2 {
  return mapNode(document, nodeId, (node) => ({
    ...node,
    config: structuredClone(config),
  }));
}

export function withNodePostcondition(
  document: FlowDocumentV2,
  nodeId: string,
  postcondition: EvidenceSpec | null,
): FlowDocumentV2 {
  return mapNode(document, nodeId, (node) => ({
    ...node,
    postcondition: postcondition === null ? null : structuredClone(postcondition),
  }));
}

function mapNode(
  document: FlowDocumentV2,
  nodeId: string,
  update: (node: FlowNode) => FlowNode,
): FlowDocumentV2 {
  let found = false;
  const nodes = document.nodes.map((node) => {
    if (node.id !== nodeId) return node;
    found = true;
    return update(node);
  });
  return found ? { ...document, nodes } : document;
}

function requireMappedId(ids: Map<string, string>, id: string): string {
  const mapped = ids.get(id);
  if (mapped === undefined) throw new Error(`Flow node ${id} is missing from the ID map`);
  return mapped;
}
