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
