import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
} from "@xyflow/react";
import {
  ArrowLeft,
  Check,
  ChevronDown,
  Copy,
  GripVertical,
  Play,
  Plus,
  Redo2,
  Save,
  Search,
  Settings2,
  Smartphone,
  Trash2,
  Undo2,
  Workflow,
} from "lucide-react";
import { APP_STEP_ICONS, APP_STEP_FIELDS } from "./appStepPresentation";
import {
  appWorkflowCatalog,
  appWorkflowSave,
  appWorkflowValidate,
  type AppStepDefinition,
  type AppWorkflowV1,
} from "../appWorkflow";
import type { DeviceInfo, JsonValue } from "../types";
import { describeError } from "../describeError";
import { useWorkspaceDraft } from "../workspaceDraft";
import { requestWorkspaceLeave } from "../workspaceDraft";
import { AppWorkflowRunPanel } from "./AppWorkflowRunPanel";
import { AppWorkflowConfig } from "./AppWorkflowConfig";
import { PhoneCanvas } from "./PhoneCanvas";
import { startDevicePreview } from "../startPreview";

type StepNode = Node<
  { label: string; action: string; ports: string[] },
  "appStep"
>;
function Step({ data, selected }: NodeProps<StepNode>) {
  const Icon = APP_STEP_ICONS[data.action] ?? Workflow;
  return (
    <div
      className={`app-step app-step-${data.action}`}
      data-selected={selected || undefined}
    >
      {data.action !== "start" && (
        <Handle
          type="target"
          position={Position.Left}
          className="app-step-port"
        />
      )}
      <Icon size={15} />
      <span>{data.label}</span>
      {data.ports.map((port, index) => (
        <Handle
          key={port}
          id={port}
          title={port}
          type="source"
          position={Position.Right}
          className="app-step-port"
          style={{ top: `${((index + 1) / (data.ports.length + 1)) * 100}%` }}
        />
      ))}
    </div>
  );
}
const nodeTypes = { appStep: Step };
const MIME = "application/riviu-app-step";
type Props = {
  initial: AppWorkflowV1;
  devices: DeviceInfo[];
  onBack: () => void;
  onSaved: (doc: AppWorkflowV1) => void;
};

