import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import {
  Archive,
  Clock3,
  GitBranch,
  Heart,
  ListChecks,
  MessageCircle,
  Play,
  Plus,
  RefreshCw,
  RotateCcw,
  Save,
  Send,
  Square,
  Trash2,
  Workflow,
} from "lucide-react";

import {
  automationList,
  orchestrationArchive,
  orchestrationCancelRun,
  orchestrationGet,
  orchestrationGetRun,
  orchestrationList,
  orchestrationListRuns,
  orchestrationReconcile,
  orchestrationRun,
  orchestrationSaveRevision,
  orchestrationValidate,
} from "../../api";
import { requestConfirm } from "../../confirmStore";
import { describeError } from "../../describeError";
import type {
  AutomationDefinition,
  AutomationKind,
  OrchestrationBranch,
  OrchestrationDocumentV1,
  OrchestrationNode,
  OrchestrationRunDetail,
  OrchestrationRunRecord,
  OrchestrationRunState,
  OrchestrationSummary,
  TargetRef,
} from "../../types";
import { EmptyState, LoadingState, StatusNotice } from "../States";

type Props = {
  onDirtyChange: (dirty: boolean) => void;
  targetRef?: TargetRef;
};

const RUN_STATE_LABEL: Record<OrchestrationRunState, string> = {
  queued: "Đang chờ",
  running: "Đang chạy",
  done: "Hoàn tất",
  partial: "Một phần",
  failed: "Thất bại",
  uncertain: "Chưa chắc chắn",
  cancelled: "Đã dừng",
};

const TERMINAL_RUN_STATES = new Set<OrchestrationRunState>([
  "done",
  "partial",
  "failed",
  "uncertain",
  "cancelled",
]);

const OUTCOMES: { branch: OrchestrationBranch; label: string }[] = [
  { branch: "done", label: "Hoàn tất" },
  { branch: "partial", label: "Một phần" },
  { branch: "failed", label: "Thất bại" },
  { branch: "uncertain", label: "Chưa chắc chắn" },
];

const CAMPAIGN_NODE: Record<AutomationKind, "runNurture" | "runInteraction" | "runPublish"> = {
  nurture: "runNurture",
  interaction: "runInteraction",
  publish: "runPublish",
};

const CAMPAIGN_LABEL: Record<AutomationKind, string> = {
  nurture: "Nuôi TikTok",
  interaction: "Tương tác",
  publish: "Đăng bài",
};

function randomId(): string {
  return crypto.randomUUID();
}

function newDocument(): OrchestrationDocumentV1 {
  const start = randomId();
  const end = randomId();
  return rebuildDocument({
    schemaVersion: 1,
    id: randomId(),
    revision: 0,
    name: "Điều phối mới",
    entryNodeId: start,
    nodes: [
      { id: start, kind: "start", position: { x: 40, y: 80 } },
      { id: end, kind: "end", position: { x: 520, y: 80 } },
    ],
    edges: [],
  });
}

function isBoundary(node: OrchestrationNode): boolean {
  return node.kind === "start" || node.kind === "end";
}

function isCampaign(node: OrchestrationNode): boolean {
  return node.kind === "runNurture" || node.kind === "runInteraction" || node.kind === "runPublish";
}

function rebuildDocument(document: OrchestrationDocumentV1): OrchestrationDocumentV1 {
  const start = document.nodes.find((node) => node.kind === "start");
  const end = document.nodes.find((node) => node.kind === "end");
  if (!start || !end) return document;
  const steps = document.nodes.filter((node) => !isBoundary(node));
  const nodes = [start, ...steps, end].map((node, index) => ({
    ...node,
    position: { x: 40 + index * 240, y: 80 },
  }));
  const edges = nodes.slice(0, -1).flatMap((source, index) => {
    const target = nodes[index + 1];
    const branches: OrchestrationBranch[] = isCampaign(source)
      ? ["done", "partial", "failed", "uncertain"]
      : ["done"];
    return branches.map((sourcePort) => ({
      sourceNodeId: source.id,
      sourcePort,
      targetNodeId: target.id,
    }));
  });
  return { ...document, entryNodeId: start.id, nodes, edges };
}

