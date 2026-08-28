import type {
  CompiledRevision,
  EvidenceSpec,
  FlowDocumentV2,
  FlowNode,
  FlowRevisionRecord,
  FlowValidationIssue,
  FlowViewport,
  JsonObject,
} from "../types";
import type { FlowCanvasEdge, FlowCanvasNode } from "./graph";
import {
  appendUnconnectedNode,
  deleteExecutableNode,
  insertNodeOnEdge,
  reconnectEdge,
  withCanvasLayout,
} from "./graph";
import {
  cloneFlowDocument,
  newFlowDocument,
  withNodeConfig,
  withNodePostcondition,
  type IdFactory,
} from "./model";
import { isFiniteJsonObject, isFiniteJsonValue } from "./validation";

const HISTORY_LIMIT = 50;

export interface DocumentRequestIdentity {
  requestId: number;
  flowId: string;
  documentEpoch: number;
}

export interface ValidatedCompilation {
  identity: DocumentRequestIdentity;
  value: CompiledRevision;
}

export type FlowEditorNotice = {
  code: "SaveCompletedForOlderDraft";
  savedRevision: number;
} | null;

export interface FlowEditorState {
  document: FlowDocumentV2;
  past: FlowDocumentV2[];
  future: FlowDocumentV2[];
  selectedNodeId: string | null;
  dirty: boolean;
  documentEpoch: number;
  validation: FlowValidationIssue[];
  validationRequest: DocumentRequestIdentity | null;
  compiled: ValidatedCompilation | null;
  saveRequest: DocumentRequestIdentity | null;
  notice: FlowEditorNotice;
  /**
   * Fingerprint of the last *clean* document, or null when there is no clean point to return to.
   *
   * `dirty` used to be set to `true` by every history step unconditionally, so undoing back to the
   * saved document left the editor claiming unsaved work: Run stayed disabled (it requires a clean
   * draft), Save stayed enabled, and autosave kept writing a draft identical to the server copy.
   * With a savepoint the reducer can answer the real question -- is what is on screen the same
   * authored content as what was saved -- instead of counting gestures.
   */
  savepoint: string | null;
}

export type FlowEditorAction =
  | { type: "selectNode"; nodeId: string | null }
  | { type: "renameFlow"; name: string }
  | { type: "setViewport"; viewport: FlowViewport }
  | { type: "moveNode"; nodeId: string; position: { x: number; y: number } }
  | { type: "replaceCanvas"; nodes: FlowCanvasNode[]; edges: FlowCanvasEdge[] }
  | {
      type: "insertNode";
      edgeId: string;
      node: FlowNode;
      sourcePort: string;
      idFactory?: IdFactory;
    }
  | { type: "appendNode"; node: FlowNode }
  | {
      type: "reconnectEdge";
      edgeId: string;
      sourceNodeId: string;
      targetNodeId: string;
    }
  | { type: "deleteSelection"; nodeIds: string[]; edgeIds: string[]; idFactory?: IdFactory }
  | {
      type: "updateNodeConfig";
      nodeId: string;
      config: JsonObject;
      /**
       * Evidence to write in the *same* mutation. `undefined` leaves the postcondition alone.
       *
       * The inspector keeps some evidence in step with the config it mirrors -- a Launch App bundle
       * and its `activeAppEquals` bundle, for instance. Committing those as two reducer actions gave
       * one field edit two history entries, and the entry in between was a state the operator never
       * saw: config B with evidence A, which the compiler then rejects as `EvidenceMismatch`. One
       * Undo landed on it; undoing one edit took two.
       */
      postcondition?: EvidenceSpec | null;
    }
  | { type: "updateNodePostcondition"; nodeId: string; postcondition: EvidenceSpec | null }
  | {
      type: "replaceDocument";
      document: FlowDocumentV2;
      source: "server" | "draft" | "import" | "new" | "duplicate";
    }
  | { type: "undo" }
  | { type: "redo" }
  | { type: "validationStarted"; identity: DocumentRequestIdentity }
  | {
      type: "validationCompleted";
      identity: DocumentRequestIdentity;
      issues: FlowValidationIssue[];
      compiled: CompiledRevision | null;
    }
  | { type: "saveStarted"; identity: DocumentRequestIdentity }
  | { type: "saveCompleted"; identity: DocumentRequestIdentity; record: FlowRevisionRecord }
  | { type: "saveFailed"; identity: DocumentRequestIdentity }
  | { type: "dismissNotice" };

