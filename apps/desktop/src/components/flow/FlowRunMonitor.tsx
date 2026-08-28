import { Ban, RotateCcw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { flowGetRun, listenRiviuEvents } from "../../api";
import { describeError } from "../../describeError";
import type {
  FlowAggregateState,
  FlowDeviceRunRecord,
  FlowNodeAttemptRecord,
  FlowRunDetail,
  JsonValue,
} from "../../types";

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
  // A refresh that keeps failing used to be invisible: `refresh` had a `finally` and no `catch`, so
  // every 750 ms rejection went to the global unhandled-rejection handler while the table sat on
  // the last projection it managed to read -- still saying "Running", with nothing to say the
  // number under it had stopped moving.
  const [stallReason, setStallReason] = useState<string | null>(null);
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
        setStallReason(null);
        setDetail((current) =>
          next.run.eventRevision > current.run.eventRevision || next.run.state !== current.run.state
            ? next
            : current,
        );
      } catch (reason) {
        if (!disposed) setStallReason(describeError(reason));
      } finally {
        refreshing = false;
      }
    };

    void listenRiviuEvents((event) => {
      if (event.type !== "flowRunUpdated") return;
      if (event.runId === detail.run.id) void refresh(event.revision);
    }).then(
      (unlisten) => {
        if (disposed) unlisten();
        else stop = unlisten;
      },
      (reason: unknown) => {
        // Without this the projection would fall back to the 750 ms poll with no sign that live
        // updates were never wired up at all.
        if (!disposed) setStallReason(describeError(reason));
      },
    );

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
          {stallReason && (
            <span role="alert" className="flow-monitor-stalled">
              Không đọc được tiến trình: {stallReason}
            </span>
          )}
          {detail.run.error && (
            <span role="alert" className="flow-monitor-error" title={detail.run.error.message}>
              {detail.run.error.code}: {detail.run.error.message}
            </span>
          )}
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
                  {/* The code alone is not actionable: the backend maps several distinct WDA and
                      device failures onto `DeviceControl` and keeps what separates them in the
                      message. Showing only the code hid the difference between a timeout, a dead
                      session, and the wrong app in the foreground. */}
                  <td title={device.error?.message ?? ""}>
                    {device.error ? `${device.error.code}: ${device.error.message}` : ""}
                  </td>
                  <td />
                </tr>
              );
            }
            const artifact = artifacts.get(attempt.id);
            return (
              <tr key={attempt.id}>
                <td>{device.udid}</td>
                <td>
                  {displayFlowState(attempt.actionKind)}
                  {/* Which branch an If Vision picked is the whole question when a vision flow
                      does the wrong thing, and it was on the wire all along -- TypeScript just
                      had no field for it. */}
                  {attempt.chosenPort && (
                    <span className="flow-monitor-branch"> → {attempt.chosenPort}</span>
                  )}
                </td>
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
                <td title={attempt.error?.message ?? ""}>
                  {attempt.error ? `${attempt.error.code}: ${attempt.error.message}` : ""}
                </td>
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
