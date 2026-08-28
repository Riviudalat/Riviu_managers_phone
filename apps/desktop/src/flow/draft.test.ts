import { describe, expect, it } from "vitest";
import type { CompiledRevision, FlowRevisionRecord } from "../types";
import { createFlowNode, newFlowDocument, type IdFactory } from "./model";
import {
  canStartSave,
  initialEditorState,
  reduceFlowEditor,
  type DocumentRequestIdentity,
} from "./draft";

function sequentialIds(prefix: string): IdFactory {
  let next = 0;
  return () => `${prefix}-${++next}`;
}

function compiledFixture(flowId: string, revision: number): CompiledRevision {
  return {
    plan: {
      schemaVersion: 2,
      flowId,
      revision,
      nodes: {},
      executionOrder: [],
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

function recordFixture(document: ReturnType<typeof newFlowDocument>): FlowRevisionRecord {
  const savedDocument = { ...document, revision: document.revision + 1 };
  return {
    document: savedDocument,
    compiledPlan: compiledFixture(savedDocument.id, savedDocument.revision).plan,
    planHash: "11".repeat(32),
    createdAt: "2026-07-31T00:00:00Z",
  };
}

describe("Flow editor reducer", () => {
  it("inserts, reconnects through deletion, and supports undo/redo", () => {
    const document = newFlowDocument("Fixture", sequentialIds("base"));
    const node = createFlowNode("wait", { x: 160, y: 80 }, sequentialIds("node"));
    let state = initialEditorState(document, false);
    state = reduceFlowEditor(state, {
      type: "insertNode",
      edgeId: document.edges[0].id,
      node,
      sourcePort: "flow",
      idFactory: sequentialIds("split"),
    });
    expect(state.document.nodes).toHaveLength(3);
    expect(state.documentEpoch).toBe(1);
    expect(state.dirty).toBe(true);

    state = reduceFlowEditor(state, { type: "undo" });
    expect(state.document.nodes).toHaveLength(2);
    expect(state.documentEpoch).toBe(2);
    state = reduceFlowEditor(state, { type: "redo" });
    expect(state.document.nodes).toHaveLength(3);
    expect(state.documentEpoch).toBe(3);
  });

  it("bounds history to 50 immutable documents", () => {
    let state = initialEditorState(newFlowDocument("0"), false);
    for (let index = 1; index <= 55; index += 1) {
      state = reduceFlowEditor(state, { type: "renameFlow", name: String(index) });
    }
    expect(state.past).toHaveLength(50);
    expect(state.past[0].name).toBe("5");
    expect(state.document.name).toBe("55");
  });

  it("rejects NaN before it can enter config, position, or viewport state", () => {
    const initial = initialEditorState(newFlowDocument("Fixture"));
    const nodeId = initial.document.nodes[0].id;
    expect(
      reduceFlowEditor(initial, {
        type: "updateNodeConfig",
        nodeId,
        config: { durationMs: Number.NaN },
      }),
    ).toBe(initial);
    expect(
      reduceFlowEditor(initial, {
        type: "moveNode",
        nodeId,
        position: { x: Number.NaN, y: 1 },
      }),
    ).toBe(initial);
    expect(
      reduceFlowEditor(initial, {
        type: "setViewport",
        viewport: { x: 0, y: 0, zoom: Number.POSITIVE_INFINITY },
      }),
    ).toBe(initial);
  });

  it("keeps pan and zoom out of the draft's dirty and compiled state", () => {
    // React Flow emits a move as soon as the canvas mounts. Treating that as an
    // edit marked an untouched flow dirty, which blocks Run (`canRun` wants a
    // clean draft) and prompts to discard changes the operator never made.
    const clean = initialEditorState(newFlowDocument("Fixture"), false);
    const panned = reduceFlowEditor(clean, {
      type: "setViewport",
      viewport: { x: 120, y: -40, zoom: 1.5 },
    });

    expect(panned).not.toBe(clean);
    expect(panned.document.viewport).toEqual({ x: 120, y: -40, zoom: 1.5 });
    expect(panned.dirty).toBe(false);
    // A view change is not an edit: no history entry, no compile invalidation.
    expect(panned.past).toEqual(clean.past);
    expect(panned.documentEpoch).toBe(clean.documentEpoch);
  });

  it("invalidates compilation on every edit and ignores an old validation result", () => {
    const initial = initialEditorState(newFlowDocument("Fixture"));
    const identity: DocumentRequestIdentity = {
      requestId: 7,
      flowId: initial.document.id,
      documentEpoch: 0,
    };
    const validating = reduceFlowEditor(initial, { type: "validationStarted", identity });
    const edited = reduceFlowEditor(validating, { type: "renameFlow", name: "Edited" });
    const stale = reduceFlowEditor(edited, {
      type: "validationCompleted",
      identity,
      issues: [],
      compiled: compiledFixture(initial.document.id, 1),
    });

    expect(stale.compiled).toBeNull();
    expect(stale.validationRequest).toBeNull();
    expect(stale.documentEpoch).toBe(1);
  });

  it("accepts save only from current compilation and preserves edits after a stale save", () => {
    const initial = initialEditorState(newFlowDocument("Fixture"));
    const identity: DocumentRequestIdentity = {
      requestId: 8,
      flowId: initial.document.id,
      documentEpoch: initial.documentEpoch,
    };
    const validating = reduceFlowEditor(initial, { type: "validationStarted", identity });
    const valid = reduceFlowEditor(validating, {
      type: "validationCompleted",
      identity,
      issues: [],
      compiled: compiledFixture(initial.document.id, 1),
    });
    expect(canStartSave(valid, identity)).toBe(true);
    const saving = reduceFlowEditor(valid, { type: "saveStarted", identity });
    const edited = reduceFlowEditor(saving, { type: "renameFlow", name: "Edited during save" });
    const completed = reduceFlowEditor(edited, {
      type: "saveCompleted",
      identity,
      record: recordFixture(initial.document),
    });

    expect(completed.document.name).toBe("Edited during save");
    expect(completed.dirty).toBe(true);
    expect(completed.saveRequest).toBeNull();
    expect(completed.notice).toEqual({
      code: "SaveCompletedForOlderDraft",
      savedRevision: 1,
    });
  });

  it("retains an in-flight save identity while switching documents", () => {
    const initial = initialEditorState(newFlowDocument("Fixture"));
    const identity: DocumentRequestIdentity = {
      requestId: 10,
      flowId: initial.document.id,
      documentEpoch: 0,
    };
    let state = reduceFlowEditor(initial, { type: "validationStarted", identity });
    state = reduceFlowEditor(state, {
      type: "validationCompleted",
      identity,
      issues: [],
      compiled: compiledFixture(initial.document.id, 1),
    });
    state = reduceFlowEditor(state, { type: "saveStarted", identity });
    state = reduceFlowEditor(state, {
      type: "replaceDocument",
      document: newFlowDocument("Another"),
      source: "new",
    });

    expect(state.saveRequest).toEqual(identity);
    const completed = reduceFlowEditor(state, {
      type: "saveCompleted",
      identity,
      record: recordFixture(initial.document),
    });
    expect(completed.document.name).toBe("Another");
    expect(completed.notice?.code).toBe("SaveCompletedForOlderDraft");
  });

  it("applies an identity-current save as a clean server revision", () => {
    const initial = initialEditorState(newFlowDocument("Fixture"));
    const identity: DocumentRequestIdentity = {
      requestId: 9,
      flowId: initial.document.id,
      documentEpoch: initial.documentEpoch,
    };
    let state = reduceFlowEditor(initial, { type: "validationStarted", identity });
    state = reduceFlowEditor(state, {
      type: "validationCompleted",
      identity,
      issues: [],
      compiled: compiledFixture(initial.document.id, 1),
    });
    state = reduceFlowEditor(state, { type: "saveStarted", identity });
    state = reduceFlowEditor(state, {
      type: "saveCompleted",
      identity,
      record: recordFixture(initial.document),
    });

    expect(state.document.revision).toBe(1);
    expect(state.dirty).toBe(false);
    expect(state.past).toEqual([]);
    expect(state.documentEpoch).toBe(1);
  });
});

describe("a save that lands for an older draft", () => {
  function editableState(name: string) {
    const document = newFlowDocument(name, sequentialIds("base"));
    document.revision = 4;
    return initialEditorState(document, false);
  }

  function startedSave(state: ReturnType<typeof editableState>) {
    const identity: DocumentRequestIdentity = {
      requestId: 1,
      flowId: state.document.id,
      documentEpoch: state.documentEpoch,
    };
    return {
      identity,
      state: reduceFlowEditor(
        {
          ...state,
          compiled: { identity, value: compiledFixture(state.document.id, state.document.revision) },
        },
        { type: "saveStarted", identity },
      ),
    };
  }

  it("rebases the newer draft onto the revision the server committed", () => {
    // Without the rebase the on-screen document kept the pre-save number, so every later save sent
    // a stale expected revision and the backend answered RevisionConflict forever -- and the
    // autosaved draft recorded the same stale number as `baseRevision`, so on restart the draft no
    // longer matched the server revision and the edit made during the save was discarded silently.
    let { state, identity } = startedSave(editableState("Rebase"));
    const record = recordFixture(state.document);
    expect(record.document.revision).toBe(5);

    state = reduceFlowEditor(state, { type: "renameFlow", name: "edited while saving" });
    state = reduceFlowEditor(state, { type: "saveCompleted", identity, record });

    expect(state.notice).toEqual({ code: "SaveCompletedForOlderDraft", savedRevision: 5 });
    expect(state.document.name).toBe("edited while saving");
    expect(state.document.revision).toBe(5);
    expect(state.dirty).toBe(true);
    expect(state.saveRequest).toBeNull();
    // History carries the same number, so undoing does not walk back into the stale one.
    expect(state.past.map((document) => document.revision)).toEqual([5]);
  });

  it("lets undo land back on what the server now holds", () => {
    let { state, identity } = startedSave(editableState("Undo to clean"));
    const record = recordFixture(state.document);
    state = reduceFlowEditor(state, { type: "renameFlow", name: "edited while saving" });
    state = reduceFlowEditor(state, { type: "saveCompleted", identity, record });
    state = reduceFlowEditor(state, { type: "undo" });

    expect(state.document.name).toBe("Undo to clean");
    expect(state.document.revision).toBe(5);
    expect(state.dirty).toBe(false);
  });
});

describe("undo reaches the clean savepoint", () => {
  it("stops claiming unsaved work once the document is the saved one again", () => {
    // `undo` used to set `dirty: true` unconditionally, so undoing the only edit left Run disabled
    // (it requires a clean draft), Save enabled, and autosave writing a draft byte-identical to
    // the server copy.
    const document = newFlowDocument("Clean", sequentialIds("base"));
    let state = initialEditorState(document, false);
    expect(state.dirty).toBe(false);

    state = reduceFlowEditor(state, { type: "renameFlow", name: "renamed" });
    expect(state.dirty).toBe(true);

    state = reduceFlowEditor(state, { type: "undo" });
    expect(state.document.name).toBe("Clean");
    expect(state.past).toHaveLength(0);
    expect(state.dirty).toBe(false);

    state = reduceFlowEditor(state, { type: "redo" });
    expect(state.dirty).toBe(true);
  });

  it("keeps a brand-new flow dirty, because it has no saved state to return to", () => {
    const state = initialEditorState(newFlowDocument("New", sequentialIds("base")));
    expect(state.document.revision).toBe(0);
    expect(state.dirty).toBe(true);
    expect(reduceFlowEditor(state, { type: "undo" }).dirty).toBe(true);
  });

  it("does not treat panning as an edit that undo has to reverse", () => {
    const document = newFlowDocument("Viewport", sequentialIds("base"));
    let state = initialEditorState(document, false);
    state = reduceFlowEditor(state, { type: "setViewport", viewport: { x: 9, y: 9, zoom: 2 } });
    expect(state.dirty).toBe(false);

    state = reduceFlowEditor(state, { type: "renameFlow", name: "renamed" });
    state = reduceFlowEditor(state, { type: "undo" });
    expect(state.dirty).toBe(false);
    expect(state.document.viewport).toEqual({ x: 9, y: 9, zoom: 2 });
  });
});

describe("one delete gesture, one mutation", () => {
  it("bridges the path even though the incident edges arrive first", () => {
    // React Flow's `deleteElements` fires `onEdgesChange` for the incident edges before it calls
    // `onNodesDelete`. Committing those separately meant the node delete ran on a document that
    // had already lost the edges it needed to reconnect: one keypress left Start and End with no
    // path at all. `deleteSelection` takes the whole gesture and applies it to the intact document.
    const document = newFlowDocument("Bridge", sequentialIds("base"));
    const wait = createFlowNode("wait", { x: 160, y: 80 }, sequentialIds("node"));
    let state = initialEditorState(document, false);
    state = reduceFlowEditor(state, {
      type: "insertNode",
      edgeId: document.edges[0].id,
      node: wait,
      sourcePort: "flow",
      idFactory: sequentialIds("split"),
    });
    const epochBeforeDelete = state.documentEpoch;

    state = reduceFlowEditor(state, {
      type: "deleteSelection",
      nodeIds: [wait.id],
      edgeIds: [],
      idFactory: sequentialIds("join"),
    });

    expect(state.document.nodes.map((node) => node.kind)).toEqual(["start", "end"]);
    expect(state.document.edges).toHaveLength(1);
    expect(state.document.edges[0]).toMatchObject({
      sourceNodeId: document.nodes[0].id,
      targetNodeId: document.nodes[1].id,
    });
    // One history entry, so one Undo restores the node *and* its wiring.
    expect(state.documentEpoch).toBe(epochBeforeDelete + 1);
    const undone = reduceFlowEditor(state, { type: "undo" });
    expect(undone.document.nodes).toHaveLength(3);
    expect(undone.document.edges).toHaveLength(2);
  });

  it("still removes edges selected in their own right", () => {
    const document = newFlowDocument("Edges", sequentialIds("base"));
    const state = reduceFlowEditor(initialEditorState(document, false), {
      type: "deleteSelection",
      nodeIds: [],
      edgeIds: [document.edges[0].id],
    });
    expect(state.document.edges).toHaveLength(0);
    expect(state.document.nodes).toHaveLength(2);
  });

  it("refuses a structural node without taking its edge with it", () => {
    const document = newFlowDocument("Structural", sequentialIds("base"));
    const state = reduceFlowEditor(initialEditorState(document, false), {
      type: "deleteSelection",
      nodeIds: [document.nodes[0].id],
      edgeIds: [],
    });
    expect(state.document.nodes).toHaveLength(2);
    expect(state.document.edges).toHaveLength(1);
    expect(state.dirty).toBe(false);
  });

  it("reconnects across several nodes deleted in one gesture", () => {
    const document = newFlowDocument("Multi", sequentialIds("base"));
    const first = createFlowNode("wait", { x: 100, y: 0 }, sequentialIds("first"));
    const second = createFlowNode("home", { x: 200, y: 0 }, sequentialIds("second"));
    let state = initialEditorState(document, false);
    state = reduceFlowEditor(state, {
      type: "insertNode",
      edgeId: document.edges[0].id,
      node: first,
      sourcePort: "flow",
      idFactory: sequentialIds("split-a"),
    });
    state = reduceFlowEditor(state, {
      type: "insertNode",
      edgeId: state.document.edges[1].id,
      node: second,
      sourcePort: "flow",
      idFactory: sequentialIds("split-b"),
    });
    expect(state.document.nodes).toHaveLength(4);

    state = reduceFlowEditor(state, {
      type: "deleteSelection",
      nodeIds: [first.id, second.id],
      edgeIds: [],
      idFactory: sequentialIds("join"),
    });
    expect(state.document.nodes.map((node) => node.kind)).toEqual(["start", "end"]);
    expect(state.document.edges).toHaveLength(1);
    expect(state.document.edges[0]).toMatchObject({
      sourceNodeId: document.nodes[0].id,
      targetNodeId: document.nodes[1].id,
    });
  });
});

describe("inserting a node on an edge uses the node's real output port", () => {
  it("wires the branch port the caller names", () => {
    // The outgoing edge used to be hard-coded to `sourcePort: "flow"`, which `ifVision` does not
    // have, so dropping an If Vision onto an edge always drew a graph the compiler rejected.
    const document = newFlowDocument("Branch", sequentialIds("base"));
    const branch = createFlowNode("ifVision", { x: 160, y: 80 }, sequentialIds("node"));
    const state = reduceFlowEditor(initialEditorState(document, false), {
      type: "insertNode",
      edgeId: document.edges[0].id,
      node: branch,
      sourcePort: "matched",
      idFactory: sequentialIds("split"),
    });
    const outgoing = state.document.edges.find((edge) => edge.sourceNodeId === branch.id);
    expect(outgoing?.sourcePort).toBe("matched");
  });
});
