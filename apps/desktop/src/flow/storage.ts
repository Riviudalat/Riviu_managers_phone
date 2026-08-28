import type { FlowDocumentV2 } from "../types";
import { cloneFlowDocument } from "./model";
import { isFlowDocumentV2 } from "./validation";

const DRAFT_SCHEMA_VERSION = 1;
const DRAFT_KEY_PREFIX = "riviu.flowDraft.";

export interface StoredFlowDraft {
  schemaVersion: 1;
  flowId: string;
  baseRevision: number;
  document: FlowDocumentV2;
  savedAt: string;
}

export function flowDraftKey(flowId: string): string {
  return `${DRAFT_KEY_PREFIX}${flowId}`;
}

export function saveDraft(
  document: FlowDocumentV2,
  storage: Storage = localStorage,
  now: () => Date = () => new Date(),
): StoredFlowDraft {
  if (!isFlowDocumentV2(document)) {
    throw new TypeError("Flow draft is not a finite Flow V2 document");
  }
  const stored: StoredFlowDraft = {
    schemaVersion: DRAFT_SCHEMA_VERSION,
    flowId: document.id,
    baseRevision: document.revision,
    document: storageDocument(document),
    savedAt: now().toISOString(),
  };
  storage.setItem(flowDraftKey(document.id), JSON.stringify(stored));
  return stored;
}

export function loadDraft(
  flowId: string,
  storage: Storage = localStorage,
): StoredFlowDraft | null {
  const encoded = storage.getItem(flowDraftKey(flowId));
  if (encoded === null) return null;
  try {
    const value: unknown = JSON.parse(encoded);
    if (!isStoredFlowDraft(value, flowId)) return null;
    return { ...value, document: storageDocument(value.document) };
  } catch {
    return null;
  }
}

export function clearDraft(flowId: string, storage: Storage = localStorage): void {
  storage.removeItem(flowDraftKey(flowId));
}

export class FlowDraftWriter {
  private timer: ReturnType<typeof setTimeout> | null = null;
  private pending: FlowDocumentV2 | null = null;
  private readonly storage: Storage;
  private readonly delayMs: number;
  private readonly onError: (reason: unknown) => void;

  /**
   * `onError` exists because the debounced write happens on a timer, long after `schedule` returned
   * `void`. A quota error or a storage exception therefore escaped into nothing: the graph stayed
   * dirty on screen, nobody was told the recovery draft had not been written, and it was gone after
   * a shutdown. The default keeps that path from being silent even when a caller does not pass one.
   */
  constructor(
    storage: Storage = localStorage,
    delayMs = 300,
    onError: (reason: unknown) => void = (reason) => {
      console.error("Flow draft autosave failed", reason);
    },
  ) {
    this.storage = storage;
    this.delayMs = delayMs;
    this.onError = onError;
  }

  schedule(document: FlowDocumentV2): void {
    if (!isFlowDocumentV2(document)) {
      throw new TypeError("Flow draft is not a finite Flow V2 document");
    }
    this.cancelTimer();
    this.pending = cloneFlowDocument(document);
    this.timer = setTimeout(() => this.flush(), this.delayMs);
  }

  flush(): void {
    this.cancelTimer();
    if (this.pending === null) return;
    const document = this.pending;
    // Cleared before the write, not after: a write that throws every time would otherwise be
    // retried by the next flush with the same document and report the same failure forever.
    this.pending = null;
    try {
      saveDraft(document, this.storage);
    } catch (reason) {
      this.onError(reason);
    }
  }

  cancel(): void {
    this.cancelTimer();
    this.pending = null;
  }

  private cancelTimer(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }
}

function isStoredFlowDraft(value: unknown, flowId: string): value is StoredFlowDraft {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const candidate = value as Record<string, unknown>;
  return (
    Object.keys(candidate).sort().join(",") ===
      "baseRevision,document,flowId,savedAt,schemaVersion" &&
    candidate.schemaVersion === DRAFT_SCHEMA_VERSION &&
    candidate.flowId === flowId &&
    typeof candidate.savedAt === "string" &&
    typeof candidate.baseRevision === "number" &&
    Number.isSafeInteger(candidate.baseRevision) &&
    candidate.baseRevision >= 0 &&
    isFlowDocumentV2(candidate.document) &&
    candidate.document.id === flowId &&
    candidate.document.revision === candidate.baseRevision
  );
}

function storageDocument(document: FlowDocumentV2): FlowDocumentV2 {
  return {
    schemaVersion: 2,
    id: document.id,
    name: document.name,
    revision: document.revision,
    entryNodeId: document.entryNodeId,
    nodes: document.nodes.map((node) => ({
      id: node.id,
      kind: node.kind,
      position: { x: node.position.x, y: node.position.y },
      config: structuredClone(node.config),
      ...(node.postcondition === undefined
        ? {}
        : {
            postcondition:
              node.postcondition === null ? null : structuredClone(node.postcondition),
          }),
    })),
    edges: document.edges.map((edge) => ({
      id: edge.id,
      sourceNodeId: edge.sourceNodeId,
      sourcePort: edge.sourcePort,
      targetNodeId: edge.targetNodeId,
      targetPort: edge.targetPort,
    })),
    viewport: {
      x: document.viewport.x,
      y: document.viewport.y,
      zoom: document.viewport.zoom,
    },
  };
}
