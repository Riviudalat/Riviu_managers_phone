import { describe, expect, it } from "vitest";
import { duplicateDocument, newFlowDocument, type IdFactory } from "./model";

function sequentialIds(prefix: string): IdFactory {
  let next = 0;
  return () => `${prefix}-${++next}`;
}

describe("Flow model", () => {
  it("creates a linear Start to End document", () => {
    const document = newFlowDocument("Fixture", sequentialIds("new"));

    expect(document).toMatchObject({ schemaVersion: 2, name: "Fixture", revision: 0 });
    expect(document.nodes.map((node) => node.kind)).toEqual(["start", "end"]);
    expect(document.entryNodeId).toBe(document.nodes[0].id);
    expect(document.edges).toEqual([
      expect.objectContaining({
        sourceNodeId: document.nodes[0].id,
        targetNodeId: document.nodes[1].id,
        sourcePort: "flow",
        targetPort: "flow",
      }),
    ]);
  });

  it("duplicates without sharing flow, node, or edge identifiers", () => {
    const source = newFlowDocument("Source", sequentialIds("source"));
    const duplicate = duplicateDocument(source, "Copy", sequentialIds("copy"));

    expect(duplicate.id).not.toBe(source.id);
    expect(duplicate.revision).toBe(0);
    expect(duplicate.name).toBe("Copy");
    expect(new Set(duplicate.nodes.map((node) => node.id))).not.toEqual(
      new Set(source.nodes.map((node) => node.id)),
    );
    expect(new Set(duplicate.edges.map((edge) => edge.id))).not.toEqual(
      new Set(source.edges.map((edge) => edge.id)),
    );
    const duplicateNodeIds = new Set(duplicate.nodes.map((node) => node.id));
    expect(duplicateNodeIds.has(duplicate.entryNodeId)).toBe(true);
    expect(
      duplicate.edges.every(
        (edge) =>
          duplicateNodeIds.has(edge.sourceNodeId) && duplicateNodeIds.has(edge.targetNodeId),
      ),
    ).toBe(true);
  });
});