export function initialEditorState(
  document: FlowDocumentV2 = newFlowDocument(),
  dirty = document.revision === 0,
): FlowEditorState {
  return {
    document: cloneFlowDocument(document),
    past: [],
    future: [],
    selectedNodeId: null,
    dirty,
    documentEpoch: 0,
    validation: [],
    validationRequest: null,
    compiled: null,
    saveRequest: null,
    notice: null,
    savepoint: dirty ? null : documentFingerprint(document),
  };
}

export function reduceFlowEditor(
  state: FlowEditorState,
  action: FlowEditorAction,
): FlowEditorState {
  switch (action.type) {
    case "selectNode":
      return action.nodeId === null || state.document.nodes.some((node) => node.id === action.nodeId)
        ? { ...state, selectedNodeId: action.nodeId }
        : state;
    case "renameFlow":
      return action.name === state.document.name
        ? state
        : mutate(state, { ...state.document, name: action.name });
    case "setViewport":
      // Panning and zooming is a view preference, not an edit. Routing it
      // through mutate() marked a freshly opened flow dirty (React Flow emits
      // a move on mount), which blocked Run — `canRun` requires a clean draft —
      // prompted to discard changes nobody made, and threw away the compiled
      // plan. The new viewport rides along with the next real edit's save.
      return finiteViewport(action.viewport) &&
          !sameViewport(action.viewport, state.document.viewport)
        ? { ...state, document: { ...state.document, viewport: { ...action.viewport } } }
        : state;
    case "moveNode": {
      if (!Number.isFinite(action.position.x) || !Number.isFinite(action.position.y)) return state;
      let found = false;
      const nodes = state.document.nodes.map((node) => {
        if (node.id !== action.nodeId) return node;
        found = true;
        return { ...node, position: { ...action.position } };
      });
      return found ? mutate(state, { ...state.document, nodes }) : state;
    }
    case "replaceCanvas": {
      if (action.nodes.some((node) => !finitePosition(node.position))) return state;
      return mutate(state, withCanvasLayout(state.document, action.nodes, action.edges));
    }
    case "insertNode": {
      if (!finiteNode(action.node)) return state;
      return mutate(
        state,
        insertNodeOnEdge(
          state.document,
          action.edgeId,
          action.node,
          action.sourcePort,
          action.idFactory,
        ),
      );
    }
    case "appendNode":
      return finiteNode(action.node)
        ? mutate(state, appendUnconnectedNode(state.document, action.node))
        : state;
    case "reconnectEdge":
      return mutate(
        state,
        reconnectEdge(
          state.document,
          action.edgeId,
          action.sourceNodeId,
          action.targetNodeId,
        ),
      );
    case "deleteSelection": {
      // One gesture, one mutation, applied to the document as it stands *before* anything is
      // removed. React Flow's `deleteElements` fires `onEdgesChange` with the incident edges
      // before it calls `onNodesDelete`, so committing those two callbacks separately meant
      // `deleteExecutableNode` ran on a document whose incident edges were already gone: it saw
      // 0 in / 0 out, skipped the reconnect, and one Delete keypress silently left `Start` and
      // `End` with no path between them. It also cost two history entries, so the first Undo
      // restored a node with none of its wiring.
      let document = state.document;
      if (action.edgeIds.length > 0) {
        const removing = new Set(action.edgeIds);
        const edges = document.edges.filter((edge) => !removing.has(edge.id));
        if (edges.length !== document.edges.length) document = { ...document, edges };
      }
      for (const nodeId of action.nodeIds) {
        document = deleteExecutableNode(document, nodeId, action.idFactory);
      }
      return mutate(state, document);
    }
    case "updateNodeConfig": {
      if (!isFiniteJsonObject(action.config)) return state;
      if (
        action.postcondition !== undefined &&
        action.postcondition !== null &&
        !isFiniteJsonValue(action.postcondition)
      ) {
        return state;
      }
      let document = withNodeConfig(state.document, action.nodeId, action.config);
      if (action.postcondition !== undefined) {
        document = withNodePostcondition(document, action.nodeId, action.postcondition);
      }
      return mutate(state, document);
    }
    case "updateNodePostcondition":
      return action.postcondition === null || isFiniteJsonValue(action.postcondition)
        ? mutate(
            state,
            withNodePostcondition(state.document, action.nodeId, action.postcondition),
          )
        : state;
    case "replaceDocument":
      return replaceDocument(state, action.document, action.source === "server" ? false : true);
    case "undo":
      return undo(state);
    case "redo":
      return redo(state);
    case "validationStarted":
      return identityMatchesState(action.identity, state) && state.validationRequest === null
        ? { ...state, validationRequest: { ...action.identity }, compiled: null }
        : state;
    case "validationCompleted":
      if (
        !identityMatchesState(action.identity, state) ||
        !sameIdentity(action.identity, state.validationRequest)
      ) {
        return state;
      }
      return {
        ...state,
        validation: action.issues.map((issue) => ({ ...issue })),
        validationRequest: null,
        compiled:
          action.compiled === null
            ? null
            : { identity: { ...action.identity }, value: structuredClone(action.compiled) },
      };
    case "saveStarted":
      return canStartSave(state, action.identity)
        ? { ...state, saveRequest: { ...action.identity } }
        : state;
    case "saveCompleted":
      return completeSave(state, action.identity, action.record);
    case "saveFailed":
      return sameIdentity(action.identity, state.saveRequest)
        ? { ...state, saveRequest: null }
        : state;
    case "dismissNotice":
      return state.notice === null ? state : { ...state, notice: null };
  }
}