function nodeKind(node: OrchestrationNode): AutomationKind | null {
  if (node.kind === "runNurture") return "nurture";
  if (node.kind === "runInteraction") return "interaction";
  if (node.kind === "runPublish") return "publish";
  return null;
}

function profileForNode(
  node: OrchestrationNode,
  profiles: AutomationDefinition[],
): AutomationDefinition | null {
  if (!isCampaign(node) || !("profile" in node)) return null;
  return profiles.find((profile) => profile.id === node.profile.definitionId) ?? null;
}

export function OrchestrationWorkspace({
  onDirtyChange,
  targetRef = { type: "all" },
}: Props) {
  const [summaries, setSummaries] = useState<OrchestrationSummary[]>([]);
  const [profiles, setProfiles] = useState<AutomationDefinition[]>([]);
  const [document, setDocument] = useState<OrchestrationDocumentV1 | null>(null);
  const [savedRevision, setSavedRevision] = useState<number | null>(null);
  const [dirty, setDirty] = useState(false);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [mode, setMode] = useState<"setup" | "monitor">("setup");
  const [runs, setRuns] = useState<OrchestrationRunRecord[]>([]);
  const [selectedRun, setSelectedRun] = useState<OrchestrationRunDetail | null>(null);
  const selectedRunId = useRef<string | null>(null);
  const selectedDocumentId = useRef<string | null>(null);

  useEffect(() => onDirtyChange(dirty), [dirty, onDirtyChange]);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [nextSummaries, nextProfiles, nextRuns] = await Promise.all([
        orchestrationList(),
        automationList(),
        orchestrationListRuns(100),
      ]);
      setSummaries(nextSummaries);
      setProfiles(nextProfiles.filter((profile) => !profile.archived));
      setRuns(nextRuns);
    } catch (cause) {
      setError(describeError(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const edit = useCallback((update: (current: OrchestrationDocumentV1) => OrchestrationDocumentV1) => {
    setDocument((current) => (current ? rebuildDocument(update(current)) : current));
    setDirty(true);
    setNotice(null);
  }, []);

  const create = useCallback(() => {
    const next = newDocument();
    selectedDocumentId.current = next.id;
    setDocument(next);
    setSavedRevision(null);
    setDirty(true);
    setError(null);
    setNotice(null);
  }, []);

  const open = useCallback(async (summary: OrchestrationSummary) => {
    if (dirty) {
      const discard = await requestConfirm({
        title: "Bỏ thay đổi chưa lưu?",
        message: "Bản điều phối đang mở có thay đổi chưa lưu.",
        confirmLabel: "Bỏ thay đổi",
        cancelLabel: "Ở lại",
        danger: true,
      });
      if (!discard) return;
    }
    selectedDocumentId.current = summary.id;
    setBusy(true);
    setError(null);
    try {
      const record = await orchestrationGet(summary.id, summary.latestRevision);
      if (!record) throw new Error("Không tìm thấy revision điều phối");
      if (selectedDocumentId.current !== summary.id) return;
      setDocument(record.compiled.document);
      setSavedRevision(record.compiled.document.revision);
      setDirty(false);
      setNotice(null);
    } catch (cause) {
      if (selectedDocumentId.current === summary.id) setError(describeError(cause));
    } finally {
      if (selectedDocumentId.current === summary.id) setBusy(false);
    }
  }, [dirty]);

  const addCampaign = useCallback((kind: AutomationKind) => {
    const profile = profiles.find((candidate) => candidate.kind === kind);
    if (!profile) return;
    edit((current) => {
      const endIndex = current.nodes.findIndex((node) => node.kind === "end");
      const node: OrchestrationNode = {
        id: randomId(),
        kind: CAMPAIGN_NODE[kind],
        profile: { definitionId: profile.id, revision: profile.latestRevision },
        position: { x: 0, y: 0 },
      };
      const nodes = [...current.nodes];
      nodes.splice(endIndex < 0 ? nodes.length : endIndex, 0, node);
      return { ...current, nodes };
    });
  }, [edit, profiles]);

  const addDelay = useCallback(() => {
    edit((current) => {
      const endIndex = current.nodes.findIndex((node) => node.kind === "end");
      const nodes = [...current.nodes];
      nodes.splice(endIndex < 0 ? nodes.length : endIndex, 0, {
        id: randomId(),
        kind: "delay",
        durationMs: 5_000,
        position: { x: 0, y: 0 },
      });
      return { ...current, nodes };
    });
  }, [edit]);

  const removeStep = useCallback((nodeId: string) => {
    edit((current) => ({
      ...current,
      nodes: current.nodes.filter((node) => node.id !== nodeId || isBoundary(node)),
    }));
  }, [edit]);

  const save = useCallback(async () => {
    if (!document) return;
    setBusy(true);
    setError(null);
    try {
      await orchestrationValidate({ ...document, revision: (savedRevision ?? 0) + 1 });
      const record = await orchestrationSaveRevision(document, savedRevision);
      setDocument(record.compiled.document);
      setSavedRevision(record.compiled.document.revision);
      setDirty(false);
      setNotice(`Đã lưu bản ${record.compiled.document.revision}`);
      setSummaries(await orchestrationList());
    } catch (cause) {
      setError(describeError(cause));
    } finally {
      setBusy(false);
    }
  }, [document, savedRevision]);

  const updateRun = useCallback((detail: OrchestrationRunDetail) => {
    selectedRunId.current = detail.run.id;
    setSelectedRun(detail);
    setRuns((current) => [
      detail.run,
      ...current.filter((candidate) => candidate.id !== detail.run.id),
    ]);
  }, []);

  const startRun = useCallback(async () => {
    if (!document || savedRevision === null || dirty) return;
    const confirmed = await requestConfirm({
      title: "Chạy điều phối?",
      message: `Bản ${savedRevision} sẽ chạy trên phạm vi vừa chọn. Danh sách máy được chốt khi bắt đầu.`,
      confirmLabel: "Chạy",
      cancelLabel: "Hủy",
    });
    if (!confirmed) return;
    setBusy(true);
    setError(null);
    try {
      const detail = await orchestrationRun(document.id, savedRevision, targetRef);
      updateRun(detail);
      setMode("monitor");
    } catch (cause) {
      setError(describeError(cause));
    } finally {
      setBusy(false);
    }
  }, [dirty, document, savedRevision, targetRef, updateRun]);

  const openRun = useCallback(async (run: OrchestrationRunRecord) => {
    selectedRunId.current = run.id;
    setBusy(true);
    setError(null);
    try {
      const detail = await orchestrationGetRun(run.id);
      if (!detail) throw new Error("Không tìm thấy lần chạy điều phối");
      if (selectedRunId.current !== run.id) return;
      updateRun(detail);
    } catch (cause) {
      if (selectedRunId.current === run.id) setError(describeError(cause));
    } finally {
      if (selectedRunId.current === run.id) setBusy(false);
    }
  }, [updateRun]);

  const monitoredRunId = selectedRun?.run.id ?? null;
  const monitoredRunState = selectedRun?.run.state ?? null;
  useEffect(() => {
    if (
      busy ||
      mode !== "monitor" ||
      !monitoredRunId ||
      !monitoredRunState ||
      TERMINAL_RUN_STATES.has(monitoredRunState)
    ) {
      return;
    }
    const runId = monitoredRunId;
    let live = true;
    let reading = false;
    const poll = async () => {
      if (reading) return;
      reading = true;
      try {
        const detail = await orchestrationGetRun(runId);
        if (!live || selectedRunId.current !== runId) return;
        if (!detail) throw new Error("Không tìm thấy lần chạy điều phối");
        updateRun(detail);
        setError(null);
      } catch (cause) {
        if (live && selectedRunId.current === runId) setError(describeError(cause));
      } finally {
        reading = false;
      }
    };
    void poll();
    const timer = window.setInterval(() => void poll(), 1_000);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [busy, mode, monitoredRunId, monitoredRunState, updateRun]);

  const reconcileRun = useCallback(async () => {
    if (!selectedRun) return;
    const runId = selectedRun.run.id;
    setBusy(true);
    setError(null);
    try {
      const detail = await orchestrationReconcile(runId);
      if (selectedRunId.current !== runId) return;
      updateRun(detail);
    } catch (cause) {
      if (selectedRunId.current === runId) setError(describeError(cause));
    } finally {
      if (selectedRunId.current === runId) setBusy(false);
    }
  }, [selectedRun, updateRun]);

  const cancelRun = useCallback(async () => {
    if (!selectedRun || TERMINAL_RUN_STATES.has(selectedRun.run.state)) return;
    const confirmed = await requestConfirm({
      title: "Dừng điều phối?",
      message: "Chỉ phần việc chưa phát sinh hiệu lực được dừng. Hành động chưa chắc chắn sẽ không bị chạy lại.",
      confirmLabel: "Dừng",
      cancelLabel: "Tiếp tục chạy",
      danger: true,
    });
    if (!confirmed) return;
    const runId = selectedRun.run.id;
    setBusy(true);
    setError(null);
    try {
      const detail = await orchestrationCancelRun(runId);
      if (selectedRunId.current !== runId) return;
      updateRun(detail);
    } catch (cause) {
      if (selectedRunId.current === runId) setError(describeError(cause));
    } finally {
      if (selectedRunId.current === runId) setBusy(false);
    }
  }, [selectedRun, updateRun]);

  const archive = useCallback(async () => {
    if (!document || savedRevision === null) return;
    const confirmed = await requestConfirm({
      title: "Lưu trữ điều phối?",
      message: "Điều phối sẽ rời danh sách đang dùng. Các lần chạy cũ vẫn được giữ.",
      confirmLabel: "Lưu trữ",
      cancelLabel: "Hủy",
      danger: true,
    });
    if (!confirmed) return;
    setBusy(true);
    try {
      await orchestrationArchive(document.id);
      selectedDocumentId.current = null;
      setDocument(null);
      setSavedRevision(null);
      setDirty(false);
      setSummaries(await orchestrationList());
    } catch (cause) {
      setError(describeError(cause));
    } finally {
      setBusy(false);
    }
  }, [document, savedRevision]);

  const campaignCount = document?.nodes.filter(isCampaign).length ?? 0;
  const canSave = Boolean(document?.name.trim()) && campaignCount > 0 && !busy;
  const profilesByKind = useMemo(
    () => ({
      nurture: profiles.filter((profile) => profile.kind === "nurture"),
      interaction: profiles.filter((profile) => profile.kind === "interaction"),
      publish: profiles.filter((profile) => profile.kind === "publish"),
    }),
    [profiles],
  );

  const activateMode = (next: "setup" | "monitor") => {
    setMode(next);
    globalThis.document.getElementById(`orchestration-tab-${next}`)?.focus();
  };
  const onModeTabKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    let next: "setup" | "monitor" | null = null;
    if (event.key === "ArrowRight" || event.key === "End") next = "monitor";
    if (event.key === "ArrowLeft" || event.key === "Home") next = "setup";
    if (!next) return;
    event.preventDefault();
    activateMode(next);
  };

  if (loading) return <LoadingState label="Đang tải điều phối…" />;

  if (error && !document && summaries.length === 0) {
    return (
      <section className="orchestration-workspace" aria-label="Không gian Điều phối">
        <StatusNotice
          tone="error"
          action={(
            <button type="button" onClick={() => void load()}>
              <RefreshCw size={15} /> Thử lại
            </button>
          )}
        >
          {error}
        </StatusNotice>
      </section>
    );
  }

  return (
    <section className="orchestration-workspace" aria-label="Không gian Điều phối">
      <div className="orchestration-mode-tabs" role="tablist" aria-label="Chế độ Điều phối">
        <button
          id="orchestration-tab-setup"
          type="button"
          role="tab"
          aria-selected={mode === "setup"}
          aria-controls="orchestration-panel-setup"
          tabIndex={mode === "setup" ? 0 : -1}
          onClick={() => setMode("setup")}
          onKeyDown={onModeTabKeyDown}
        >
          Thiết lập
        </button>
        <button
          id="orchestration-tab-monitor"
          type="button"
          role="tab"
          aria-selected={mode === "monitor"}
          aria-controls="orchestration-panel-monitor"
          tabIndex={mode === "monitor" ? 0 : -1}
          onClick={() => setMode("monitor")}
          onKeyDown={onModeTabKeyDown}
        >
          Theo dõi điều phối
        </button>
      </div>

      <div
        id="orchestration-panel-setup"
        className="orchestration-mode-panel"
        role="tabpanel"
        aria-labelledby="orchestration-tab-setup"
        hidden={mode !== "setup"}
      >
        {mode === "setup" && (
          <>
          <aside className="orchestration-library" aria-label="Danh sách điều phối">
        <div className="orchestration-library-head">
          <strong>Điều phối</strong>
          <button type="button" className="icon-btn" onClick={create} title="Tạo điều phối mới" aria-label="Tạo điều phối mới">
            <Plus size={17} />
          </button>
        </div>
        {summaries.length === 0 ? (
          <EmptyState
            compact
            icon={<Workflow size={18} />}
            title="Chưa có điều phối nào"
            action={null}
          />
        ) : (
          <div className="orchestration-list">
            {summaries.map((summary) => (
              <button
                type="button"
                key={summary.id}
                className={document?.id === summary.id ? "is-active" : ""}
                onClick={() => void open(summary)}
              >
                <strong>{summary.name}</strong>
                <span>Bản {summary.latestRevision}</span>
              </button>
            ))}
          </div>
        )}
          </aside>

          <div className="orchestration-editor">
        {!document ? (
          <EmptyState
            icon={<GitBranch size={20} />}
            title="Chọn hoặc tạo một điều phối"
            action={<button type="button" onClick={create}>Tạo điều phối</button>}
          />
        ) : (
          <>
            <header className="orchestration-toolbar">
              <label>
                <span className="sr-only">Tên điều phối</span>
                <input
                  value={document.name}
                  onChange={(event) => edit((current) => ({ ...current, name: event.target.value }))}
                  aria-label="Tên điều phối"
                />
              </label>
              <span className="orchestration-revision">
                {savedRevision === null ? "Chưa lưu" : `Bản ${savedRevision}`}
              </span>
              <div className="grow" />
              {savedRevision !== null && (
                <button type="button" className="icon-btn" onClick={() => void archive()} title="Lưu trữ" aria-label="Lưu trữ">
                  <Archive size={16} />
                </button>
              )}
              <button
                type="button"
                disabled={savedRevision === null || dirty || busy}
                onClick={() => void startRun()}
              >
                <Play size={16} /> Chạy điều phối
              </button>
              <button type="button" className="primary" disabled={!canSave} onClick={() => void save()}>
                <Save size={16} /> Lưu bản
              </button>
            </header>

            {error && <StatusNotice tone="error">{error}</StatusNotice>}
            {notice && <StatusNotice tone="success">{notice}</StatusNotice>}

            <div className="orchestration-add" aria-label="Thêm bước">
              <button type="button" onClick={addDelay}><Clock3 size={15} /> Thêm Chờ</button>
              <button type="button" disabled={!profilesByKind.nurture.length} onClick={() => addCampaign("nurture")}>
                <Heart size={15} /> Thêm Nuôi TikTok
              </button>
              <button type="button" disabled={!profilesByKind.interaction.length} onClick={() => addCampaign("interaction")}>
                <MessageCircle size={15} /> Thêm Tương tác
              </button>
              <button type="button" disabled={!profilesByKind.publish.length} onClick={() => addCampaign("publish")}>
                <Send size={15} /> Thêm Đăng bài
              </button>
            </div>

            {profiles.length === 0 && (
              <StatusNotice tone="info">Chưa có hồ sơ tự động hóa để ghim vào điều phối.</StatusNotice>
            )}

            <div className="orchestration-canvas" aria-label="Các bước điều phối">
              {document.nodes.map((node, index) => {
                if (node.kind === "start" || node.kind === "end") {
                  return (
                    <div className="orchestration-node is-boundary" key={node.id}>
                      <strong>{node.kind === "start" ? "Bắt đầu" : "Kết thúc"}</strong>
                    </div>
                  );
                }
                const kind = nodeKind(node);
                const profile = profileForNode(node, profiles);
                return (
                  <div className="orchestration-node" key={node.id}>
                    <div className="orchestration-node-head">
                      <span className="orchestration-step">{index}</span>
                      <strong>{node.kind === "delay" ? "Chờ" : kind ? CAMPAIGN_LABEL[kind] : "Bước"}</strong>
                      <button type="button" className="icon-btn" onClick={() => removeStep(node.id)} title="Xóa bước" aria-label="Xóa bước">
                        <Trash2 size={15} />
                      </button>
                    </div>
                    {node.kind === "delay" ? (
                      <label className="orchestration-duration">
                        <span>Thời gian</span>
                        <input
                          type="number"
                          min={1}
                          max={86_400}
                          value={Math.round(node.durationMs / 1_000)}
                          onChange={(event) => {
                            const durationMs = Math.max(1, Number(event.target.value) || 1) * 1_000;
                            edit((current) => ({
                              ...current,
                              nodes: current.nodes.map((candidate) =>
                                candidate.id === node.id && candidate.kind === "delay"
                                  ? { ...candidate, durationMs }
                                  : candidate,
                              ),
                            }));
                          }}
                        />
                        <span>giây</span>
                      </label>
                    ) : kind && "profile" in node ? (
                      <>
                        <select
                          aria-label={`Hồ sơ ${CAMPAIGN_LABEL[kind]}`}
                          value={node.profile.definitionId}
                          onChange={(event) => {
                            const next = profilesByKind[kind].find((candidate) => candidate.id === event.target.value);
                            if (!next) return;
                            edit((current) => ({
                              ...current,
                              nodes: current.nodes.map((candidate) =>
                                candidate.id === node.id && "profile" in candidate
                                  ? { ...candidate, profile: { definitionId: next.id, revision: next.latestRevision } }
                                  : candidate,
                              ),
                            }));
                          }}
                        >
                          {profilesByKind[kind].map((candidate) => (
                            <option key={candidate.id} value={candidate.id}>{candidate.name}</option>
                          ))}
                        </select>
                        <div className="orchestration-profile">
                          <span>{profile ? "Đã ghim" : "Hồ sơ không còn khả dụng"}</span>
                          <strong>Bản {node.profile.revision}</strong>
                        </div>
                        <div className="orchestration-branches" aria-label="Nhánh kết quả">
                          {OUTCOMES.map((outcome) => <span key={outcome.branch}>{outcome.label}</span>)}
                        </div>
                      </>
                    ) : null}
                  </div>
                );
              })}
            </div>
          </>
        )}
          </div>
          </>
        )}
      </div>
      <div
        id="orchestration-panel-monitor"
        className="orchestration-mode-panel"
        role="tabpanel"
        aria-labelledby="orchestration-tab-monitor"
        hidden={mode !== "monitor"}
      >
        {mode === "monitor" && (
          <>
          <aside className="orchestration-library" aria-label="Danh sách lần chạy">
            <div className="orchestration-library-head">
              <strong>Lần chạy</strong>
              <button
                type="button"
                className="icon-btn"
                onClick={() => void load()}
                title="Làm mới danh sách"
                aria-label="Làm mới danh sách lần chạy"
              >
                <RefreshCw size={17} />
              </button>
            </div>
            {runs.length === 0 ? (
              <EmptyState
                compact
                icon={<ListChecks size={18} />}
                title="Chưa có lần chạy nào"
                action={null}
              />
            ) : (
              <div className="orchestration-list">
                {runs.map((run) => (
                  <button
                    type="button"
                    key={run.id}
                    className={selectedRun?.run.id === run.id ? "is-active" : ""}
                    onClick={() => void openRun(run)}
                  >
                    <strong>Bản {run.documentRevision} · {RUN_STATE_LABEL[run.state]}</strong>
                    <span>{run.target.included.length} máy</span>
                  </button>
                ))}
              </div>
            )}
          </aside>

          <div className="orchestration-editor orchestration-monitor">
            {error && <StatusNotice tone="error">{error}</StatusNotice>}
            {!selectedRun ? (
              <EmptyState
                icon={<ListChecks size={20} />}
                title="Chọn một lần chạy để theo dõi"
                action={null}
              />
            ) : (
              <>
                <header className="orchestration-toolbar">
                  <div className="orchestration-run-heading">
                    <strong>{RUN_STATE_LABEL[selectedRun.run.state]}</strong>
                    <span>Bản {selectedRun.run.documentRevision}</span>
                  </div>
                  <div className="grow" />
                  <button type="button" disabled={busy} onClick={() => void reconcileRun()}>
                    <RotateCcw size={16} /> Đối soát
                  </button>
                  {!TERMINAL_RUN_STATES.has(selectedRun.run.state) && (
                    <button type="button" className="danger" disabled={busy} onClick={() => void cancelRun()}>
                      <Square size={15} /> Dừng điều phối
                    </button>
                  )}
                </header>
                <div className="orchestration-run-summary">
                  <strong>{selectedRun.run.target.included.length} máy trong phạm vi đã chốt</strong>
                  <span>{selectedRun.attempts.length} bước đã ghi nhận</span>
                </div>
                {selectedRun.attempts.length === 0 ? (
                  <EmptyState compact title="Chưa có bước nào bắt đầu" action={null} />
                ) : (
                  <div className="orchestration-attempts" aria-label="Tiến độ điều phối">
                    {selectedRun.attempts.map((attempt) => (
                      <article key={attempt.snapshot.attemptId}>
                        <div>
                          <strong>{attempt.childKind ? CAMPAIGN_LABEL[attempt.childKind] : "Bước điều phối"}</strong>
                          <span>{attempt.branch ? OUTCOMES.find((item) => item.branch === attempt.branch)?.label : RUN_STATE_LABEL[selectedRun.run.state]}</span>
                        </div>
                        <span>{attempt.snapshot.target.included.length} máy</span>
                        {(attempt.errorCode || attempt.childCampaignId) && (
                          <details>
                            <summary>Chi tiết</summary>
                            {attempt.childCampaignId && <code>{attempt.childCampaignId}</code>}
                            {attempt.errorCode && <code>{attempt.errorCode}</code>}
                          </details>
                        )}
                      </article>
                    ))}
                  </div>
                )}
                {selectedRun.run.errorCode && (
                  <details className="orchestration-run-details">
                    <summary>Chi tiết lỗi</summary>
                    <code>{selectedRun.run.errorCode}</code>
                  </details>
                )}
              </>
            )}
          </div>
          </>
        )}
      </div>
    </section>
  );
}
