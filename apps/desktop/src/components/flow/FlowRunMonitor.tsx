import { Ban, RotateCcw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { flowGetRun, listenRiviuEvents } from "../../api";
import type {
  FlowAggregateState,
  FlowDeviceRunRecord,
  FlowNodeAttemptRecord,
  FlowRunDetail,
  JsonValue,
} from "../../types";

function flowRunEvent(value: unknown): { runId: string; revision: number } | null {
  if (typeof value !== "object" || value === null) return null;
  const event = value as Record<string, unknown>;
  return event.type === "flowRunUpdated" &&
    typeof event.runId === "string" &&
    typeof event.revision === "number"
    ? { runId: event.runId, revision: event.revision }
    : null;
}

function attemptDurationMs(attempt: FlowNodeAttemptRecord): number {
  if (!attempt.startedAt || !attempt.finishedAt) return 0;
  const start = Date.parse(attempt.startedAt);
  const finish = Date.parse(attempt.finishedAt);
  return Number.isFinite(start) && Number.isFinite(finish) && finish >= start
    ? finish - start
    : 0;
}

function formatEvidence(value: JsonValue | null): string {
  if (value === null) return "";
  const encoded = JSON.stringify(value);
  return encoded.length <= 160 ? encoded : `${encoded.slice(0, 157)}...`;
}

function displayFlowState(value: string): string {
  if (value.length === 0) return value;
  return value
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .replace(/^./, (letter) => letter.toUpperCase());
}

function terminal(state: FlowAggregateState): boolean {
  return ["succeeded", "partial", "failed", "cancelled"].includes(state);
}

export function FlowRunMonitor({
  run,
  onCancel,
  onRetry,
  onOpenArtifact = () => undefined,
}: {
  run: FlowRunDetail;
  onCancel: (runId: string) => void;
  onRetry: (attemptId: string) => void;
  onOpenArtifact?: (artifactId: string) => void;
}) {
  const [detail, setDetail] = useState(run);
  useEffect(() => setDetail(run), [run]);

  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    let refreshing = false;

    const refresh = async (minimumRevision = 0) => {
      if (refreshing) return;
      refreshing = true;
      try {
        const next = await flowGetRun(detail.run.id);
        if (disposed || !next || next.run.eventRevision < minimumRevision) return;
        setDetail((current) =>
          next.run.eventRevision > current.run.eventRevision || next.run.state !== current.run.state
            ? next
            : current,
        );
      } finally {
        refreshing = false;
      }
    };

    void listenRiviuEvents((payload) => {
      const event = flowRunEvent(payload);
      if (event?.runId === detail.run.id) void refresh(event.revision);
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });

    // Runtime events are projection invalidations. Node commits that precede a
    // run projection event are covered by this bounded poll while nonterminal.
    const timer = terminal(detail.run.state)
      ? undefined
      : window.setInterval(() => void refresh(), 750);

    return () => {
      disposed = true;
      stop?.();
      if (timer !== undefined) window.clearInterval(timer);
    };
  }, [detail.run.id, detail.run.state]);

  const artifacts = useMemo(
    () => new Map(detail.artifacts.map((item) => [item.attemptId, item])),
    [detail.artifacts],
  );
  const rows = detail.deviceRuns.flatMap<{
    device: FlowDeviceRunRecord;
    attempt: FlowNodeAttemptRecord | null;
  }>((device) => {
    const attempts = detail.attempts.filter((attempt) => attempt.deviceRunId === device.id);
    return attempts.length > 0
      ? attempts.map((attempt) => ({ device, attempt }))
      : [{ device, attempt: null }];
  });

  return (
    <section className="flow-monitor" data-testid="flow-monitor">
      <header>
        <div>
          <strong>{displayFlowState(detail.run.state)}</strong>
          <span>{detail.run.selection.targetUdids.length} devices</span>
        </div>
        <button
          type="button"
          title="Hủy lượt chạy"
          onClick={() => onCancel(detail.run.id)}
          disabled={terminal(detail.run.state)}
        >
          <Ban size={14} />
          Hủy
        </button>
      </header>
      <table>
        <thead>
          <tr>
            <th>Thiết bị</th>
            <th>Node</th>
            <th>Lượt thử</th>
            <th>Trạng thái</th>
            <th>Thời lượng</th>
            <th>Bằng chứng</th>
            <th>Tệp kết quả</th>
            <th>Lỗi</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {rows.map(({ device, attempt }) => {
            if (!attempt) {
              return (
                <tr key={device.id}>
                  <td>{device.udid}</td>
                  <td />
                  <td />
                  <td>{displayFlowState(device.state)}</td>
                  <td />
                  <td />
                  <td />
                  <td>{device.error?.code ?? ""}</td>
                  <td />
                </tr>
              );
            }
            const artifact = artifacts.get(attempt.id);
            return (
              <tr key={attempt.id}>
                <td>{device.udid}</td>
                <td>{displayFlowState(attempt.actionKind)}</td>
                <td>{attempt.attemptNo}</td>
                <td>{displayFlowState(attempt.state)}</td>
                <td>{attemptDurationMs(attempt)} ms</td>
                <td title={formatEvidence(attempt.evidenceResult)}>
                  {formatEvidence(attempt.evidenceResult)}
                </td>
                <td>
                  {artifact ? (
                    <button type="button" onClick={() => onOpenArtifact(artifact.id)}>
                      {artifact.label}
                    </button>
                  ) : null}
                </td>
                <td>{attempt.error?.code ?? ""}</td>
                <td>
                  {attempt.retryAllowed && (
                    <button
                      type="button"
                      onClick={() => onRetry(attempt.id)}
                      title={`Retry ${displayFlowState(attempt.actionKind)}`}
                    >
                      <RotateCcw size={14} />
                      Retry {displayFlowState(attempt.actionKind)}
                    </button>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </section>
  );
}