export function isCompilationCurrent(state: FlowEditorState): boolean {
  return state.compiled !== null && identityMatchesState(state.compiled.identity, state);
}

export function canStartSave(
  state: FlowEditorState,
  identity: DocumentRequestIdentity,
): boolean {
  return (
    state.saveRequest === null &&
    state.validationRequest === null &&
    state.validation.length === 0 &&
    state.compiled !== null &&
    sameIdentity(identity, state.compiled.identity) &&
    identityMatchesState(identity, state)
  );
}

function mutate(state: FlowEditorState, document: FlowDocumentV2): FlowEditorState {
  if (document === state.document) return state;
  const past = [...state.past, cloneFlowDocument(state.document)].slice(-HISTORY_LIMIT);
  return invalidatedState(state, document, {
    past,
    future: [],
    dirty: dirtyAgainstSavepoint(state, document),
    selectedNodeId: document.nodes.some((node) => node.id === state.selectedNodeId)
      ? state.selectedNodeId
      : null,
  });
}

function replaceDocument(
  state: FlowEditorState,
  document: FlowDocumentV2,
  dirty: boolean,
): FlowEditorState {
  return invalidatedState(state, document, {
    past: [],
    future: [],
    dirty,
    selectedNodeId: null,
    notice: null,
    // A server copy is the clean point; an import, a new flow, or a duplicate is unsaved work with
    // nothing behind it to return to.
    savepoint: dirty ? null : documentFingerprint(document),
  });
}

function undo(state: FlowEditorState): FlowEditorState {
  const document = state.past.at(-1);
  if (document === undefined) return state;
  return invalidatedState(state, document, {
    past: state.past.slice(0, -1),
    future: [cloneFlowDocument(state.document), ...state.future].slice(0, HISTORY_LIMIT),
    dirty: dirtyAgainstSavepoint(state, document),
    selectedNodeId: document.nodes.some((node) => node.id === state.selectedNodeId)
      ? state.selectedNodeId
      : null,
  });
}

function redo(state: FlowEditorState): FlowEditorState {
  const document = state.future[0];
  if (document === undefined) return state;
  return invalidatedState(state, document, {
    past: [...state.past, cloneFlowDocument(state.document)].slice(-HISTORY_LIMIT),
    future: state.future.slice(1),
    dirty: dirtyAgainstSavepoint(state, document),
    selectedNodeId: document.nodes.some((node) => node.id === state.selectedNodeId)
      ? state.selectedNodeId
      : null,
  });
}

