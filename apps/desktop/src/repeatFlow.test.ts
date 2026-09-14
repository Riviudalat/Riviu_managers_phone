import { describe, expect, it } from "vitest";
import { analyzeFlowRepeat, repeatLinearFlow } from "./repeatFlow";
import { initialEditorState, reduceFlowEditor } from "./flow/draft";
import type { FlowDocumentV2 } from "./types";

const source: FlowDocumentV2 = {
  schemaVersion: 2, id: "flow", name: "Routine", revision: 3, entryNodeId: "s", viewport: { x: 0, y: 0, zoom: 1 },
  nodes: [
    { id: "s", kind: "start", config: {}, position: { x: 0, y: 0 } },
    { id: "a", kind: "launchApp", config: { bundleId: "com.example.app" }, postcondition: { kind: "activeAppEquals", bundleId: "com.example.app" }, position: { x: 100, y: 0 } },
    { id: "w", kind: "wait", config: { durationMs: 500 }, position: { x: 200, y: 0 } },
    { id: "e", kind: "end", config: {}, position: { x: 300, y: 0 } },
  ],
  edges: [["s", "a"], ["a", "w"], ["w", "e"]].map(([from, to], index) => ({ id: `edge${index}`, sourceNodeId: from, sourcePort: "flow", targetNodeId: to, targetPort: "flow" })),
};
const idFactory = () => { let count = 0; return () => `fresh-${++count}`; };

describe("bounded repeat expansion", () => {
  it("follows graph order and creates one distinct action ID per iteration", () => {
    const shuffled = structuredClone(source);
    shuffled.nodes.reverse();
    const result = repeatLinearFlow(shuffled, 3, idFactory());
    expect(result.nodes.map((item) => item.kind)).toEqual(["start", "launchApp", "wait", "launchApp", "wait", "launchApp", "wait", "end"]);
    expect(new Set(result.nodes.map((item) => item.id)).size).toBe(8);
    expect(result.nodes[1].id).toBe("a");
    expect(result.nodes[3].postcondition).toEqual(source.nodes[1].postcondition);
    expect(result.edges.map((edge) => [edge.sourceNodeId, edge.targetNodeId])).toEqual(result.nodes.slice(1).map((item, index) => [result.nodes[index].id, item.id]));
    expect(result.id).toBe(source.id);
    expect(result.revision).toBe(source.revision);
    expect(shuffled.nodes[0].id).toBe("e");
  });

  it("copies configs and evidence independently and includes explicit waits in preview", () => {
    const result = repeatLinearFlow(source, 2, idFactory());
    result.nodes[3].config.bundleId = "changed";
    expect(source.nodes[1].config.bundleId).toBe("com.example.app");
    expect(result.nodes[1].config.bundleId).toBe("com.example.app");
    expect(analyzeFlowRepeat(source, 3)).toMatchObject({ totalNodeCount: 8, waitDurationMs: 1500, error: null });
  });

  it.each([0, 51, 2.1, NaN, Infinity])("rejects invalid count %s", (count) => expect(() => repeatLinearFlow(source, count)).toThrow());

  it("refuses to expand condition branches or disconnected nodes", () => {
    const branch = structuredClone(source);
    branch.nodes[1].kind = "ifVision";
    expect(analyzeFlowRepeat(branch, 2).error).toContain("nhánh điều kiện");
    const dangling = structuredClone(source);
    dangling.edges.pop();
    expect(analyzeFlowRepeat(dangling, 2).error).not.toBeNull();
    const orphan = structuredClone(source);
    orphan.nodes.push({ ...orphan.nodes[2], id: "orphan" });
    expect(analyzeFlowRepeat(orphan, 2).error).not.toBeNull();
  });

  it("rejects cycles, wrong ports, duplicate IDs and ID factory collisions", () => {
    const cycle = structuredClone(source);
    cycle.edges[2].targetNodeId = "a";
    expect(analyzeFlowRepeat(cycle, 2).error).not.toBeNull();
    const port = structuredClone(source);
    port.edges[1].sourcePort = "matched";
    expect(analyzeFlowRepeat(port, 2).error).not.toBeNull();
    const duplicate = structuredClone(source);
    duplicate.nodes[2].id = "a";
    expect(analyzeFlowRepeat(duplicate, 2).error).not.toBeNull();
    expect(() => repeatLinearFlow(source, 2, () => "a")).toThrow();
  });

  it("caps total nodes including Start and End", () => {
    const large = repeatLinearFlow(source, 50, idFactory());
    expect(large.nodes).toHaveLength(102);
    expect(analyzeFlowRepeat(large, 5).error).toContain("502");
    expect(analyzeFlowRepeat(large, 4).totalNodeCount).toBe(402);
  });

  it("keeps count one unchanged", () => expect(repeatLinearFlow(source, 1)).toEqual(source));

  it("applies and undoes all repetitions as a single editor mutation", () => {
    const initial = initialEditorState(source, false);
    const edited = reduceFlowEditor(initial, { type: "applyDocumentEdit", document: repeatLinearFlow(source, 3, idFactory()) });
    expect(edited.document.nodes).toHaveLength(8);
    expect(edited.past).toHaveLength(1);
    expect(edited.dirty).toBe(true);
    expect(edited.compiled).toBeNull();
    const undone = reduceFlowEditor(edited, { type: "undo" });
    expect(undone.document).toEqual(source);
    expect(undone.dirty).toBe(false);
    expect(reduceFlowEditor(undone, { type: "redo" }).document).toEqual(edited.document);
  });

  it("does not apply a document with stale revision, different ID or invalid numbers", () => {
    const initial = initialEditorState(source, false);
    expect(reduceFlowEditor(initial, { type: "applyDocumentEdit", document: { ...source, id: "another" } })).toBe(initial);
    expect(reduceFlowEditor(initial, { type: "applyDocumentEdit", document: { ...source, revision: 2 } })).toBe(initial);
    const invalid = structuredClone(source);
    invalid.nodes[1].config.bad = NaN;
    expect(reduceFlowEditor(initial, { type: "applyDocumentEdit", document: invalid })).toBe(initial);
  });
});
