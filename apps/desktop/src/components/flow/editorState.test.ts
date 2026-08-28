import { describe, expect, it } from "vitest";
import type {
  CompiledRevision,
  FlowDocumentV2,
  FlowRevisionRecord,
  FlowValidationIssue,
} from "../../types";
import {
  canStartSave,
  createFlowNode,
  duplicateDocument,
  initialEditorState,
  newFlowDocument,
  reduce,
  toCanvas,
  toDocument,
} from "./editorState";

function sequentialIds(prefix = "id") {
  let next = 0;
  return () => `${prefix}-${++next}`;
}

function fixtureDocument(name = "Fixture"): FlowDocumentV2 {
  return newFlowDocument(name, sequentialIds());
}

function compiledFixture(document: FlowDocumentV2): CompiledRevision {
  return {
    plan: {
      schemaVersion: 2,
      flowId: document.id,
      revision: document.revision,
      nodes: {},
      executionOrder: document.nodes.map((node) => node.id),
      contextPlan: {
        requiresExclusive: false,
        requiresUiSession: false,
        requiresStream: false,
        requiresFreshTextSession: false,
        initialBundleId: null,
      },
      actionDefinitionVersions: {},
      requiredCapabilities: [],
    },
    canonicalJson: "{}",
    sha256: "11".repeat(32),
  };
}

function savedRecord(document: FlowDocumentV2, revision = document.revision + 1): FlowRevisionRecord {
  const savedDocument = { ...structuredClone(document), revision };
  return {
    document: savedDocument,
    compiledPlan: { ...compiledFixture(savedDocument).plan, revision },
    planHash: "22".repeat(32),
    createdAt: "2026-07-31T00:00:00Z",
  };
}

describe("flow editor graph", () => {
  it("creates a stable Start-to-End draft and inserts, reconnects, then deletes an action", () => {
    const ids = sequentialIds();
    const document = newFlowDocument("New", ids);
    const [start, end] = document.nodes;
    expect(document.entryNodeId).toBe(start.id);
    expect(document.nodes.map((node) => node.kind)).toEqual(["start", "end"]);

    const wait = createFlowNode("wait", { x: 160, y: 80 }, ids);
    let state = initialEditorState(document, false);
    state = reduce(state, {
      type: "insertNode",
      edgeId: document.edges[0].id,
      node: wait,
      sourcePort: "flow",
      idFactory: ids,
    });
    expect(state.document.nodes.map((node) => node.id)).toEqual([start.id, end.id, wait.id]);
    expect(state.document.edges).toHaveLength(2);
    expect(state.document.edges.map((edge) => [edge.sourceNodeId, edge.targetNodeId])).toEqual([
      [start.id, wait.id],
      [wait.id, end.id],
    ]);

    state = reduce(state, {
      type: "reconnectEdge",
      edgeId: state.document.edges[1].id,
      sourceNodeId: start.id,
      targetNodeId: end.id,
    });
    expect(state.document.edges[1]).toMatchObject({
      sourceNodeId: start.id,
      targetNodeId: end.id,
    });

    state = reduce(state, {
      type: "deleteSelection",
      nodeIds: [wait.id],
      edgeIds: [],
      idFactory: ids,
    });
    expect(state.document.nodes.some((node) => node.id === wait.id)).toBe(false);
    expect(state.document.nodes.map((node) => node.id)).toEqual([start.id, end.id]);
    // The point of the delete is the path it leaves behind, not the node it removes. Asserting
    // only the disappearance is what let the edge-first callback order go unnoticed.
    expect(state.document.edges).toHaveLength(1);
    expect(state.document.edges[0]).toMatchObject({
      sourceNodeId: start.id,
      targetNodeId: end.id,
    });
  });

  it("duplicates a flow with no shared domain identifiers", () => {
    const source = fixtureDocument("Source");
    const duplicate = duplicateDocument(source, "Copy", sequentialIds("copy"));
    expect(duplicate.id).not.toBe(source.id);
    expect(duplicate.revision).toBe(0);
    expect(new Set(duplicate.nodes.map((node) => node.id))).not.toEqual(
      new Set(source.nodes.map((node) => node.id)),
    );
    expect(new Set(duplicate.edges.map((edge) => edge.id))).not.toEqual(
      new Set(source.edges.map((edge) => edge.id)),
    );
    const ids = new Set(duplicate.nodes.map((node) => node.id));
    expect(ids.has(duplicate.entryNodeId)).toBe(true);
    expect(
      duplicate.edges.every(
        (edge) => ids.has(edge.sourceNodeId) && ids.has(edge.targetNodeId),
      ),
    ).toBe(true);
  });

  it("round-trips domain data and strips React-only canvas state", () => {
    const document = fixtureDocument();
    const issue: FlowValidationIssue = {
      code: "FixtureIssue",
      message: "fixture",
      nodeId: document.nodes[0].id,
    };
    const canvas = toCanvas(document, [issue]);
    const editedNodes = canvas.nodes.map((node, index) => ({
      ...node,
      selected: true,
      dragging: true,
      measured: { width: 240, height: 72 },
      position: { x: node.position.x + 10, y: node.position.y + 20 },
      data:
        index === 0
          ? { ...node.data, config: { marker: "updated" }, postcondition: null }
          : node.data,
    }));
    const roundTrip = toDocument(document, editedNodes, canvas.edges);

    expect(roundTrip.nodes[0].position).toEqual({ x: 10, y: 100 });
    expect(roundTrip.nodes[0].config).toEqual({ marker: "updated" });
    expect(roundTrip).not.toHaveProperty("selected");
    expect(roundTrip.nodes[0]).not.toHaveProperty("selected");
    expect(roundTrip.nodes[0]).not.toHaveProperty("measured");
  });
});

