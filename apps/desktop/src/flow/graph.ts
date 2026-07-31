import type { Edge, Node } from "@xyflow/react";
import type {
  ActionKind,
  EvidenceSpec,
  FlowDocumentV2,
  FlowEdge,
  FlowNode,
  FlowValidationIssue,
  JsonObject,
} from "../types";
import { createFlowId, type IdFactory } from "./model";

export interface FlowNodeData extends Record<string, unknown> {
  kind: ActionKind;
  config: JsonObject;
  postcondition: EvidenceSpec | null;
  issues: FlowValidationIssue[];
}

export type FlowCanvasNode = Node<FlowNodeData, "flowAction">;
export type FlowCanvasEdge = Edge;

export interface FlowCanvasGraph {
  nodes: FlowCanvasNode[];
  edges: FlowCanvasEdge[];
}

export function toCanvas(
  document: FlowDocumentV2,
  issues: FlowValidationIssue[] = [],
): FlowCanvasGraph {
  return {
    nodes: document.nodes.map<FlowCanvasNode>((node) => ({
      id: node.id,
      type: "flowAction",
      position: { ...node.position },
      data: {
        kind: node.kind,
        config: structuredClone(node.config),
        postcondition: node.postcondition ? structuredClone(node.postcondition) : null,
        issues: issues.filter((issue) => issue.nodeId === node.id),
      },
    })),
    edges: document.edges.map<FlowCanvasEdge>((edge) => ({
      id: edge.id,
      source: edge.sourceNodeId,
      sourceHandle: edge.sourcePort,
      target: edge.targetNodeId,
      targetHandle: edge.targetPort,
    })),
  };
}

export function withCanvasLayout(
  document: FlowDocumentV2,
  nodes: FlowCanvasNode[],
  edges: FlowCanvasEdge[],
): FlowDocumentV2 {
  const positionById = new Map(nodes.map((node) => [node.id, node.position]));
  return {
    ...document,
    nodes: document.nodes.map((node) => ({
      ...node,
      position: { ...(positionById.get(node.id) ?? node.position) },
    })),
    edges: edges.map((edge) => ({
      id: edge.id,
      sourceNodeId: edge.source,
      sourcePort: edge.sourceHandle ?? "flow",
      targetNodeId: edge.target,
      targetPort: edge.targetHandle ?? "flow",
    })),
  };
}

export function insertNodeOnEdge(
  document: FlowDocumentV2,
  edgeId: string,
  node: FlowNode,
  idFactory: IdFactory = createFlowId,
): FlowDocumentV2 {
  if (document.nodes.some((candidate) => candidate.id === node.id)) return document;
  const edgeIndex = document.edges.findIndex((edge) => edge.id === edgeId);
  if (edgeIndex < 0) return document;

  const selected = document.edges[edgeIndex];
  const replacement: FlowEdge[] = [
    {
      id: idFactory(),
      sourceNodeId: selected.sourceNodeId,
      sourcePort: selected.sourcePort,
      targetNodeId: node.id,
      targetPort: "flow",
    },
    {
      id: idFactory(),
      sourceNodeId: node.id,
      sourcePort: "flow",
      targetNodeId: selected.targetNodeId,
      targetPort: selected.targetPort,
    },
  ];
  return {
    ...document,
    nodes: [...document.nodes, structuredClone(node)],
    edges: [
      ...document.edges.slice(0, edgeIndex),
      ...replacement,
      ...document.edges.slice(edgeIndex + 1),
    ],
  };
}

export function appendUnconnectedNode(
  document: FlowDocumentV2,
  node: FlowNode,
): FlowDocumentV2 {
  if (document.nodes.some((candidate) => candidate.id === node.id)) return document;
  return { ...document, nodes: [...document.nodes, structuredClone(node)] };
}

export function reconnectEdge(
  document: FlowDocumentV2,
  edgeId: string,
  sourceNodeId: string,
  targetNodeId: string,
): FlowDocumentV2 {
  if (
    sourceNodeId === targetNodeId ||
    !document.nodes.some((node) => node.id === sourceNodeId) ||
    !document.nodes.some((node) => node.id === targetNodeId)
  ) {
    return document;
  }
  let found = false;
  const edges = document.edges.map((edge) => {
    if (edge.id !== edgeId) return edge;
    found = true;
    return { ...edge, sourceNodeId, targetNodeId };
  });
  return found ? { ...document, edges } : document;
}

export function deleteExecutableNode(
  document: FlowDocumentV2,
  nodeId: string,
  idFactory: IdFactory = createFlowId,
): FlowDocumentV2 {
  const node = document.nodes.find((candidate) => candidate.id === nodeId);
  if (node === undefined || node.kind === "start" || node.kind === "end") return document;

  const incoming = document.edges.filter((edge) => edge.targetNodeId === nodeId);
  const outgoing = document.edges.filter((edge) => edge.sourceNodeId === nodeId);
  const remaining = document.edges.filter(
    (edge) => edge.sourceNodeId !== nodeId && edge.targetNodeId !== nodeId,
  );
  if (incoming.length === 1 && outgoing.length === 1) {
    remaining.push({
      id: idFactory(),
      sourceNodeId: incoming[0].sourceNodeId,
      sourcePort: incoming[0].sourcePort,
      targetNodeId: outgoing[0].targetNodeId,
      targetPort: outgoing[0].targetPort,
    });
  }

  return {
    ...document,
    nodes: document.nodes.filter((candidate) => candidate.id !== nodeId),
    edges: remaining,
  };
}
