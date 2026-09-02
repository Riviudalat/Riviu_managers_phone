import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { EdgeChange, NodeChange } from "@xyflow/react";
import { FlowCanvas } from "./FlowCanvas";
import { createFlowNode, newFlowDocument, type IdFactory } from "../../flow/model";
import type { FlowCanvasEdge, FlowCanvasNode } from "../../flow/graph";
import type { ActionDefinition } from "../../types";

// The real `applyNodeChanges`/`applyEdgeChanges`/`addEdge` stay — they are the pure
// arithmetic under test. Only the rendering half is stubbed, because the point of these
// tests is the handler wiring, and ReactFlow's canvas needs a real layout engine jsdom
// does not have.
let canvasProps: Record<string, unknown> = {};
vi.mock("@xyflow/react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@xyflow/react")>();
  return {
    ...actual,
    ReactFlow: (props: Record<string, unknown>) => {
      canvasProps = props;
      return <div data-testid="flow-canvas-stub" />;
    },
    ReactFlowProvider: ({ children }: { children?: React.ReactNode }) => <>{children}</>,
    Background: () => null,
    Controls: () => null,
    MiniMap: () => null,
    useReactFlow: () => ({
      screenToFlowPosition: (point: { x: number; y: number }) => point,
    }),
  };
});

function sequentialIds(prefix: string): IdFactory {
  let next = 0;
  return () => `${prefix}-${++next}`;
}

function twoWaitDocument() {
  const document = newFlowDocument("Canvas fixture", sequentialIds("base"));
  const first = { ...createFlowNode("wait", { x: 100, y: 100 }, sequentialIds("first")) };
  const second = { ...createFlowNode("wait", { x: 300, y: 100 }, sequentialIds("second")) };
  return {
    document: { ...document, nodes: [...document.nodes, first, second] },
    first,
    second,
  };
}

function renderCanvas(document: ReturnType<typeof twoWaitDocument>["document"]) {
  const onReplaceCanvas = vi.fn();
  render(
    <FlowCanvas
      document={document}
      catalog={[]}
      issues={[]}
      selectedNodeId={null}
      onSelectNode={vi.fn()}
      onReplaceCanvas={onReplaceCanvas}
      onInsertNode={vi.fn()}
      onAppendNode={vi.fn()}
      onDeleteSelection={vi.fn()}
      onViewport={vi.fn()}
    />,
  );
  return onReplaceCanvas;
}

function settledMove(id: string, x: number, y: number): NodeChange<FlowCanvasNode>[] {
  return [{ type: "position", id, position: { x, y }, dragging: false }];
}

/// A document with two edges, so a tick can carry two structural edge changes.
function twoEdgeDocument() {
  const { document, first, second } = twoWaitDocument();
  const [start, end] = document.nodes;
  return {
    document: {
      ...document,
      edges: [
        {
          id: "edge-a",
          sourceNodeId: start.id,
          sourcePort: "flow",
          targetNodeId: first.id,
          targetPort: "flow",
        },
        {
          id: "edge-b",
          sourceNodeId: second.id,
          sourcePort: "flow",
          targetNodeId: end.id,
          targetPort: "flow",
        },
      ],
    },
    first,
  };
}

