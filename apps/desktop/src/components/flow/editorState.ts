import type {
  FlowDocumentV2,
  JsonObject,
} from "../../types";
import {
  type FlowCanvasEdge,
  type FlowCanvasNode,
  withCanvasLayout,
} from "../../flow/graph";

export {
  canStartSave,
  initialEditorState,
  isCompilationCurrent,
  reduceFlowEditor,
  reduceFlowEditor as reduce,
} from "../../flow/draft";
export type {
  DocumentRequestIdentity,
  FlowEditorAction,
  FlowEditorNotice,
  FlowEditorState,
  ValidatedCompilation,
} from "../../flow/draft";
export {
  cloneFlowDocument,
  createFlowId,
  createFlowNode,
  defaultConfigForAction,
  duplicateDocument,
  newFlowDocument,
  withNodeConfig,
  withNodePostcondition,
} from "../../flow/model";
export type { IdFactory } from "../../flow/model";
export {
  appendUnconnectedNode,
  deleteExecutableNode,
  insertNodeOnEdge,
  reconnectEdge,
  toCanvas,
  withCanvasLayout,
} from "../../flow/graph";
export type {
  FlowCanvasEdge,
  FlowCanvasGraph,
  FlowCanvasNode,
  FlowNodeData,
} from "../../flow/graph";

/**
 * Converts the controlled React Flow graph back to the persisted domain model.
 * React-only fields such as selection, dimensions, and drag state are ignored.
 */
export function toDocument(
  document: FlowDocumentV2,
  nodes: FlowCanvasNode[],
  edges: FlowCanvasEdge[],
): FlowDocumentV2 {
  const withLayout = withCanvasLayout(document, nodes, edges);
  const dataById = new Map(nodes.map((node) => [node.id, node.data]));

  return {
    ...withLayout,
    nodes: withLayout.nodes.map((node) => {
      const data = dataById.get(node.id);
      if (data === undefined) return node;
      return {
        ...node,
        config: structuredClone(data.config) as JsonObject,
        postcondition:
          data.postcondition === null ? null : structuredClone(data.postcondition),
      };
    }),
  };
}
