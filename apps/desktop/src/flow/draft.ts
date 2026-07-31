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
}

export type FlowEditorAction =
  | { type: "selectNode"; nodeId: string | null }
  | { type: "renameFlow"; name: string }
  | { type: "setViewport"; viewport: FlowViewport }
  | { type: "moveNode"; nodeId: string; position: { x: number; y: number } }
  | { type: "replaceCanvas"; nodes: FlowCanvasNode[]; edges: FlowCanvasEdge[] }
  | { type: "insertNode"; edgeId: string; node: FlowNode; idFactory?: IdFactory }
  | { type: "appendNode"; node: FlowNode }
  | {
      type: "reconnectEdge";
      edgeId: string;
      sourceNodeId: string;
      targetNodeId: string;
    }
  | { type: "deleteNode"; nodeId: string; idFactory?: IdFactory }
  | { type: "updateNodeConfig"; nodeId: string; config: JsonObject }
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
      return finiteViewport(action.viewport) &&
          !sameViewport(action.viewport, state.document.viewport)
        ? mutate(state, { ...state.document, viewport: { ...action.viewport } })
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
        insertNodeOnEdge(state.document, action.edgeId, action.node, action.idFactory),
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
    case "deleteNode":
      return mutate(state, deleteExecutableNode(state.document, action.nodeId, action.idFactory));
    case "updateNodeConfig":
      return isFiniteJsonObject(action.config)
        ? mutate(state, withNodeConfig(state.document, action.nodeId, action.config))
        : state;
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
    dirty: true,
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
  });
}

function undo(state: FlowEditorState): FlowEditorState {
  const document = state.past.at(-1);
  if (document === undefined) return state;
  return invalidatedState(state, document, {
    past: state.past.slice(0, -1),
    future: [cloneFlowDocument(state.document), ...state.future].slice(0, HISTORY_LIMIT),
    dirty: true,
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
    dirty: true,
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
    return {
      ...state,
      saveRequest: null,
      notice: {
        code: "SaveCompletedForOlderDraft",
        savedRevision: record.document.revision,
      },
    };
  }
  return invalidatedState(state, record.document, {
    past: [],
    future: [],
    dirty: false,
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
