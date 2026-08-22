import {
  useCallback,
  useEffect,
  useReducer,
  useRef,
  useState,
} from "react";
import {
  flowActionCatalog,
  flowArchive,
  flowCancelRun,
  flowExport,
  flowGet,
  flowGetRun,
  flowList,
  flowListRuns,
  flowReadArtifact,
  flowRetryAttempt,
  flowRun,
  flowSaveRevision,
  flowValidate,
  listenRiviuEvents,
} from "../../api";
import { FlowDraftWriter, clearDraft, loadDraft } from "./draftStorage";
import {
  canStartSave,
  duplicateDocument,
  initialEditorState,
  isCompilationCurrent,
  newFlowDocument,
  reduceFlowEditor,
  type DocumentRequestIdentity,
} from "./editorState";
import { normalizeFlowIssues, validateDraftNumbers } from "../../flow/validation";
import { requestConfirm } from "../../confirmStore";
import type {
  ActionDefinition,
  DeviceInfo,
  FlowArtifactPayload,
  FlowDocumentV2,
  FlowRevisionRecord,
  FlowRunDetail,
  FlowRunRecord,
  FlowSummary,
  FlowTargetSelection,
} from "../../types";
import { FlowCanvas } from "./FlowCanvas";
import { FlowDiagnostics } from "./FlowDiagnostics";
import { FlowImportDialog } from "./FlowImportDialog";
import { FlowInspector } from "./FlowInspector";
import { FlowJsonDialog } from "./FlowJsonDialog";
import { FlowPalette } from "./FlowPalette";
import { FlowRunDialog } from "./FlowRunDialog";
import { FlowRunMonitor } from "./FlowRunMonitor";
import { FlowToolbar } from "./FlowToolbar";
import { describeError } from "../../describeError";

export interface FlowWorkspaceProps {
  devices: DeviceInfo[];
  selectedUdids: string[];
  onDirtyChange: (dirty: boolean) => void;
}

type OpenDialog = "import" | "json" | "run" | null;

function savedSummary(document: FlowDocumentV2, createdAt: string): FlowSummary {
  return {
    id: document.id,
    name: document.name,
    latestRevision: document.revision,
    archived: false,
    updatedAt: createdAt,
  };
}

function downloadJson(name: string, body: string): void {
  const blob = new Blob([body], { type: "application/json;charset=utf-8" });
  const href = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = href;
  anchor.download = `${name.trim().replace(/[^a-z0-9_-]+/gi, "-") || "flow"}.json`;
  anchor.click();
  URL.revokeObjectURL(href);
}

function detailForRun(run: FlowRunRecord): FlowRunDetail {
  return { run, deviceRuns: [], attempts: [], artifacts: [] };
}

function flowDocumentEvent(value: unknown): { flowId: string; revision: number } | null {
  if (typeof value !== "object" || value === null) return null;
  const event = value as Record<string, unknown>;
  return event.type === "flowUpdated" &&
    typeof event.flowId === "string" &&
    event.flowId.length > 0 &&
    typeof event.revision === "number" &&
    Number.isSafeInteger(event.revision) &&
    event.revision > 0
    ? { flowId: event.flowId, revision: event.revision }
    : null;
}