function completeSave(
  state: FlowEditorState,
  identity: DocumentRequestIdentity,
  record: FlowRevisionRecord,
): FlowEditorState {
  if (!sameIdentity(identity, state.saveRequest)) return state;
  if (!identityMatchesState(identity, state)) {
    // The operator edited while the save was in flight. Keeping their newer edits is right -- that
    // is what this branch is for -- but the *revision* those edits carry is now stale: the server
    // committed the snapshot as `record.document.revision`, and what is on screen is a descendant
    // of it.
    //
    // Left un-rebased, the document kept the pre-save number, so `save` sent that number as the
    // expected revision and the backend answered `RevisionConflict` -- on every retry, forever,
    // because event invalidation deliberately skips dirty documents and so can never rebase it.
    // And autosave wrote the stale number as `baseRevision`, so on restart the draft no longer
    // matched the server revision, restoration refused it, and the edit made during the save was
    // gone with no message. History is rebased along with it, so undoing back through those edits
    // does not walk into the same stale number.
    const revision = record.document.revision;
    return {
      ...state,
      ...rebasedHistory(state, revision),
      document: { ...state.document, revision },
      saveRequest: null,
      // What the server now holds is the clean point, so undoing the post-save edits lands on it.
      savepoint: documentFingerprint(record.document),
      notice: {
        code: "SaveCompletedForOlderDraft",
        savedRevision: revision,
      },
    };
  }
  return invalidatedState(state, record.document, {
    past: [],
    future: [],
    dirty: false,
    savepoint: documentFingerprint(record.document),
    selectedNodeId: null,
    saveRequest: null,
    notice: null,
  });
}

function invalidatedState(
  state: FlowEditorState,
  document: FlowDocumentV2,
  changes: Partial<FlowEditorState>,
): FlowEditorState {
  return {
    ...state,
    ...changes,
    document: cloneFlowDocument(document),
    documentEpoch: state.documentEpoch + 1,
    validation: [],
    validationRequest: null,
    compiled: null,
  };
}

/**
 * Authored content of a document, as a comparable string.
 *
 * Deliberately excludes `viewport` and `revision`. The viewport is a view preference --
 * `setViewport` already refuses to mark the document dirty, and including it here would undo that
 * decision. `revision` is a server label, not authoring: history snapshots taken before a rebase
 * carry the older number, and comparing it would report a document as changed when only its label
 * moved.
 */
function documentFingerprint(document: FlowDocumentV2): string {
  return JSON.stringify({
    name: document.name,
    entryNodeId: document.entryNodeId,
    nodes: document.nodes.map((node) => ({
      id: node.id,
      kind: node.kind,
      position: node.position,
      config: node.config,
      postcondition: node.postcondition ?? null,
    })),
    edges: document.edges.map((edge) => ({
      id: edge.id,
      sourceNodeId: edge.sourceNodeId,
      sourcePort: edge.sourcePort,
      targetNodeId: edge.targetNodeId,
      targetPort: edge.targetPort,
    })),
  });
}

/** True when `document` differs from the savepoint, or when there is no savepoint to compare to. */
function dirtyAgainstSavepoint(state: FlowEditorState, document: FlowDocumentV2): boolean {
  return state.savepoint === null || documentFingerprint(document) !== state.savepoint;
}

/** History snapshots with `revision` moved to a newer one the server has committed. */
function rebasedHistory(state: FlowEditorState, revision: number): Partial<FlowEditorState> {
  const rebase = (document: FlowDocumentV2): FlowDocumentV2 => ({ ...document, revision });
  return { past: state.past.map(rebase), future: state.future.map(rebase) };
}

function identityMatchesState(
  identity: DocumentRequestIdentity,
  state: FlowEditorState,
): boolean {
  return identity.flowId === state.document.id && identity.documentEpoch === state.documentEpoch;
}

function sameIdentity(
  left: DocumentRequestIdentity,
  right: DocumentRequestIdentity | null,
): boolean {
  return (
    right !== null &&
    left.requestId === right.requestId &&
    left.flowId === right.flowId &&
    left.documentEpoch === right.documentEpoch
  );
}

function finiteViewport(viewport: FlowViewport): boolean {
  return Number.isFinite(viewport.x) && Number.isFinite(viewport.y) &&
    Number.isFinite(viewport.zoom);
}

function sameViewport(left: FlowViewport, right: FlowViewport): boolean {
  return left.x === right.x && left.y === right.y && left.zoom === right.zoom;
}

function finitePosition(position: { x: number; y: number }): boolean {
  return Number.isFinite(position.x) && Number.isFinite(position.y);
}

function finiteNode(node: FlowNode): boolean {
  return finitePosition(node.position) && isFiniteJsonObject(node.config) &&
    (node.postcondition === undefined || node.postcondition === null ||
      isFiniteJsonValue(node.postcondition));
}
