import {
  Background,
  Controls,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  addEdge,
  applyEdgeChanges,
  applyNodeChanges,
  type Connection,
  type EdgeChange,
  type NodeChange,
  type NodeTypes,
  useReactFlow,
} from "@xyflow/react";
import { useEffect, useMemo, useState, type DragEvent } from "react";
import type {
  ActionDefinition,
  ActionKind,
  FlowDocumentV2,
  FlowNode,
  FlowValidationIssue,
  FlowViewport,
} from "../../types";
import { createFlowNode } from "../../flow/model";
import { toCanvas, type FlowCanvasEdge, type FlowCanvasNode } from "../../flow/graph";
import { FlowActionNode } from "./FlowActionNode";
import { FLOW_ACTION_MIME } from "./FlowPalette";

const NODE_TYPES: NodeTypes = { flowAction: FlowActionNode };

interface FlowCanvasProps {
  document: FlowDocumentV2;
  catalog: ActionDefinition[];
  issues: FlowValidationIssue[];
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string | null) => void;
  onReplaceCanvas: (nodes: FlowCanvasNode[], edges: FlowCanvasEdge[]) => void;
  onInsertNode: (edgeId: string, node: FlowNode) => void;
  onAppendNode: (node: FlowNode) => void;
  onDeleteNode: (nodeId: string) => void;
  onViewport: (viewport: FlowViewport) => void;
}

function pointSegmentDistance(
  point: { x: number; y: number },
  start: { x: number; y: number },
  end: { x: number; y: number },
): number {
  const dx = end.x - start.x;
  const dy = end.y - start.y;
  const lengthSquared = dx * dx + dy * dy;
  if (lengthSquared === 0) return Math.hypot(point.x - start.x, point.y - start.y);
  const ratio = Math.max(
    0,
    Math.min(1, ((point.x - start.x) * dx + (point.y - start.y) * dy) / lengthSquared),
  );
  return Math.hypot(point.x - (start.x + ratio * dx), point.y - (start.y + ratio * dy));
}

function nearestEdge(
  point: { x: number; y: number },
  nodes: FlowCanvasNode[],
  edges: FlowCanvasEdge[],
): string | null {
  const positions = new Map(nodes.map((node) => [node.id, node.position]));
  let nearest: { id: string; distance: number } | null = null;
  for (const edge of edges) {
    const source = positions.get(edge.source);
    const target = positions.get(edge.target);
    if (!source || !target) continue;
    const distance = pointSegmentDistance(
      point,
      { x: source.x + 168, y: source.y + 34 },
      { x: target.x, y: target.y + 34 },
    );
    if (nearest === null || distance < nearest.distance) nearest = { id: edge.id, distance };
  }
  return nearest && nearest.distance <= 24 ? nearest.id : null;
}

function FlowCanvasInner(props: FlowCanvasProps) {
  const { screenToFlowPosition } = useReactFlow();
  const [selectedEdges, setSelectedEdges] = useState<Set<string>>(new Set());
  const graph = useMemo(() => toCanvas(props.document, props.issues), [props.document, props.issues]);
  const [transientNodes, setTransientNodes] = useState(graph.nodes);
  useEffect(() => setTransientNodes(graph.nodes), [graph.nodes]);
  const nodes = transientNodes.map((node) => ({
    ...node,
    selected: node.id === props.selectedNodeId,
  }));
  const edges = graph.edges.map((edge) => ({
    ...edge,
    selected: selectedEdges.has(edge.id),
  }));

  const replaceAfterNodeChanges = (changes: NodeChange<FlowCanvasNode>[]) => {
    setTransientNodes((current) => applyNodeChanges(changes, current));
    const selected = changes.find(
      (change): change is Extract<NodeChange<FlowCanvasNode>, { type: "select" }> =>
        change.type === "select" && change.selected,
    );
    if (selected) props.onSelectNode(selected.id);
  };

  const changeEdges = (changes: EdgeChange<FlowCanvasEdge>[]) => {
    const selected = new Set(selectedEdges);
    for (const change of changes) {
      if (change.type === "select") {
        if (change.selected) selected.add(change.id);
        else selected.delete(change.id);
      }
    }
    setSelectedEdges(selected);
    const structural = changes.filter((change) => change.type !== "select");
    if (structural.length > 0) {
      props.onReplaceCanvas(nodes, applyEdgeChanges(structural, edges));
    }
  };

  const connect = (connection: Connection) => {
    if (!connection.source || !connection.target || connection.source === connection.target) return;
    props.onReplaceCanvas(
      nodes,
      addEdge(
        {
          ...connection,
          id: crypto.randomUUID(),
          sourceHandle: connection.sourceHandle ?? "flow",
          targetHandle: connection.targetHandle ?? "flow",
        },
        edges,
      ),
    );
  };

  const drop = (event: DragEvent) => {
    event.preventDefault();
    const kind = event.dataTransfer.getData(FLOW_ACTION_MIME) as ActionKind;
    if (!props.catalog.some((action) => action.kind === kind && action.disabledReason === null)) {
      return;
    }
    const position = screenToFlowPosition({ x: event.clientX, y: event.clientY });
    if (!Number.isFinite(position.x) || !Number.isFinite(position.y)) return;
    const node = createFlowNode(kind, position);
    const exact = selectedEdges.size === 1 ? [...selectedEdges][0] : null;
    const edgeId = exact ?? nearestEdge(position, nodes, edges);
    if (edgeId) props.onInsertNode(edgeId, node);
    else props.onAppendNode(node);
  };

  return (
    <div className="flow-canvas-region" data-testid="flow-canvas">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={NODE_TYPES}
        onNodesChange={replaceAfterNodeChanges}
        onEdgesChange={changeEdges}
        onConnect={connect}
        onNodeClick={(_, node) => props.onSelectNode(node.id)}
        onNodeDragStop={(_, node) => {
          props.onReplaceCanvas(
            transientNodes.map((candidate) =>
              candidate.id === node.id ? { ...candidate, position: { ...node.position } } : candidate,
            ),
            edges,
          );
        }}
        onPaneClick={() => props.onSelectNode(null)}
        onNodesDelete={(deleted) => deleted.forEach((node) => props.onDeleteNode(node.id))}
        onDrop={drop}
        onDragOver={(event) => {
          event.preventDefault();
          event.dataTransfer.dropEffect = "copy";
        }}
        onMoveEnd={(_, viewport) => props.onViewport(viewport)}
        defaultViewport={props.document.viewport}
        snapToGrid
        snapGrid={[16, 16]}
        fitView={props.document.revision === 0}
        deleteKeyCode={["Backspace", "Delete"]}
      >
        <Background gap={16} />
        <MiniMap pannable zoomable />
        <Controls showInteractive={false} />
      </ReactFlow>
    </div>
  );
}

export function FlowCanvas(props: FlowCanvasProps) {
  return (
    <ReactFlowProvider>
      <FlowCanvasInner {...props} />
    </ReactFlowProvider>
  );
}
