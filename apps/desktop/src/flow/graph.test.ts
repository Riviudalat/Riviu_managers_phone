import { describe, expect, it } from "vitest";
import { createFlowNode, newFlowDocument, type IdFactory } from "./model";
import {
  deleteExecutableNode,
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
