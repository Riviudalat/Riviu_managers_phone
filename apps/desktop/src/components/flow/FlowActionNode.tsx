import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";
import {
  Camera,
  CirclePlay,
  CircleStop,
  Crosshair,
  GitBranch,
  House,
  Keyboard,
  MousePointerClick,
  MoveUp,
  PowerOff,
  Rocket,
  ScanSearch,
  Timer,
  type LucideIcon,
} from "lucide-react";
import type {
  ActionKind,
  FlowValidationIssue,
  JsonObject,
  JsonValue,
} from "../../types";

export interface FlowActionNodeData extends Record<string, unknown> {
  kind: ActionKind;
  config: JsonObject;
  issues: FlowValidationIssue[];
}

export type FlowCanvasNode = Node<FlowActionNodeData, "flowAction">;

export const ACTION_PRESENTATION: Partial<
  Record<ActionKind, { label: string; icon: LucideIcon }>
> = {
  start: { label: "Start", icon: CirclePlay },
  end: { label: "End", icon: CircleStop },
  launchApp: { label: "Launch App", icon: Rocket },
  terminateApp: { label: "Terminate App", icon: PowerOff },
  wait: { label: "Wait", icon: Timer },
  tap: { label: "Tap", icon: MousePointerClick },
  swipe: { label: "Swipe", icon: MoveUp },
  typeText: { label: "Type Text", icon: Keyboard },
  screenshot: { label: "Screenshot", icon: Camera },
  home: { label: "Home", icon: House },
  assertVisible: { label: "Assert Visible", icon: ScanSearch },
  tapVision: { label: "Tap Vision", icon: Crosshair },
  ifVision: { label: "If Vision", icon: GitBranch },
};

function objectNumber(value: JsonValue | undefined, key: string): number | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  const field = value[key];
  return typeof field === "number" && Number.isFinite(field) ? field : null;
}

export function summarizeAction(kind: ActionKind, config: JsonObject): string {
  const text = (key: string) => {
    const value = config[key];
    return typeof value === "string" ? value : "";
  };
  switch (kind) {
    case "launchApp":
    case "terminateApp":
      return text("bundleId");
    case "wait":
      return typeof config.durationMs === "number" ? `${config.durationMs} ms` : "";
    case "tap": {
      const coordinates = [objectNumber(config.point, "x"), objectNumber(config.point, "y")]
        .filter((value): value is number => value !== null)
        .join(", ");
      return text("accessibilityId") || coordinates;
    }
    case "swipe":
      return `Swipe${
        typeof config.durationMs === "number" ? ` ${config.durationMs} ms` : ""
      }`;
    case "typeText":
      return `${text("text").length} characters`;
    case "screenshot":
      return text("label");
    case "assertVisible":
      return text("accessibilityId");
    case "tapVision":
    case "ifVision": {
      const hasTemplate = text("templatePngBase64").length > 0;
      const threshold =
        typeof config.threshold === "number" ? config.threshold.toFixed(2) : "?";
      return hasTemplate ? `vision ≥ ${threshold}` : "no template";
    }
    default:
      return "";
  }
}

export function FlowActionNode({ data, selected }: NodeProps<FlowCanvasNode>) {
  const presentation = ACTION_PRESENTATION[data.kind];
  if (!presentation) return null;
  const Icon = presentation.icon;
  const firstIssue = data.issues[0];

  return (
    <div className="flow-node" data-selected={selected || undefined}>
      {data.kind !== "start" && (
        <Handle type="target" position={Position.Left} id="flow" />
      )}
      <div className="flow-node-heading">
        <Icon aria-hidden="true" size={16} />
        <span className="flow-node-title">{presentation.label}</span>
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
