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

  constructor(
    storage: Storage = localStorage,
    delayMs = 300,
  ) {
    this.storage = storage;
    this.delayMs = delayMs;
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
    saveDraft(this.pending, this.storage);
    this.pending = null;
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
