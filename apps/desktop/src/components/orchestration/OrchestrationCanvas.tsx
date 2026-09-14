import { useMemo, useState } from "react";
import {
  Background,
  Controls,
  Handle,
  Position,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Node,
  type NodeProps,
  type Connection,
} from "@xyflow/react";
import {
  CirclePlay,
  CircleStop,
  Clock3,
  MessagesSquare,
  Send,
  Sprout,
} from "lucide-react";
import type { AutomationKind, OrchestrationDocumentV1 } from "../../types";
import { AUTOMATION_APP_MIME } from "./automationDrag";

const LABELS: Record<string, string> = {
  start: "Bắt đầu",
  end: "Kết thúc",
  delay: "Chờ",
  log: "Ghi nhật ký",
  runNurture: "Nuôi TikTok",
  runInteraction: "Tương tác",
  runPublish: "Đăng bài",
};
const ICONS = {
  start: CirclePlay,
  end: CircleStop,
  delay: Clock3,
  log: MessagesSquare,
  runNurture: Sprout,
  runInteraction: MessagesSquare,
  runPublish: Send,
};
type Card = Node<{ kind: keyof typeof ICONS; detail: string }, "application">;
function ApplicationNode({ data, selected }: NodeProps<Card>) {
  const Icon = ICONS[data.kind];
  return (
    <div className="application-node" data-selected={selected || undefined}>
      {data.kind !== "start" && (
        <Handle
          className="application-port"
          type="target"
          position={Position.Left}
          id="input"
        />
      )}
      <span className="application-node-icon">
        <Icon size={20} />
      </span>
      <div>
        <strong>{LABELS[data.kind]}</strong>
        <small>{data.detail}</small>
      </div>
      {data.kind !== "end" && (
        <Handle
          className="application-port"
          type="source"
          position={Position.Right}
          id="done"
        />
      )}
    </div>
  );
}
const nodeTypes = { application: ApplicationNode };
function Canvas({
  document,
  onPosition,
  onSelect,
  onDropApp,
  onConnect,
  selectedId,
}: Props) {
  const flow = useReactFlow<Card>();
  const [positions, setPositions] = useState<
    Record<string, { x: number; y: number }>
  >({});
  const nodes = useMemo(
    () =>
      document.nodes.map((n) => ({
        id: n.id,
        type: "application" as const,
        position: positions[n.id] ?? n.position,
        selected: n.id === selectedId,
        data: {
          kind: n.kind as keyof typeof ICONS,
          detail:
            "profile" in n
              ? `Cấu hình · bản ${n.profile.revision}`
              : n.kind === "delay"
                ? `${n.durationMs / 1000} giây`
                : n.kind === "start"
                  ? "Điểm bắt đầu"
                  : "Hoàn tất quy trình",
        },
      })),
    [document, positions, selectedId],
  );
  const edges = useMemo(
    () =>
      document.edges
        .filter((e) => e.sourcePort === "done")
        .map((e) => ({
          id: `${e.sourceNodeId}:${e.sourcePort}`,
          source: e.sourceNodeId,
          target: e.targetNodeId,
          sourceHandle: "done",
          targetHandle: "input",
          type: "smoothstep",
          style: { stroke: "var(--primary)", strokeWidth: 1.6 },
        })),
    [document],
  );
  return (
    <div
      className="application-canvas"
      aria-label="Canvas quy trình kéo thả"
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes(AUTOMATION_APP_MIME)) {
          e.preventDefault();
          e.dataTransfer.dropEffect = "copy";
        }
      }}
      onDrop={(e) => {
        const kind = e.dataTransfer.getData(AUTOMATION_APP_MIME);
        if (!["nurture", "interaction", "publish"].includes(kind)) return;
        e.preventDefault();
        onDropApp(
          kind as AutomationKind,
          flow.screenToFlowPosition({ x: e.clientX, y: e.clientY }),
        );
      }}
    >
      <ReactFlow<Card>
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        fitView
        fitViewOptions={{ padding: 0.25, maxZoom: 1 }}
        minZoom={0.25}
        maxZoom={1.5}
        onNodeClick={(_, n) => onSelect(n.id)}
        onPaneClick={() => onSelect(null)}
        onNodesChange={(changes) => {
          for (const c of changes) {
            if (c.type === "position" && c.position) {
              const p = c.position;
              if (c.dragging) {
                setPositions((old) => ({ ...old, [c.id]: p }));
              } else {
                // Commit once at drag end or on a keyboard move. Drop the local
                // preview so discarding the draft restores document coordinates.
                onPosition(c.id, p);
                setPositions((old) => {
                  const next = { ...old };
                  delete next[c.id];
                  return next;
                });
              }
            }
            if (c.type === "select" && c.selected) onSelect(c.id);
          }
        }}
        onConnect={onConnect}
        deleteKeyCode={null}
        proOptions={{ hideAttribution: true }}
      >
        <Background gap={24} size={1} />
        <Controls showInteractive={false} />
      </ReactFlow>
      <div className="application-canvas-hint">
        Kéo ứng dụng vào đây · Chọn khối để thiết lập
      </div>
    </div>
  );
}
type Props = {
  document: OrchestrationDocumentV1;
  selectedId: string | null;
  onPosition: (id: string, p: { x: number; y: number }) => void;
  onSelect: (id: string | null) => void;
  onDropApp: (kind: AutomationKind, p: { x: number; y: number }) => void;
  onConnect: (c: Connection) => void;
};
export function OrchestrationCanvas(props: Props) {
  return (
    <ReactFlowProvider>
      <Canvas {...props} />
    </ReactFlowProvider>
  );
}
