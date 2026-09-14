import { useEffect, useState } from "react";
import { Play, Square, X } from "lucide-react";
import { appWorkflowRun, type AppWorkflowV1 } from "../appWorkflow";
import { orchestrationCancelRun, orchestrationGetRun } from "../api";
import type { DeviceInfo, OrchestrationRunDetail } from "../types";
import { describeError } from "../describeError";
import { requestConfirm } from "../confirmStore";
import { OperationSourceDetail } from "./OperationSourceDetail";

export function AppWorkflowRunPanel({
  document,
  devices,
  onClose,
}: {
  document: AppWorkflowV1;
  devices: DeviceInfo[];
  onClose: () => void;
}) {
  const [selected, setSelected] = useState<string[]>([]),
    [run, setRun] = useState<OrchestrationRunDetail | null>(null),
    [busy, setBusy] = useState(false),
    [error, setError] = useState<string | null>(null);
  const running = run && ["queued", "running"].includes(run.run.state);
  const runId = run?.run.id;
  useEffect(() => {
    if (!runId || !running) return;
    let alive = true,
      reading = false;
    const refresh = async () => {
      if (reading) return;
      reading = true;
      try {
        const next = await orchestrationGetRun(runId);
        if (alive && next) setRun(next);
      } catch (cause) {
        if (alive) setError(describeError(cause));
      } finally {
        reading = false;
      }
    };
    const timer = setInterval(() => void refresh(), 2000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [runId, running]);
  const launch = async () => {
    if (!selected.length || busy) return;
    if (
      !(await requestConfirm({
        title: "Chạy ứng dụng?",
        message: `${document.name} · bản ${document.revision} trên ${selected.length} máy đã chọn.`,
        confirmLabel: "Chạy",
      }))
    )
      return;
    setBusy(true);
    setError(null);
    try {
      setRun(
        await appWorkflowRun(document.id, document.revision, {
          type: "explicit",
          udids: selected,
        }),
      );
    } catch (cause) {
      setError(describeError(cause));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="app-run-panel" aria-label="Chạy ứng dụng">
      <header>
        <strong>
          {document.name} · bản {document.revision}
        </strong>
        <button type="button" aria-label="Đóng bảng chạy" onClick={onClose}>
          <X size={16} />
        </button>
      </header>
      {error && <p role="alert">{error}</p>}
      {!run && (
        <>
          <p>Chọn thiết bị thực hiện</p>
          <div className="app-run-devices">
            {devices.map((device) => (
              <label key={device.udid}>
                <input
                  type="checkbox"
                  checked={selected.includes(device.udid)}
                  onChange={(e) =>
                    setSelected((current) =>
                      e.target.checked
                        ? [...current, device.udid]
                        : current.filter((id) => id !== device.udid),
                    )
                  }
                />
                {device.name}
                <small>{device.status}</small>
              </label>
            ))}
          </div>
          <button
            type="button"
            className="primary"
            disabled={!selected.length || busy}
            onClick={() => void launch()}
          >
            <Play size={15} />
            Chạy trên {selected.length} máy
          </button>
        </>
      )}
      {run && (
        <>
          <p role="status">
            {run.run.state} · {run.run.target.included.length} máy
          </p>
          <code>{run.run.id}</code>
          <OperationSourceDetail source={{kind:"orchestration",operationId:`orchestration:${run.run.id}`,sourceId:run.run.id}}/>
          {run.run.errorCode && <p>{run.run.errorCode}</p>}
          <table>
            <thead>
              <tr>
                <th>Ứng dụng</th>
                <th>Trạng thái</th>
                <th>Kết quả</th>
              </tr>
            </thead>
            <tbody>
              {run.attempts
                .filter((a) => a.childKind)
                .map((attempt) => (
                  <tr key={attempt.snapshot.attemptId}>
                    <td>{attempt.childKind}</td>
                    <td>{attempt.state}</td>
                    <td>{attempt.errorCode ?? attempt.branch ?? "Đang chờ"}</td>
                  </tr>
                ))}
            </tbody>
          </table>
          {running && (
            <button
              type="button"
              disabled={busy}
              onClick={async () => {
                setBusy(true);
                try {
                  await orchestrationCancelRun(run.run.id);
                  const next = await orchestrationGetRun(run.run.id);
                  if (next) setRun(next);
                } catch (cause) {
                  setError(describeError(cause));
                } finally {
                  setBusy(false);
                }
              }}
            >
              <Square size={15} />
              Dừng lượt chạy
            </button>
          )}
        </>
      )}
    </section>
  );
}