describe("FlowCanvas batched node changes", () => {
  it("inserts on an edge selected in the same tick as the drop", () => {
    const { document } = twoEdgeDocument();
    const onInsertNode = vi.fn();
    const waitAction: ActionDefinition = {
      kind: "wait",
      schemaVersion: 1,
      label: "Chờ",
      disabledReason: null,
      category: "timing",
      configSchema: { type: "object", properties: {} },
      inputPorts: [{ name: "flow", valueType: "flow", required: true }],
      outputPorts: [{ name: "flow", valueType: "flow", required: true }],
      requiredCapabilities: [],
      resourceClass: "pureDesktop",
      sideEffectClass: "none",
      evidenceRequirement: "none",
      allowedEvidence: [],
      qualifiedDetectorIds: [],
      reconciliationPolicy: "none",
      defaultTimeoutMs: 1_000,
      retryPolicy: "beforeDispatchOnly",
    };
    render(
      <FlowCanvas
        document={document}
        catalog={[waitAction]}
        issues={[]}
        selectedNodeId={null}
        onSelectNode={vi.fn()}
        onReplaceCanvas={vi.fn()}
        onInsertNode={onInsertNode}
        onAppendNode={vi.fn()}
        onDeleteSelection={vi.fn()}
        onViewport={vi.fn()}
      />,
    );
    const onEdgesChange = canvasProps.onEdgesChange as (
      changes: EdgeChange<FlowCanvasEdge>[],
    ) => void;
    const onDrop = canvasProps.onDrop as (event: {
      preventDefault: () => void;
      dataTransfer: { getData: () => string };
      clientX: number;
      clientY: number;
    }) => void;

    act(() => {
      onEdgesChange([{ type: "select", id: "edge-a", selected: true }]);
      onDrop({
        preventDefault: vi.fn(),
        dataTransfer: { getData: () => "wait" },
        clientX: 800,
        clientY: 800,
      });
    });

    expect(onInsertNode).toHaveBeenCalledOnce();
    expect(onInsertNode).toHaveBeenCalledWith(
      "edge-a",
      expect.objectContaining({ kind: "wait" }),
      "flow",
    );
  });

  /// Two `onNodesChange` calls in one React tick — a keyboard move landing beside another
  /// change — must BOTH survive into the commit. The captured-state version of the handler
  /// computed each call from last render's array, so the second call silently discarded
  /// the first call's movement and committed the half-merged canvas to the document.
  it("keeps both movements when two changes land in one tick", () => {
    const { document, first, second } = twoWaitDocument();
    const onReplaceCanvas = renderCanvas(document);
    const onNodesChange = canvasProps.onNodesChange as (
      changes: NodeChange<FlowCanvasNode>[],
    ) => void;

    act(() => {
      onNodesChange(settledMove(first.id, 111, 222));
      onNodesChange(settledMove(second.id, 333, 444));
    });

    const [nodes] = onReplaceCanvas.mock.lastCall as [FlowCanvasNode[], unknown];
    const positions = new Map(nodes.map((node) => [node.id, node.position]));
    expect(positions.get(first.id)).toEqual({ x: 111, y: 222 });
    expect(positions.get(second.id)).toEqual({ x: 333, y: 444 });
  });

  it("commits the merged canvas, not one call's view of it, on the final change", () => {
    const { document, first, second } = twoWaitDocument();
    const onReplaceCanvas = renderCanvas(document);
    const onNodesChange = canvasProps.onNodesChange as (
      changes: NodeChange<FlowCanvasNode>[],
    ) => void;

    act(() => {
      // A drag still in flight commits nothing…
      onNodesChange([
        { type: "position", id: first.id, position: { x: 50, y: 60 }, dragging: true },
      ]);
      // …and the settling change in the same tick must carry the in-flight move too.
      onNodesChange(settledMove(second.id, 70, 80));
    });

    expect(onReplaceCanvas).toHaveBeenCalledTimes(1);
    const [nodes] = onReplaceCanvas.mock.lastCall as [FlowCanvasNode[], unknown];
    const positions = new Map(nodes.map((node) => [node.id, node.position]));
    expect(positions.get(first.id)).toEqual({ x: 50, y: 60 });
    expect(positions.get(second.id)).toEqual({ x: 70, y: 80 });
  });

  /// The same tick, the same hazard, the other half of the canvas — and the half the first
  /// fix missed. `selectedEdgesRef` made the *selection* synchronous while `edgesNow()` still
  /// rebuilt from `graph.edges`, a memo over `props.document` that cannot change until the
  /// parent re-renders. So two structural edge changes in one tick both started from the same
  /// base: deleting two edges in one gesture committed a canvas with one of them back.
  it("keeps both edge removals when two structural changes land in one tick", () => {
    const { document } = twoEdgeDocument();
    const onReplaceCanvas = renderCanvas(document);
    const onEdgesChange = canvasProps.onEdgesChange as (
      changes: EdgeChange<FlowCanvasEdge>[],
    ) => void;

    act(() => {
      onEdgesChange([{ type: "remove", id: "edge-a" }]);
      onEdgesChange([{ type: "remove", id: "edge-b" }]);
    });

    const [, edges] = onReplaceCanvas.mock.lastCall as [unknown, FlowCanvasEdge[]];
    expect(
      edges.map((edge) => edge.id),
      "the second removal computed from last render's edges, so the first edge came back",
    ).toEqual([]);
  });

  /// An edge commit carries the tick's node moves too: `changeEdges` hands the document
  /// `nodesNow()`, and nothing exercised that argument before this.
  it("carries a node move made in the same tick into an edge commit", () => {
    const { document, first } = twoEdgeDocument();
    const onReplaceCanvas = renderCanvas(document);
    const onNodesChange = canvasProps.onNodesChange as (
      changes: NodeChange<FlowCanvasNode>[],
    ) => void;
    const onEdgesChange = canvasProps.onEdgesChange as (
      changes: EdgeChange<FlowCanvasEdge>[],
    ) => void;

    act(() => {
      onNodesChange(settledMove(first.id, 11, 22));
      onEdgesChange([{ type: "remove", id: "edge-a" }]);
    });

    const [nodes, edges] = onReplaceCanvas.mock.lastCall as [
      FlowCanvasNode[],
      FlowCanvasEdge[],
    ];
    const positions = new Map(nodes.map((node) => [node.id, node.position]));
    expect(positions.get(first.id)).toEqual({ x: 11, y: 22 });
    expect(edges.map((edge) => edge.id)).toEqual(["edge-b"]);
  });
});
