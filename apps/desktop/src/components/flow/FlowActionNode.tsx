import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";
import type {
  ActionKind,
  FlowValidationIssue,
  JsonObject,
} from "../../types";
import { ACTION_PRESENTATION, summarizeAction } from "./actionPresentation";

export interface FlowActionNodeData extends Record<string, unknown> {
  kind: ActionKind;
  config: JsonObject;
  issues: FlowValidationIssue[];
}

export type FlowCanvasNode = Node<FlowActionNodeData, "flowAction">;

export function FlowActionNode({ data, selected }: NodeProps<FlowCanvasNode>) {
  const presentation = ACTION_PRESENTATION[data.kind];
  if (!presentation) return null;
  const Icon = presentation.icon;
  const firstIssue = data.issues[0];

  return (
    <div className="flow-node" data-testid="flow-node" data-selected={selected || undefined}>
      {data.kind !== "start" && (
        <Handle type="target" position={Position.Left} id="flow" />
      )}
      <div className="flow-node-heading">
        <Icon aria-hidden="true" size={16} />
        <span className="flow-node-title" data-testid="flow-node-title">
          {presentation.label}
        </span>
        {data.issues.length > 0 && (
          <span className="flow-node-error" title={firstIssue?.message}>
            {data.issues.length}
          </span>
        )}
      </div>
      <div className="flow-node-summary">{summarizeAction(data.kind, data.config)}</div>
      {data.kind === "ifVision" ? (
        <>
          <span className="flow-node-port-label flow-node-port-matched">matched</span>
          <Handle
            type="source"
            position={Position.Right}
            id="matched"
            style={{ top: "38%" }}
          />
          <span className="flow-node-port-label flow-node-port-unmatched">no match</span>
          <Handle
            type="source"
            position={Position.Right}
            id="notMatched"
            style={{ top: "72%" }}
          />
        </>
      ) : (
        data.kind !== "end" && (
          <Handle type="source" position={Position.Right} id="flow" />
        )
      )}
    </div>
  );
}