describe("flow editor reducer", () => {
  it("invalidates compilation and advances an epoch for layout, config, and postcondition edits", () => {
    const document = fixtureDocument();
    const identity = { requestId: 1, flowId: document.id, documentEpoch: 0 };
    let state = reduce(initialEditorState(document, false), {
      type: "validationStarted",
      identity,
    });
    state = reduce(state, {
      type: "validationCompleted",
      identity,
      issues: [],
      compiled: compiledFixture(document),
    });
    expect(state.compiled?.identity).toEqual(identity);

    const startId = state.document.nodes[0].id;
    state = reduce(state, { type: "moveNode", nodeId: startId, position: { x: 4, y: 5 } });
    expect(state.documentEpoch).toBe(1);
    expect(state.compiled).toBeNull();
    expect(state.validation).toEqual([]);
    state = reduce(state, {
      type: "updateNodeConfig",
      nodeId: startId,
      config: { marker: 1 },
    });
    expect(state.documentEpoch).toBe(2);
    state = reduce(state, {
      type: "updateNodePostcondition",
      nodeId: startId,
      postcondition: { kind: "frameDigestChanged", minimumDistance: 1 },
    });
    expect(state.documentEpoch).toBe(3);
    expect(state.dirty).toBe(true);
  });

  it("bounds undo and redo history at 50 immutable documents", () => {
    let state = initialEditorState(fixtureDocument(), false);
    for (let index = 0; index < 55; index += 1) {
      state = reduce(state, { type: "renameFlow", name: `Name ${index}` });
    }
    expect(state.past).toHaveLength(50);
    expect(state.documentEpoch).toBe(55);

    for (let index = 0; index < 50; index += 1) state = reduce(state, { type: "undo" });
    expect(state.future).toHaveLength(50);
    expect(state.document.name).toBe("Name 4");
    expect(state.documentEpoch).toBe(105);
    expect(reduce(state, { type: "undo" })).toBe(state);

    state = reduce(state, { type: "redo" });
    expect(state.document.name).toBe("Name 5");
    expect(state.past).toHaveLength(1);
  });

  it("resets dirty/history from a server document without resetting the monotonic epoch", () => {
    const original = fixtureDocument();
    let state = reduce(initialEditorState(original, false), { type: "renameFlow", name: "Local" });
    const server = { ...fixtureDocument("Server"), revision: 7 };
    state = reduce(state, { type: "replaceDocument", document: server, source: "server" });
    expect(state.document).toEqual(server);
    expect(state.dirty).toBe(false);
    expect(state.past).toEqual([]);
    expect(state.future).toEqual([]);
    expect(state.documentEpoch).toBe(2);
  });

  it("ignores validation and save completions for an older draft epoch", () => {
    const document = fixtureDocument();
    const identity = { requestId: 7, flowId: document.id, documentEpoch: 0 };
    const started = reduce(initialEditorState(document), {
      type: "validationStarted",
      identity,
    });
    const edited = reduce(started, { type: "renameFlow", name: "Edited" });
    const validated = reduce(edited, {
      type: "validationCompleted",
      identity,
      issues: [],
      compiled: compiledFixture(document),
    });
    expect(validated.compiled).toBeNull();

    const valid = reduce(
      reduce(initialEditorState(document), { type: "validationStarted", identity }),
      {
        type: "validationCompleted",
        identity,
        issues: [],
        compiled: compiledFixture(document),
      },
    );
    const saving = reduce(valid, { type: "saveStarted", identity });
    const newerDraft = reduce(saving, { type: "renameFlow", name: "Edited during save" });
    const afterStaleSave = reduce(newerDraft, {
      type: "saveCompleted",
      identity,
      record: savedRecord(document),
    });
    expect(afterStaleSave.document.name).toBe("Edited during save");
    expect(afterStaleSave.dirty).toBe(true);
    expect(afterStaleSave.saveRequest).toBeNull();
    expect(afterStaleSave.notice).toEqual({
      code: "SaveCompletedForOlderDraft",
      savedRevision: 1,
    });
  });

  it("starts a save only from the current successful compilation identity", () => {
    const document = fixtureDocument();
    const identity = { requestId: 3, flowId: document.id, documentEpoch: 0 };
    let state = initialEditorState(document);
    expect(canStartSave(state, identity)).toBe(false);
    state = reduce(state, { type: "validationStarted", identity });
    state = reduce(state, {
      type: "validationCompleted",
      identity,
      issues: [],
      compiled: compiledFixture(document),
    });
    expect(canStartSave(state, identity)).toBe(true);

    const oldIdentity = identity;
    state = reduce(state, { type: "renameFlow", name: "Changed" });
    expect(canStartSave(state, oldIdentity)).toBe(false);
    expect(reduce(state, { type: "saveStarted", identity: oldIdentity })).toBe(state);
  });

  it("applies an identity-current save and marks the server revision clean", () => {
    const document = fixtureDocument();
    const identity = { requestId: 4, flowId: document.id, documentEpoch: 0 };
    let state = reduce(initialEditorState(document), { type: "validationStarted", identity });
    state = reduce(state, {
      type: "validationCompleted",
      identity,
      issues: [],
      compiled: compiledFixture(document),
    });
    state = reduce(state, { type: "saveStarted", identity });
    const record = savedRecord(document, 3);
    state = reduce(state, { type: "saveCompleted", identity, record });
    expect(state.document.revision).toBe(3);
    expect(state.dirty).toBe(false);
    expect(state.documentEpoch).toBe(1);
    expect(state.past).toEqual([]);
  });

  it("rejects NaN and other non-finite numeric mutations", () => {
    const state = initialEditorState(fixtureDocument(), false);
    const nodeId = state.document.nodes[0].id;
    expect(
      reduce(state, { type: "moveNode", nodeId, position: { x: Number.NaN, y: 0 } }),
    ).toBe(state);
    expect(
      reduce(state, {
        type: "setViewport",
        viewport: { x: 0, y: 0, zoom: Number.POSITIVE_INFINITY },
      }),
    ).toBe(state);
    expect(
      reduce(state, {
        type: "updateNodeConfig",
        nodeId,
        config: { durationMs: Number.NaN },
      }),
    ).toBe(state);
    expect(
      reduce(state, {
        type: "updateNodePostcondition",
        nodeId,
        postcondition: { kind: "frameDigestChanged", minimumDistance: Number.NEGATIVE_INFINITY },
      }),
    ).toBe(state);
  });
});
