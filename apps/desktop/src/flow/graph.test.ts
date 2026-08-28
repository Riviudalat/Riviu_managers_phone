import { describe, expect, it } from "vitest";
import { createFlowNode, newFlowDocument, type IdFactory } from "./model";
import {
  deleteExecutableNode,
  initialLaunchBundleId,
  insertNodeOnEdge,
  reconnectEdge,
  toCanvas,
  withCanvasLayout,
} from "./graph";

function sequentialIds(prefix: string): IdFactory {
  let next = 0;
  return () => `${prefix}-${++next}`;
}

describe("Flow graph mapping", () => {
  it("maps domain identifiers and strips React-only state on round-trip", () => {
    const document = newFlowDocument("Fixture", sequentialIds("fixture"));
    const canvas = toCanvas(document, [
      { code: "FixtureIssue", message: "fixture", nodeId: document.entryNodeId },
    ]);
    canvas.nodes[0] = {
      ...canvas.nodes[0],
      selected: true,
      measured: { width: 120, height: 48 },
      position: { x: 15, y: 25 },
    };

    const roundTrip = withCanvasLayout(document, canvas.nodes, canvas.edges);

    expect(roundTrip.nodes[0].position).toEqual({ x: 15, y: 25 });
    expect(roundTrip.nodes[0]).not.toHaveProperty("selected");
    expect(roundTrip.nodes[0]).not.toHaveProperty("measured");
    expect(roundTrip.nodes[0]).not.toHaveProperty("issues");
    expect(canvas.nodes[0].data.issues).toHaveLength(1);
  });

  it("atomically splits and rejoins a selected edge", () => {
    const document = newFlowDocument("Fixture", sequentialIds("base"));
    const node = createFlowNode("wait", { x: 160, y: 80 }, sequentialIds("node"));
    const inserted = insertNodeOnEdge(
      document,
      document.edges[0].id,
      node,
      "flow",
      sequentialIds("insert-edge"),
    );

    expect(inserted.nodes).toHaveLength(3);
    expect(inserted.edges).toHaveLength(2);
    expect(inserted.edges.map((edge) => [edge.sourceNodeId, edge.targetNodeId])).toEqual([
      [document.nodes[0].id, node.id],
      [node.id, document.nodes[1].id],
    ]);

    const deleted = deleteExecutableNode(inserted, node.id, sequentialIds("join-edge"));
    expect(deleted.nodes).toHaveLength(2);
    expect(deleted.edges).toEqual([
      expect.objectContaining({
        sourceNodeId: document.nodes[0].id,
        targetNodeId: document.nodes[1].id,
      }),
    ]);
  });

  it("reconnects only to existing distinct nodes", () => {
    const document = newFlowDocument("Fixture", sequentialIds("base"));
    const edgeId = document.edges[0].id;
    expect(reconnectEdge(document, edgeId, "missing", document.nodes[1].id)).toBe(document);
    expect(
      reconnectEdge(document, edgeId, document.nodes[0].id, document.nodes[0].id),
    ).toBe(document);

    const third = createFlowNode("wait", { x: 200, y: 100 }, sequentialIds("third"));
    const withThird = { ...document, nodes: [...document.nodes, third] };
    const reconnected = reconnectEdge(withThird, edgeId, third.id, document.nodes[1].id);
    expect(reconnected.edges[0].sourceNodeId).toBe(third.id);
  });
});

describe("the bundle the picker needs comes from the document", () => {
  it("reads the launch bundle one hop from the entry node", () => {
    // It used to come from `compiled.plan.contextPlan.initialBundleId`, and every edit clears
    // `compiled` — so the coordinate picker was disabled exactly when it was needed: a fresh Swipe
    // has no from/to, cannot compile without them, and the button that fills them in was off
    // because compilation had failed.
    const document = newFlowDocument("Launch", sequentialIds("base"));
    const launch = createFlowNode("launchApp", { x: 100, y: 0 }, sequentialIds("launch"));
    launch.config = { bundleId: "com.example.fixture" };
    const inserted = insertNodeOnEdge(
      document,
      document.edges[0].id,
      launch,
      "flow",
      sequentialIds("split"),
    );
    expect(initialLaunchBundleId(inserted)).toBe("com.example.fixture");
  });

  it("answers null when the first action is not a launch, or the bundle is blank", () => {
    const document = newFlowDocument("No launch", sequentialIds("base"));
    expect(initialLaunchBundleId(document)).toBeNull();

    const wait = createFlowNode("wait", { x: 100, y: 0 }, sequentialIds("wait"));
    expect(
      initialLaunchBundleId(
        insertNodeOnEdge(document, document.edges[0].id, wait, "flow", sequentialIds("a")),
      ),
    ).toBeNull();

    const blank = createFlowNode("launchApp", { x: 100, y: 0 }, sequentialIds("blank"));
    blank.config = { bundleId: "   " };
    expect(
      initialLaunchBundleId(
        insertNodeOnEdge(document, document.edges[0].id, blank, "flow", sequentialIds("b")),
      ),
    ).toBeNull();
  });
});