function Editor({ initial, devices, onBack, onSaved }: Props) {
  const [doc, setDoc] = useState(initial),
    [saved, setSaved] = useState(initial),
    [catalog, setCatalog] = useState<AppStepDefinition[]>([]);
  const [selected, setSelected] = useState<string | null>(null),
    [search, setSearch] = useState(""),
    [error, setError] = useState<string | null>(null),
    [notice, setNotice] = useState<string | null>(null),
    [busy, setBusy] = useState(false);
  const [history, setHistory] = useState<AppWorkflowV1[]>([]),
    [future, setFuture] = useState<AppWorkflowV1[]>([]),
    [preview, setPreview] = useState(false),
    [deviceId, setDeviceId] = useState("");
  const flow = useReactFlow<StepNode>();
  const latest = useRef(doc);
  latest.current = doc;
  const baseline = useRef(saved);
  baseline.current = saved;
  const [runOpen, setRunOpen] = useState(false),
    [configOpen, setConfigOpen] = useState(false);
  const [positions, setPositions] = useState<
    Record<string, { x: number; y: number }>
  >({});
  const dirty = JSON.stringify(doc) !== JSON.stringify(saved);
  useEffect(() => {
    let alive = true;
    void appWorkflowCatalog(initial.kind)
      .then((value) => {
        if (alive) setCatalog(value);
      })
      .catch((cause) => {
        if (alive) setError(describeError(cause));
      });
    return () => {
      alive = false;
    };
  }, [initial.kind]);
  const edit = useCallback(
    (update: (value: AppWorkflowV1) => AppWorkflowV1) => {
      setHistory((old) => [...old.slice(-99), latest.current]);
      setFuture([]);
      setDoc((current) => update(current));
      setNotice(null);
    },
    [],
  );
  const save = async () => {
    if (busy) return false;
    setBusy(true);
    setError(null);
    try {
      const value = latest.current;
      const result = await appWorkflowSave(value);
      setSaved(result);
      setDoc((current) =>
        current === value ? result : { ...current, revision: result.revision },
      );
      onSaved(result);
      setNotice(`Đã lưu bản ${result.revision}`);
      return latest.current === value;
    } catch (cause) {
      setError(describeError(cause));
      return false;
    } finally {
      setBusy(false);
    }
  };
  useWorkspaceDraft({
    id: `app-workflow-${initial.id}`,
    label: doc.name,
    dirty,
    snapshotKey: JSON.stringify(doc),
    save,
    discard: () => {
      setDoc(baseline.current);
      setHistory([]);
      setFuture([]);
      setPositions({});
    },
  });
  const undo = () => {
    const previous = history.at(-1);
    if (previous) {
      setFuture((old) => [latest.current, ...old]);
      setHistory((old) => old.slice(0, -1));
      setDoc({ ...previous, revision: saved.revision });
      setPositions({});
    }
  };
  const redo = () => {
    const next = future[0];
    if (next) {
      setHistory((old) => [...old, latest.current]);
      setFuture((old) => old.slice(1));
      setDoc({ ...next, revision: saved.revision });
      setPositions({});
    }
  };
  const node = doc.nodes.find((node) => node.id === selected);
  const nodes: StepNode[] = useMemo(
    () =>
      doc.nodes.map((n) => ({
        id: n.id,
        type: "appStep",
        position: positions[n.id] ?? n.position,
        selected: n.id === selected,
        data: {
          label: catalog.find((a) => a.action === n.action)?.label ?? n.action,
          action: n.action,
          ports: catalog.find((a) => a.action === n.action)?.ports ?? [],
        },
      })),
    [doc.nodes, catalog, positions, selected],
  );
  const edges = useMemo(
    () =>
      doc.edges.map((e) => ({
        id: e.id,
        source: e.source,
        target: e.target,
        sourceHandle: e.port,
        label: e.port === "done" ? undefined : e.port,
        type: "smoothstep",
      })),
    [doc.edges],
  );
  const add = (action: string, point?: { x: number; y: number }) => {
    const def = catalog.find((d) => d.action === action);
    if (!def || action === "start") return;
    const id = crypto.randomUUID();
    edit((current) => ({
      ...current,
      nodes: [
        ...current.nodes,
        {
          id,
          action,
          position:
            point ??
            flow.screenToFlowPosition({
              x: window.innerWidth / 2,
              y: window.innerHeight / 2,
            }),
          config: structuredClone(def.defaultConfig),
        },
      ],
    }));
    setSelected(id);
  };
  const remove = () => {
    if (!node || node.action === "start" || node.action === "end") return;
    edit((current) => {
      const incoming = current.edges.filter((e) => e.target === node.id),
        out = current.edges.filter((e) => e.source === node.id);
      const edges = current.edges.filter(
        (e) => e.source !== node.id && e.target !== node.id,
      );
      if (out.length === 1)
        edges.push(...incoming.map((e) => ({ ...e, target: out[0].target })));
      return {
        ...current,
        nodes: current.nodes.filter((n) => n.id !== node.id),
        edges,
      };
    });
    setSelected(null);
  };
  const changeConfig = (key: string, value: JsonValue) => {
    if (node)
      edit((current) => ({
        ...current,
        nodes: current.nodes.map((n) =>
          n.id === node.id
            ? { ...n, config: { ...n.config, [key]: value } }
            : n,
        ),
      }));
  };
  return (
    <section
      className="app-workflow-editor"
      aria-label={`Trình thiết kế ${doc.name}`}
    >
      <div className="app-editor-tab">
        <Workflow size={17} />
        <span>{doc.name}</span>
        <button
          type="button"
          aria-label="Ứng dụng mới"
          onClick={async () => {
            if (await requestWorkspaceLeave()) onBack();
          }}
        >
          <Plus size={16} />
        </button>
      </div>
      <header className="app-editor-toolbar">
        <button
          type="button"
          onClick={async () => {
            if (await requestWorkspaceLeave()) onBack();
          }}
        >
          <ArrowLeft size={16} />
          Quay lại
        </button>
        <label className="app-node-search">
          <Search size={16} />
          <input
            aria-label="Tìm node hoặc ID"
            placeholder="Tìm node hoặc ID…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            onKeyDown={event=>{if(event.key!=="Enter")return;const found=doc.nodes.find(node=>`${node.id} ${catalog.find(step=>step.action===node.action)?.label??node.action}`.toLowerCase().includes(search.toLowerCase()));if(found){setSelected(found.id);void flow.fitView({nodes:[{id:found.id}],maxZoom:1,duration:150});}}}
          />
        </label>
        <button
          type="button"
          title="Hoàn tác"
          disabled={!history.length || busy}
          onClick={undo}
        >
          <Undo2 size={16} />
        </button>
        <button
          type="button"
          title="Làm lại"
          disabled={!future.length || busy}
          onClick={redo}
        >
          <Redo2 size={16} />
        </button>
        <div className="grow" />
        <button
          type="button"
          aria-pressed={preview}
          onClick={() => setPreview((v) => !v)}
        >
          <Smartphone size={16} />
          Xem thiết bị
        </button>
        <button
          type="button"
          title="Cấu hình ứng dụng"
          onClick={() => setConfigOpen(true)}
        >
          <Settings2 size={18} />
        </button>
        <button
          type="button"
          onClick={async () => {
            try {
              await appWorkflowValidate(doc);
              setNotice("Quy trình hợp lệ");
              setError(null);
            } catch (cause) {
              setError(describeError(cause));
            }
          }}
        >
          <Check size={16} />
          Kiểm tra
        </button>
        <button
          type="button"
          disabled={busy || (!dirty && doc.revision > 0)}
          onClick={() => void save()}
        >
          <Save size={16} />
          Lưu
        </button>
        <button
          className="primary"
          type="button"
          disabled={dirty || busy || doc.revision === 0}
          onClick={() => setRunOpen(true)}
        >
          <Play size={16} />
          Chạy
        </button>
      </header>
      {(error || notice) && (
        <div
          className="app-editor-notice"
          role={error ? "alert" : "status"}
          data-error={!!error}
        >
          {error ?? notice}
        </div>
      )}
      {runOpen && (
        <AppWorkflowRunPanel
          document={saved}
          devices={devices}
          onClose={() => setRunOpen(false)}
        />
      )}
      {configOpen && (
        <AppWorkflowConfig
          document={doc}
          onChange={(profileConfig) =>
            edit((current) => ({ ...current, profileConfig }))
          }
          onClose={() => setConfigOpen(false)}
        />
      )}
      <div className="app-editor-body">
        <aside className="app-action-library">
          <div className="app-library-head">
            <input
              aria-label="Tên ứng dụng"
              value={doc.name}
              onChange={(e) =>
                edit((current) => ({ ...current, name: e.target.value }))
              }
            />
            <small>bản {doc.revision || "nháp"}</small>
          </div>
          <select
            aria-label="Thiết bị xem thử"
            value={deviceId}
            onChange={(e) => {
              setDeviceId(e.target.value);
              const device = devices.find((d) => d.udid === e.target.value);
              if (device)
                void startDevicePreview(device).catch((cause) =>
                  setError(describeError(cause)),
                );
            }}
          >
            <option value="">Chọn thiết bị</option>
            {devices.map((d) => (
              <option key={d.udid} value={d.udid}>
                {d.name}
              </option>
            ))}
          </select>
          {[...new Set(catalog.map((d) => d.category))].map((category) => (
            <details open key={category}>
              <summary>
                {category}
                <ChevronDown size={14} />
              </summary>
              {catalog
                .filter(
                  (d) =>
                    d.category === category &&
                    d.action !== "start" &&
                    `${d.label} ${d.action}`
                      .toLowerCase()
                      .includes(search.toLowerCase()),
                )
                .map((def) => (
                  <button
                    draggable
                    key={def.action}
                    type="button"
                    onDragStart={(e) => {
                      e.dataTransfer.setData(MIME, def.action);
                      e.dataTransfer.effectAllowed = "copy";
                    }}
                    onClick={() => add(def.action)}
                  >
                    <GripVertical size={14} />
                    {def.label}
                  </button>
                ))}
            </details>
          ))}
        </aside>
        <div
          className="app-graph"
          onDragOver={(e) => {
            if (e.dataTransfer.types.includes(MIME)) {
              e.preventDefault();
              e.dataTransfer.dropEffect = "copy";
            }
          }}
          onDrop={(e) => {
            const action = e.dataTransfer.getData(MIME);
            if (action) {
              e.preventDefault();
              add(
                action,
                flow.screenToFlowPosition({ x: e.clientX, y: e.clientY }),
              );
            }
          }}
        >
          {catalog.length>0&&<ReactFlow<StepNode>
            nodes={nodes}
            edges={edges}
            nodeTypes={nodeTypes}
            fitView={initial.revision === 0}
            defaultViewport={initial.viewport}
            fitViewOptions={{ maxZoom: 1, padding: 0.2 }}
            minZoom={0.2}
            maxZoom={2}
            deleteKeyCode={null}
            onNodeClick={(_, n) => setSelected(n.id)}
            onPaneClick={() => setSelected(null)}
            onNodesChange={(changes) => {
              for (const change of changes) {
                if (change.type === "position" && change.position) {
                  const p = change.position;
                  if (change.dragging)
                    setPositions((old) => ({ ...old, [change.id]: p }));
                  else {
                    edit((current) => ({
                      ...current,
                      nodes: current.nodes.map((n) =>
                        n.id === change.id ? { ...n, position: p } : n,
                      ),
                    }));
                    setPositions((old) => {
                      const next = { ...old };
                      delete next[change.id];
                      return next;
                    });
                  }
                }
                if (change.type === "select" && change.selected)
                  setSelected(change.id);
              }
            }}
            onConnect={(c) => {
              if (c.source && c.target && c.sourceHandle)
                edit((current) => ({
                  ...current,
                  edges: [
                    ...current.edges.filter(
                      (e) => e.source !== c.source || e.port !== c.sourceHandle,
                    ),
                    {
                      id: crypto.randomUUID(),
                      source: c.source!,
                      target: c.target!,
                      port: c.sourceHandle!,
                    },
                  ],
                }));
            }}
            onMoveEnd={(_, viewport) => {
              if (
                JSON.stringify(viewport) !==
                JSON.stringify(latest.current.viewport)
              )
                setDoc((current) => ({ ...current, viewport }));
            }}
            proOptions={{ hideAttribution: true }}
          >
            <Background gap={32} size={1} />
            <Controls showInteractive={false} />
          </ReactFlow>}
        </div>
        {(node || preview) && (
          <aside className="app-step-inspector">
            {preview && (
              <div className="app-device-preview">
                {deviceId ? (
                  <PhoneCanvas
                    udid={deviceId}
                    surfaceId={`app-editor-${initial.id}`}
                  />
                ) : (
                  <p>Chọn thiết bị ở bên trái để xem.</p>
                )}
              </div>
            )}
            {node && (
              <>
                <header>
                  <strong>
                    {catalog.find((d) => d.action === node.action)?.label}
                  </strong>
                  <button
                    type="button"
                    title="Nhân bản bước"
                    onClick={() =>
                      add(node.action, {
                        x: node.position.x + 30,
                        y: node.position.y + 90,
                      })
                    }
                  >
                    <Copy size={15} />
                  </button>
                  <button
                    type="button"
                    title="Xóa bước"
                    disabled={["start", "end"].includes(node.action)}
                    onClick={remove}
                  >
                    <Trash2 size={15} />
                  </button>
                </header>
                <small className="app-node-id">{node.id}</small>
                {Object.entries(node.config).map(([key, value]) => (
                  <label key={key}>
                    {APP_STEP_FIELDS[key] ?? key}
                    <input
                      type={typeof value === "number" ? "number" : "text"}
                      value={
                        typeof value === "object"
                          ? JSON.stringify(value)
                          : String(value ?? "")
                      }
                      onChange={(e) =>
                        changeConfig(
                          key,
                          typeof value === "number"
                            ? Number(e.target.value)
                            : e.target.value,
                        )
                      }
                    />
                  </label>
                ))}
                <details>
                  <summary>Thông tin bước</summary>
                  <p>Cấu hình của bước được lưu cùng phiên bản ứng dụng.</p>
                </details>
              </>
            )}
          </aside>
        )}
      </div>
    </section>
  );
}
export function AppWorkflowEditor(props: Props) {
  return (
    <ReactFlowProvider>
      <Editor {...props} />
    </ReactFlowProvider>
  );
}