export function FlowWorkspace({
  devices,
  selectedUdids,
  onDirtyChange,
}: FlowWorkspaceProps) {
  const [state, dispatch] = useReducer(
    reduceFlowEditor,
    undefined,
    () => initialEditorState(),
  );
  const [catalog, setCatalog] = useState<ActionDefinition[]>([]);
  const [flows, setFlows] = useState<FlowSummary[]>([]);
  const [runs, setRuns] = useState<FlowRunRecord[]>([]);
  const [activeRun, setActiveRun] = useState<FlowRunDetail | null>(null);
  const [artifact, setArtifact] = useState<FlowArtifactPayload | null>(null);
  const [dialog, setDialog] = useState<OpenDialog>(null);
  const [paletteOpen, setPaletteOpen] = useState(true);
  const [inspectorOpen, setInspectorOpen] = useState(true);
  const [loading, setLoading] = useState(true);
  const [operationError, setOperationError] = useState<string | null>(null);
  const validationSequence = useRef(0);
  const invalidationSequence = useRef(0);
  const invalidationState = useRef({
    flowId: state.document.id,
    revision: state.document.revision,
    dirty: state.dirty,
  });
  invalidationState.current = {
    flowId: state.document.id,
    revision: state.document.revision,
    dirty: state.dirty,
  };
  const draftWriter = useRef<FlowDraftWriter | null>(null);
  if (draftWriter.current === null) draftWriter.current = new FlowDraftWriter();

  const replaceFromRecord = useCallback((record: FlowRevisionRecord) => {
    const draft = loadDraft(record.document.id);
    const canRestore = draft?.baseRevision === record.document.revision;
    dispatch({
      type: "replaceDocument",
      document: canRestore && draft ? draft.document : record.document,
      source: canRestore ? "draft" : "server",
    });
  }, []);

  const openSavedFlow = useCallback(async (id: string) => {
    setOperationError(null);
    const record = await flowGet(id);
    if (record === null) throw new Error("FlowNotFound");
    draftWriter.current?.cancel();
    replaceFromRecord(record);
  }, [replaceFromRecord]);

  useEffect(() => {
    let disposed = false;
    void (async () => {
      try {
        const [nextCatalog, nextFlows, nextRuns] = await Promise.all([
          flowActionCatalog(),
          flowList(),
          flowListRuns(100),
        ]);
        if (disposed) return;
        setCatalog(nextCatalog);
        setFlows(nextFlows);
        setRuns(nextRuns);
        if (nextFlows.length > 0) {
          const record = await flowGet(nextFlows[0].id);
          if (!disposed && record !== null) replaceFromRecord(record);
        }
      } catch (error) {
        if (!disposed) setOperationError(describeError(error));
      } finally {
        if (!disposed) setLoading(false);
      }
    })();
    return () => {
      disposed = true;
      draftWriter.current?.flush();
    };
  }, [replaceFromRecord]);

  useEffect(() => {
    onDirtyChange(state.dirty);
    if (state.dirty) draftWriter.current?.schedule(state.document);
    else {
      draftWriter.current?.cancel();
      clearDraft(state.document.id);
    }
  }, [onDirtyChange, state.dirty, state.document]);

  useEffect(() => {
    const identity: DocumentRequestIdentity = {
      requestId: ++validationSequence.current,
      flowId: state.document.id,
      documentEpoch: state.documentEpoch,
    };
    const snapshot = structuredClone(state.document);
    dispatch({ type: "validationStarted", identity });
    const timer = window.setTimeout(() => {
      const localIssues = validateDraftNumbers(snapshot);
      if (localIssues.length > 0) {
        dispatch({ type: "validationCompleted", identity, issues: localIssues, compiled: null });
        return;
      }
      void flowValidate(snapshot).then(
        (compiled) => dispatch({
          type: "validationCompleted",
          identity,
          issues: [],
          compiled,
        }),
        (error) => dispatch({
          type: "validationCompleted",
          identity,
          issues: normalizeFlowIssues(error),
          compiled: null,
        }),
      );
    }, 250);
    return () => window.clearTimeout(timer);
    // Keyed on the document's semantic version, not its object identity.
    // documentEpoch is bumped by every real edit, while panning the canvas
    // replaces the document object without changing what compiles — watching
    // identity restarted validation on each pan and discarded the in-flight
    // result (its requestId no longer matched), leaving compiled null.
  }, [state.documentEpoch]);

  const compiled = isCompilationCurrent(state) ? state.compiled?.value ?? null : null;
  const selectedNode = state.document.nodes.find((node) => node.id === state.selectedNodeId) ?? null;
  const selectedDefinition = selectedNode
    ? catalog.find((definition) => definition.kind === selectedNode.kind) ?? null
    : null;
  const saved = flows.some((flow) => flow.id === state.document.id);
  const coordinateUdid = selectedUdids.length === 1 ? selectedUdids[0] : null;
  const launchBundleId = compiled?.plan.contextPlan.initialBundleId ?? null;

  const refreshFlows = useCallback(async () => {
    const next = await flowList();
    setFlows(next);
    return next;
  }, []);

  const confirmDiscard = useCallback(async () => (
    !state.dirty ||
    (await requestConfirm({
      title: "Bỏ thay đổi Flow chưa lưu?",
      message: "Bản nháp hiện tại chưa được lưu và sẽ mất.",
      confirmLabel: "Bỏ thay đổi",
      cancelLabel: "Ở lại",
      danger: true,
    }))
  ), [state.dirty]);

  const selectFlow = useCallback(async (id: string) => {
    if (!(await confirmDiscard())) return;
    await openSavedFlow(id).catch((error) => setOperationError(describeError(error)));
  }, [confirmDiscard, openSavedFlow]);

  const replaceWithNew = useCallback((document: FlowDocumentV2, source: "new" | "duplicate") => {
    draftWriter.current?.cancel();
    setOperationError(null);
    setActiveRun(null);
    dispatch({ type: "replaceDocument", document, source });
  }, []);

  useEffect(() => {
    let disposed = false;
    let latestEventRevision = 0;
    let stop: (() => void) | undefined;

    const invalidate = async (flowId: string, eventRevision: number) => {
      if (eventRevision <= latestEventRevision) return;
      latestEventRevision = eventRevision;
      const requestId = ++invalidationSequence.current;
      const next = await flowList();
      if (disposed || requestId !== invalidationSequence.current) return;
      setFlows(next);

      const current = invalidationState.current;
      if (current.dirty || current.flowId !== flowId) return;
      const summary = next.find((item) => item.id === flowId);
      if (summary && summary.latestRevision <= current.revision) return;

      const nextFlowId = summary ? flowId : next[0]?.id;
      if (!nextFlowId) {
        replaceWithNew(newFlowDocument(), "new");
        return;
      }
      const record = await flowGet(nextFlowId);
      if (disposed || requestId !== invalidationSequence.current || record === null) return;
      const latest = invalidationState.current;
      if (latest.dirty || latest.flowId !== flowId) return;
      if (
        record.document.id === latest.flowId &&
        record.document.revision <= latest.revision
      ) return;
      draftWriter.current?.cancel();
      replaceFromRecord(record);
    };

    void listenRiviuEvents((payload) => {
      const event = flowDocumentEvent(payload);
      if (event) {
        void invalidate(event.flowId, event.revision).catch((error) => {
          if (!disposed) setOperationError(describeError(error));
        });
      }
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    }, (error) => {
      if (!disposed) setOperationError(describeError(error));
    });

    return () => {
      disposed = true;
      stop?.();
    };
  }, [replaceFromRecord, replaceWithNew]);

  const save = useCallback(() => {
    if (!state.compiled || !isCompilationCurrent(state)) return;
    const identity = { ...state.compiled.identity };
    if (!canStartSave(state, identity)) return;
    const snapshot = structuredClone(state.document);
    dispatch({ type: "saveStarted", identity });
    setOperationError(null);
    void flowSaveRevision(snapshot, snapshot.revision === 0 ? null : snapshot.revision).then(
      (record) => {
        dispatch({ type: "saveCompleted", identity, record });
        setFlows((current) => {
          const next = current.filter((item) => item.id !== record.document.id);
          return [savedSummary(record.document, record.createdAt), ...next];
        });
      },
      (error) => {
        dispatch({ type: "saveFailed", identity });
        setOperationError(describeError(error));
      },
    );
  }, [state]);

  const archive = useCallback(() => {
    if (!saved) return;
    void (async () => {
      const proceed = await requestConfirm({
        title: `Lưu trữ «${state.document.name}»?`,
        message: "Flow sẽ được đưa khỏi danh sách đang dùng. Các bản chạy đã ghi vẫn giữ nguyên.",
        confirmLabel: "Lưu trữ",
        danger: true,
      });
      if (!proceed) return;
      setOperationError(null);
      try {
        await flowArchive(state.document.id);
        clearDraft(state.document.id);
        const next = await refreshFlows();
        if (next.length > 0) await openSavedFlow(next[0].id);
        else replaceWithNew(newFlowDocument(), "new");
      } catch (error) {
        setOperationError(describeError(error));
      }
    })();
  }, [openSavedFlow, refreshFlows, replaceWithNew, saved, state.document.id, state.document.name]);

  const startRun = useCallback((selection: FlowTargetSelection) => {
    if (!saved || state.dirty || compiled === null) return;
    setOperationError(null);
    setDialog(null);
    void (async () => {
      try {
        const run = await flowRun(state.document.id, state.document.revision, selection);
        setRuns((current) => [run, ...current.filter((item) => item.id !== run.id)]);
        setActiveRun(detailForRun(run));
        const detail = await flowGetRun(run.id);
        if (detail) setActiveRun(detail);
      } catch (error) {
        setOperationError(describeError(error));
      }
    })();
  }, [compiled, saved, state.dirty, state.document.id, state.document.revision]);

  const selectRun = useCallback((runId: string) => {
    void flowGetRun(runId).then((detail) => setActiveRun(detail), (error) => {
      setOperationError(describeError(error));
    });
  }, []);

  const cancelRun = useCallback((runId: string) => {
    void (async () => {
      try {
        await flowCancelRun(runId);
        const detail = await flowGetRun(runId);
        if (detail) setActiveRun(detail);
      } catch (error) {
        setOperationError(describeError(error));
      }
    })();
  }, []);

  const retryAttempt = useCallback((attemptId: string) => {
    void (async () => {
      try {
        await flowRetryAttempt(attemptId);
        if (!activeRun) return;
        const detail = await flowGetRun(activeRun.run.id);
        if (detail) setActiveRun(detail);
      } catch (error) {
        setOperationError(describeError(error));
      }
    })();
  }, [activeRun]);

  const openArtifact = useCallback((artifactId: string) => {
    setOperationError(null);
    void flowReadArtifact(artifactId).then(setArtifact, (error) => {
      setOperationError(describeError(error));
    });
  }, []);

  return (
    <section className="flow-workspace" aria-label="Không gian Flow" data-loading={loading}>
      <FlowToolbar
        flows={flows}
        currentFlowId={saved ? state.document.id : null}
        flowName={state.document.name}
        dirty={state.dirty}
        canUndo={state.past.length > 0}
        canRedo={state.future.length > 0}
        compiled={compiled}
        issues={state.validation}
        catalog={catalog}
        validationPending={state.validationRequest !== null}
        savePending={state.saveRequest !== null}
        onRename={(name) => dispatch({ type: "renameFlow", name })}
        onSelectFlow={(id) => void selectFlow(id)}
        onNew={() => {
          void confirmDiscard().then((ok) => {
            if (ok) replaceWithNew(newFlowDocument(), "new");
          });
        }}
        onDuplicate={() => {
          void confirmDiscard().then((ok) => {
            if (ok) replaceWithNew(duplicateDocument(state.document), "duplicate");
          });
        }}
        onArchive={archive}
        onSave={save}
        onRun={() => setDialog("run")}
        onImport={() => setDialog("import")}
        onExport={() => {
          if (!saved) return;
          void flowExport(state.document.id, state.document.revision).then(
            (body) => downloadJson(state.document.name, body),
            (error) => setOperationError(describeError(error)),
          );
        }}
        onJson={() => setDialog("json")}
        onUndo={() => dispatch({ type: "undo" })}
        onRedo={() => dispatch({ type: "redo" })}
        onTogglePalette={() => setPaletteOpen((open) => !open)}
        onToggleInspector={() => setInspectorOpen((open) => !open)}
      />

      <div className="flow-notices" aria-live="polite">
        {operationError && (
          <div className="flow-operation-error" role="alert">
            <span>{operationError}</span>
            <button type="button" onClick={() => setOperationError(null)}>Bỏ qua</button>
          </div>
        )}
        {state.notice && (
          <div className="flow-operation-error" role="status">
            Revision {state.notice.savedRevision} saved; the newer local draft remains open.
            <button type="button" onClick={() => dispatch({ type: "dismissNotice" })}>Bỏ qua</button>
          </div>
        )}
      </div>

      <div
        className="flow-layout"
        data-palette-open={String(paletteOpen)}
        data-inspector-open={String(inspectorOpen)}
      >
        <FlowPalette catalog={catalog} open={paletteOpen} />
        <FlowCanvas
          key={state.document.id}
          document={state.document}
          catalog={catalog}
          issues={state.validation}
          selectedNodeId={state.selectedNodeId}
          onSelectNode={(nodeId) => dispatch({ type: "selectNode", nodeId })}
          onReplaceCanvas={(nodes, edges) => dispatch({ type: "replaceCanvas", nodes, edges })}
          onInsertNode={(edgeId, node) => dispatch({ type: "insertNode", edgeId, node })}
          onAppendNode={(node) => dispatch({ type: "appendNode", node })}
          onDeleteNode={(nodeId) => dispatch({ type: "deleteNode", nodeId })}
          onViewport={(viewport) => dispatch({ type: "setViewport", viewport })}
        />
        <div className="flow-inspector-shell" data-open={String(inspectorOpen)}>
          <FlowInspector
            node={selectedNode}
            definition={selectedDefinition}
            issues={state.validation}
            coordinateDeviceUdid={coordinateUdid}
            launchBundleId={launchBundleId}
            onConfigChange={(config) => {
              if (selectedNode) dispatch({
                type: "updateNodeConfig",
                nodeId: selectedNode.id,
                config,
              });
            }}
            onPostconditionChange={(postcondition) => {
              if (selectedNode) dispatch({
                type: "updateNodePostcondition",
                nodeId: selectedNode.id,
                postcondition,
              });
            }}
          />
        </div>
      </div>

      <div className="flow-lower-band">
        <FlowDiagnostics
          issues={state.validation}
          onSelectNode={(nodeId) => dispatch({ type: "selectNode", nodeId })}
        />
        <div className="flow-run-history" data-testid="flow-run-history">
          <label>
            <span>Lượt chạy</span>
            <select
              value={activeRun?.run.id ?? ""}
              onChange={(event) => selectRun(event.currentTarget.value)}
            >
              <option value="">Chưa chọn lượt chạy</option>
              {runs.map((run) => (
                <option key={run.id} value={run.id}>
                  {run.state} / {run.id.slice(0, 8)}
                </option>
              ))}
            </select>
          </label>
        </div>
        {activeRun ? (
          <FlowRunMonitor
            run={activeRun}
            onCancel={cancelRun}
            onRetry={retryAttempt}
            onOpenArtifact={openArtifact}
          />
        ) : (
          <section className="flow-monitor flow-monitor-empty" data-testid="flow-monitor">
            <span>Chưa có lượt chạy</span>
          </section>
        )}
      </div>

      {dialog === "run" && (
        <div className="flow-dialog-layer">
          <FlowRunDialog
            devices={devices}
            selectedUdids={selectedUdids}
            onRun={startRun}
            onClose={() => setDialog(null)}
          />
        </div>
      )}
      {dialog === "import" && (
        <div className="flow-dialog-layer">
          <FlowImportDialog
            onClose={() => setDialog(null)}
            onImport={(document) => {
              replaceWithNew(document, "new");
              setDialog(null);
            }}
          />
        </div>
      )}
      {dialog === "json" && (
        <div className="flow-dialog-layer">
          <FlowJsonDialog
            document={state.document}
            onClose={() => setDialog(null)}
            onApply={(document) => {
              replaceWithNew(document, "new");
              setDialog(null);
            }}
          />
        </div>
      )}
      {artifact && (
        <div className="flow-dialog-layer">
          <section className="flow-dialog flow-artifact-dialog" role="dialog" aria-label="Tệp kết quả">
            <header>
              <strong>{artifact.label}</strong>
              <button type="button" onClick={() => setArtifact(null)}>Đóng</button>
            </header>
            <img
              src={`data:${artifact.kind.includes("/") ? artifact.kind : `image/${artifact.kind}`};base64,${artifact.base64}`}
              alt={artifact.label}
            />
            <code>{artifact.sha256}</code>
          </section>
        </div>
      )}
    </section>
  );
}
