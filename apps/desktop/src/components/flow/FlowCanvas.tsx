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
import { pushToast } from "../../toastStore";

const NODE_TYPES: NodeTypes = { flowAction: FlowActionNode };

interface FlowCanvasProps {
  document: FlowDocumentV2;
  catalog: ActionDefinition[];
  issues: FlowValidationIssue[];
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string | null) => void;
  onReplaceCanvas: (nodes: FlowCanvasNode[], edges: FlowCanvasEdge[]) => void;
  onInsertNode: (edgeId: string, node: FlowNode, sourcePort: string) => void;
  onAppendNode: (node: FlowNode) => void;
  onDeleteSelection: (nodeIds: string[], edgeIds: string[]) => void;
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
  // Drop the ids of edges that no longer exist.
  //
  // The drop handler prefers a single selected edge over geometric hit testing, and this set was
  // never pruned. So: select an edge, insert a Wait on it (the edge becomes two new ones), then drop
  // another action anywhere -- the stale id won, `insertNodeOnEdge` could not find it, the reducer
  // returned the same document, and the dropped node simply vanished with no message.
  useEffect(() => {
    setSelectedEdges((current) => {
      if (current.size === 0) return current;
      const alive = new Set(
        [...current].filter((id) => graph.edges.some((edge) => edge.id === id)),
      );
      return alive.size === current.size ? current : alive;
    });
  }, [graph.edges]);
  const nodes = transientNodes.map((node) => ({
    ...node,
    selected: node.id === props.selectedNodeId,
  }));
  const edges = graph.edges.map((edge) => ({
    ...edge,
    selected: selectedEdges.has(edge.id),
  }));

  const replaceAfterNodeChanges = (changes: NodeChange<FlowCanvasNode>[]) => {
    const next = applyNodeChanges(changes, transientNodes);
    setTransientNodes(next);
    const selected = changes.find(
      (change): change is Extract<NodeChange<FlowCanvasNode>, { type: "select" }> =>
        change.type === "select" && change.selected,
    );
    if (selected) props.onSelectNode(selected.id);
    // **A position change that is not part of a drag is already final, so commit it.**
    //
    // Positions used to be committed only by `onNodeDragStop`, and React Flow moves a selected
    // node with the arrow keys through `moveSelectedNodes` -> `updateNodePositions(items)`, which
    // emits a position change and no drag-stop event. The node visibly moved, the document never
    // heard about it: the draft stayed clean, a reload restored the old coordinates, and a save
    // that happened for some other reason wrote the old ones back so the node snapped.
    //
    // `dragging` is the discriminator and it is reliable in both directions: the drag handler
    // passes `true` while the pointer is down, and `updateNodePositions`'s second parameter
    // defaults to `false` — which is what the keyboard path relies on. React Flow also emits one
    // final change with `dragging: false` at drag end, *before* `onNodeDragStop`, and only when
    // the positions actually changed. So this one path covers mouse and keyboard both, and it is
    // one history entry per gesture rather than two — which is why `onNodeDragStop` no longer
    // commits anything.
    const settled = changes.some(
      (change) => change.type === "position" && change.dragging !== true,
    );
    if (settled) props.onReplaceCanvas(next, edges);
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

  /**
   * A drop that does nothing has to say why it did nothing.
   *
   * All three refusals below used to be a bare `return`. Dragging an action onto the canvas
   * and getting **no node, no message and no log** is the exact complaint this whole review
   * pass started from, and it is worse here than elsewhere: the operator has just performed a
   * deliberate gesture, so silence reads as "the app is broken" rather than "that action is
   * not available".
   *
   * It is also what made a CI failure unreadable. A Playwright drag that lost its
   * `DataTransfer` payload produced `0` nodes with nothing to explain it, and the DOM snapshot
   * showed a perfectly enabled palette button — so the evidence pointed nowhere. A refusal
   * that names itself would have said which of the three it was in one line.
   */
  const drop = (event: DragEvent) => {
    event.preventDefault();
    const kind = event.dataTransfer.getData(FLOW_ACTION_MIME) as ActionKind;
    if (!kind) {
      // The drag carried no action. Reachable from a real drag only when something outside the
      // palette is dropped on the canvas — a file, a selection, a link.
      pushToast(
        "warn",
        "Không nhận ra thứ được kéo vào",
        "Kéo một hành động từ bảng bên trái; thứ vừa thả không mang hành động nào.",
      );
      return;
    }
    const offered = props.catalog.find((action) => action.kind === kind);
    if (!offered) {
      pushToast("warn", "Hành động không có trong danh mục", `Kind: ${kind}`);
      return;
    }
    if (offered.disabledReason !== null) {
      // The palette already sets `disabled` and shows this text as a tooltip, but a tooltip is
      // not an answer to a gesture that just failed.
      pushToast("warn", `Chưa dùng được: ${offered.label}`, offered.disabledReason);
      return;
    }
    const position = screenToFlowPosition({ x: event.clientX, y: event.clientY });
    if (!Number.isFinite(position.x) || !Number.isFinite(position.y)) {
      // React Flow computes this from the viewport transform, which is measured by a
      // ResizeObserver. Before the first measurement lands the transform can be degenerate,
      // and then every dropped coordinate is `NaN`.
      pushToast(
        "warn",
        "Khung Flow chưa đo xong",
        "Thả lại sau một nhịp, hoặc bấm Fit View rồi thả.",
      );
      return;
    }
    const node = createFlowNode(kind, position);
    const exact = selectedEdges.size === 1 ? [...selectedEdges][0] : null;
    const edgeId = exact ?? nearestEdge(position, nodes, edges);
    const [only] = offered.outputPorts;
    if (edgeId !== null && offered.outputPorts.length !== 1) {
      // Splitting an edge wires the new node's output to whatever the edge used to reach, and that
      // only has one answer when the action has exactly one output. `ifVision` has two (`matched`,
      // `notMatched`) and `end` has none, so the split has no honest source port -- it used to
      // assume `"flow"` and draw a graph the compiler rejects. Drop it in unconnected instead and
      // say why, so the operator wires both branches themselves.
      pushToast(
        "warn",
        `${offered.label} không chèn được vào một cạnh`,
        offered.outputPorts.length === 0
          ? "Hành động này không có cổng ra."
          : `Hành động này có ${offered.outputPorts.length} cổng ra (${offered.outputPorts
              .map((port) => port.name)
              .join(", ")}) — đã thả vào canvas, hãy tự nối từng cổng.`,
      );
      props.onAppendNode(node);
      return;
    }
    if (edgeId !== null && only) props.onInsertNode(edgeId, node, only.name);
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
        onPaneClick={() => props.onSelectNode(null)}
        onBeforeDelete={({ nodes: removing, edges: removingEdges }) => {
          // Returning false cancels React Flow's own removal entirely, which is the only way to
          // get the whole gesture as one mutation: it fires `onEdgesChange` for the incident edges
          // *before* `onNodesDelete`, so letting it proceed meant the node delete ran on a
          // document that had already lost the edges it needed in order to reconnect the path.
          // Pure edge deletions have no such ordering problem and go through unchanged.
          if (removing.length === 0) return Promise.resolve(true);
          const nodeIds = new Set(removing.map((node) => node.id));
          props.onDeleteSelection(
            [...nodeIds],
            // Edges React Flow included only because they touch a deleted node are that node's
            // business; the rest were selected in their own right and must still go.
            removingEdges
              .filter((edge) => !nodeIds.has(edge.source) && !nodeIds.has(edge.target))
              .map((edge) => edge.id),
          );
          return Promise.resolve(false);
        }}
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
