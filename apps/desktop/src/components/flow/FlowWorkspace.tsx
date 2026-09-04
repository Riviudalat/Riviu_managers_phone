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
  initialLaunchBundleId,
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
import { LoadingState, StatusNotice } from "../States";
import { useMediaQuery } from "../../useMediaQuery";

export interface FlowWorkspaceProps {
  devices: DeviceInfo[];
  deviceLabel?: (device: DeviceInfo, index: number) => string;
  selectedUdids: string[];
  onDirtyChange: (dirty: boolean) => void;
}

type OpenDialog = "import" | "json" | "run" | null;
type WorkspaceLoadState = "loading" | "error" | "empty" | "data";

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

function runStateLabel(state: FlowRunRecord["state"]): string {
  const labels: Record<FlowRunRecord["state"], string> = {
    queued: "Đang chờ",
    running: "Đang chạy",
    succeeded: "Thành công",
    partial: "Một phần",
    failed: "Thất bại",
    cancelled: "Đã hủy",
  };
  return labels[state] ?? "Trạng thái chưa nhận diện";
}

export function FlowWorkspace({
  devices,
  deviceLabel,
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
  const compactLayout = useMediaQuery("(max-width: 1100px)");
  const [paletteOpenOverride, setPaletteOpenOverride] = useState<boolean | null>(null);
  const paletteOpen = paletteOpenOverride ?? !compactLayout;
  const [inspectorOpen, setInspectorOpen] = useState(true);
  const [loadState, setLoadState] = useState<WorkspaceLoadState>("loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loadAttempt, setLoadAttempt] = useState(0);
  const [operationError, setOperationError] = useState<string | null>(null);
  const validationSequence = useRef(0);
  const invalidationSequence = useRef(0);
  const invalidationState = useRef({
    flowId: state.document.id,
    revision: state.document.revision,
    dirty: state.dirty,
    // The open ticket compares epochs, not `dirty`: a blank document is born dirty
    // (`revision === 0`), so "is it dirty now" cannot tell an edit made during a fetch from
    // a document that was simply new when the fetch started. The epoch moves only on real
    // edits — panning does not bump it, and neither does time.
    epoch: state.documentEpoch,
  });
  invalidationState.current = {
    flowId: state.document.id,
    revision: state.document.revision,
    dirty: state.dirty,
    epoch: state.documentEpoch,
  };
  const [draftError, setDraftError] = useState<string | null>(null);
  const draftWriter = useRef<FlowDraftWriter | null>(null);
  if (draftWriter.current === null) {
    // The debounced write runs on a timer, so its failure has no caller to return to. Without a
    // channel a quota error simply vanished: the graph stayed dirty on screen and the recovery
    // draft that was supposed to survive a shutdown had never been written.
    draftWriter.current = new FlowDraftWriter(localStorage, 300, (reason) =>
      setDraftError(describeError(reason)),
    );
  }

  const replaceFromRecord = useCallback((record: FlowRevisionRecord) => {
    const draft = loadDraft(record.document.id);
    const canRestore = draft?.baseRevision === record.document.revision;
    dispatch({
      type: "replaceDocument",
      document: canRestore && draft ? draft.document : record.document,
      source: canRestore ? "draft" : "server",
    });
  }, []);

  // Every path that replaces the document takes a ticket, and only the newest ticket's
  // response may replace. Without it `flowGet` resolved whenever it resolved: a slower
  // older open could land after a newer one and win, and — worse — an edit typed while the
  // request was in flight was silently destroyed together with its undo history, because
  // the discard confirmation had been answered before the typing existed. The invalidation
  // effect below already lives by this rule; these are the two doors that did not.
  const openSequence = useRef(0);

  const openSavedFlow = useCallback(async (id: string) => {
    setOperationError(null);
    const ticket = ++openSequence.current;
    // Captured at issue time: whatever discard the operator confirmed covered the document
    // as it stood THEN. An epoch that moved while the request was in flight is typing that
    // answer never covered, so the response is dropped rather than the keystrokes.
    const epochAtIssue = invalidationState.current.epoch;
    const record = await flowGet(id);
    if (record === null) throw new Error("FlowNotFound");
    if (ticket !== openSequence.current) return;
    if (invalidationState.current.epoch !== epochAtIssue) return;
    draftWriter.current?.cancel();
    replaceFromRecord(record);
  }, [replaceFromRecord]);

  useEffect(() => {
    let disposed = false;
    setLoadState("loading");
    setLoadError(null);
    void (async () => {
      try {
        const [nextCatalog, nextFlows, nextRuns] = await Promise.all([
          flowActionCatalog(),
          flowList(),
          flowListRuns(100),
        ]);
        if (disposed) return;
        let initialRecord: FlowRevisionRecord | null = null;
        if (nextFlows.length > 0) {
          const ticket = ++openSequence.current;
          const epochAtIssue = invalidationState.current.epoch;
          const record = await flowGet(nextFlows[0].id);
          if (record === null) throw new Error("FlowNotFound");
          // The workspace is already editable while this first request runs, so it obeys
          // the same two rules as any open: newest ticket wins, and typing that happened
          // during the fetch must not be replaced away.
          if (
            !disposed &&
            ticket === openSequence.current &&
            invalidationState.current.epoch === epochAtIssue
          ) {
            initialRecord = record;
          }
        }
        if (disposed) return;
        // Publish the projections together. A partial catalog/list/runs snapshot must never look
        // like a valid editor, because it can make valid actions disappear or hide a live run.
        setCatalog(nextCatalog);
        setFlows(nextFlows);
        setRuns(nextRuns);
        if (initialRecord) replaceFromRecord(initialRecord);
        setLoadState(nextFlows.length === 0 ? "empty" : "data");
      } catch (error) {
        if (!disposed) {
          setLoadError(describeError(error));
          setLoadState("error");
        }
      }
    })();
    return () => {
      disposed = true;
      draftWriter.current?.flush();
    };
  }, [loadAttempt, replaceFromRecord]);

  const hasUnsavedWork = state.dirty && state.documentEpoch > 0;

  useEffect(() => {
    onDirtyChange(hasUnsavedWork);
    if (hasUnsavedWork) {
      try {
        draftWriter.current?.schedule(state.document);
        setDraftError(null);
      } catch (reason) {
        // `schedule` throws synchronously on a document it will not store, and this is an effect
        // body: an escaping throw unmounts the whole editor. That is exactly what happened when
        // the local shape check did not know about the vision action kinds -- adding a Tap Vision
        // node crashed the editor. The kinds are fixed, but the *shape* of the mistake is what
        // needs closing: autosave must never be able to take the editor down with it.
        setDraftError(describeError(reason));
      }
    } else {
      draftWriter.current?.cancel();
      clearDraft(state.document.id);
      setDraftError(null);
    }
  }, [hasUnsavedWork, onDirtyChange, state.document]);

  useEffect(() => {
    const identity: DocumentRequestIdentity = {
      requestId: ++validationSequence.current,
      flowId: state.document.id,
      documentEpoch: state.documentEpoch,
    };
    // Deliberate: the dependency array below records why this effect is keyed on the
    // epoch and not on the document. exhaustive-deps is switched off for this file in
    // .oxlintrc.json -- oxlint 1.x does not honour an inline disable for this rule.
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
  // Read from the document first: `compiled` is null after every edit, and that is precisely when
  // the operator needs the coordinate picker.
  const launchBundleId =
    initialLaunchBundleId(state.document) ?? compiled?.plan.contextPlan.initialBundleId ?? null;

  const refreshFlows = useCallback(async () => {
    const next = await flowList();
    setFlows(next);
    return next;
  }, []);

  const confirmDiscard = useCallback(async () => (
    !hasUnsavedWork ||
    (await requestConfirm({
      title: "Bỏ thay đổi Flow chưa lưu?",
      message: "Bản nháp hiện tại chưa được lưu và sẽ mất.",
      confirmLabel: "Bỏ thay đổi",
      cancelLabel: "Ở lại",
      danger: true,
    }))
  ), [hasUnsavedWork]);

  const selectFlow = useCallback(async (id: string) => {
    if (!(await confirmDiscard())) return;
    await openSavedFlow(id).catch((error) => setOperationError(describeError(error)));
  }, [confirmDiscard, openSavedFlow]);

  const replaceWithNew = useCallback((document: FlowDocumentV2, source: "new" | "duplicate") => {
    // A new or duplicated document is the newest intent, so it retires every open still in
    // flight — otherwise a late `flowGet` response would replace the document just created.
    openSequence.current += 1;
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

    void listenRiviuEvents((event) => {
      // A blank id or a revision of 0 would mean the backend announced a projection it has
      // not written yet; invalidating on that fetches nothing and drops the draft.
      if (event.type !== "flowUpdated") return;
      if (!event.flowId || !Number.isSafeInteger(event.revision) || event.revision <= 0) return;
      void invalidate(event.flowId, event.revision).catch((error) => {
        if (!disposed) setOperationError(describeError(error));
      });
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
      // The archive confirmation talks about the flow leaving the active list; it says nothing
      // about unsaved work, and the handler then calls `clearDraft` and opens another flow. So an
      // operator who added three actions and pressed Archive before Save lost those actions to a
      // dialog that never mentioned them. `New`, `Duplicate` and picking another flow all ask
      // first; this did not.
      if (!(await confirmDiscard())) return;
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
  }, [
    confirmDiscard,
    openSavedFlow,
    refreshFlows,
    replaceWithNew,
    saved,
    state.document.id,
    state.document.name,
  ]);

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

  if (loadState === "loading") {
    return (
      <section
        className="flow-workspace"
        aria-label="Không gian Flow"
        data-loading="true"
        data-state="loading"
      >
        <LoadingState label="Đang tải Flow…" />
      </section>
    );
  }

  if (loadState === "error") {
    return (
      <section
        className="flow-workspace"
        aria-label="Không gian Flow"
        data-loading="false"
        data-state="error"
      >
        <StatusNotice
          tone="error"
          action={(
            <button type="button" onClick={() => setLoadAttempt((attempt) => attempt + 1)}>
              Thử lại
            </button>
          )}
        >
          <strong>Không tải được Flow.</strong>
          {loadError && (
            <details>
              <summary>Chi tiết lỗi</summary>
              <code>{loadError}</code>
            </details>
          )}
        </StatusNotice>
      </section>
    );
  }

  return (
    <section
      className="flow-workspace"
      aria-label="Không gian Flow"
      data-loading="false"
      data-state={loadState}
    >
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
        onImport={() => {
          // A successful import replaces the open document outright, so it is the same discard as
          // New and Duplicate -- which both ask. This one did not, and on a never-saved flow the
          // old draft became unreachable through the UI.
          void confirmDiscard().then((ok) => {
            if (ok) setDialog("import");
          });
        }}
        onExport={() => {
          if (!saved || state.dirty) return;
          void flowExport(state.document.id, state.document.revision).then(
            (body) => downloadJson(state.document.name, body),
            (error) => setOperationError(describeError(error)),
          );
        }}
        onJson={() => setDialog("json")}
        onUndo={() => dispatch({ type: "undo" })}
        onRedo={() => dispatch({ type: "redo" })}
        onTogglePalette={() =>
          setPaletteOpenOverride((open) => !(open ?? !compactLayout))
        }
        onToggleInspector={() => setInspectorOpen((open) => !open)}
      />

      <div className="flow-notices" aria-live="polite">
        {loadState === "empty" && (
          <StatusNotice tone="info">
            <strong>Chưa có Flow đã lưu.</strong> Bản nháp mới đã sẵn sàng để chỉnh sửa.
          </StatusNotice>
        )}
        {operationError && (
          <div className="flow-operation-error" role="alert">
            <details>
              <summary>Không hoàn tất thao tác Flow.</summary>
              <code>{operationError}</code>
            </details>
            <button type="button" onClick={() => setOperationError(null)}>Bỏ qua</button>
          </div>
        )}
        {draftError && (
          <div className="flow-operation-error" role="alert">
            <span>Không lưu được bản nháp cục bộ. Hãy lưu một bản trước khi đóng.</span>
            <details>
              <summary>Chi tiết lỗi</summary>
              <code>{draftError}</code>
            </details>
            <button type="button" onClick={() => setDraftError(null)}>Bỏ qua</button>
          </div>
        )}
        {state.notice && (
          <div className="flow-operation-error" role="status">
            Đã lưu bản {state.notice.savedRevision}; bản nháp mới hơn vẫn đang mở.
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
          onInsertNode={(edgeId, node, sourcePort) =>
            dispatch({ type: "insertNode", edgeId, node, sourcePort })
          }
          onAppendNode={(node) => dispatch({ type: "appendNode", node })}
          onDeleteSelection={(nodeIds, edgeIds) =>
            dispatch({ type: "deleteSelection", nodeIds, edgeIds })
          }
          onViewport={(viewport) => dispatch({ type: "setViewport", viewport })}
        />
        <div className="flow-inspector-shell" data-open={String(inspectorOpen)}>
          <FlowInspector
            node={selectedNode}
            definition={selectedDefinition}
            issues={state.validation}
            coordinateDeviceUdid={coordinateUdid}
            launchBundleId={launchBundleId}
            onConfigChange={(config, postcondition) => {
              // One dispatch, so one history entry: the evidence the inspector keeps in step with
              // this config rides along instead of landing as a second mutation.
              if (selectedNode) dispatch({
                type: "updateNodeConfig",
                nodeId: selectedNode.id,
                config,
                postcondition,
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
          pending={state.validationRequest !== null}
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
                  {runStateLabel(run.state)}
                </option>
              ))}
            </select>
          </label>
        </div>
        {activeRun ? (
          <FlowRunMonitor
            run={activeRun}
            devices={devices}
            deviceLabel={deviceLabel}
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
          <section
            className="flow-dialog flow-artifact-dialog"
            role="dialog"
            aria-modal="true"
            aria-label="Tệp kết quả"
          >
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
