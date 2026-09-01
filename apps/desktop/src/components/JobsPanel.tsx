import { useEffect, useState } from "react";
import type { DeviceInfo, JobRecord } from "../types";
import { cancelJob, exampleScript, runScript } from "../api";
import { SelectionStrip } from "./SelectionStrip";
import { flash } from "../farmToast";
import { targetsOf } from "../selectionTargets";
import { describeError } from "../describeError";
import { EmptyState, LoadingState, StatusNotice } from "./States";

interface Props {
  jobs: JobRecord[];
  devices: DeviceInfo[];
  selectedUdids: string[];
  onSelectUdids: (udids: string[]) => void;
  onRefresh: () => void | Promise<void>;
  initialScript?: string | null;
  loading?: boolean;
  loadError?: string | null;
}

export function JobsPanel({
  jobs,
  devices,
  selectedUdids,
  onSelectUdids,
  onRefresh,
  initialScript,
  loading = false,
  loadError = null,
}: Props) {
  const [scriptJson, setScriptJson] = useState(initialScript ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const targets = targetsOf(selectedUdids, devices);

  useEffect(() => {
    if (initialScript) setScriptJson(initialScript);
  }, [initialScript]);

  useEffect(() => {
    if (!scriptJson.trim()) {
      exampleScript()
        .then(setScriptJson)
        .catch(() => undefined);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="panel">
      <div className="panel-header" style={{ justifyContent: "flex-end" }}>
        <button type="button" className="ghost" onClick={onRefresh}>
          Làm mới
        </button>
      </div>
      <SelectionStrip
        devices={devices}
        selected={selectedUdids}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
      />
      <div className="panel-grid">
        <section>
          <h3>Chạy kịch bản</h3>
          <textarea
            rows={14}
            value={scriptJson}
            onChange={(e) => setScriptJson(e.target.value)}
            placeholder="Script JSON…"
          />
          {error && <p className="error">{error}</p>}
          <div className="row">
            <button
              type="button"
              className="primary"
              disabled={busy || !targets.length || !scriptJson.trim()}
              onClick={async () => {
                setBusy(true);
                setError(null);
                try {
                  await runScript(scriptJson, targets);
                  onRefresh();
                  flash(`Đã xếp tác vụ cho ${targets.length} máy`);
                } catch (e) {
                  setError(describeError(e));
                } finally {
                  setBusy(false);
                }
              }}
            >
              Chạy ({targets.length})
            </button>
            <button
              type="button"
              className="ghost"
              onClick={async () => setScriptJson(await exampleScript())}
            >
              Tải mẫu
            </button>
          </div>
        </section>
        <section>
          <h3>Lịch sử</h3>
          <div className="job-list">
            {loading && !jobs.length && <LoadingState label="Đang tải lịch sử tác vụ…" />}
            {loadError && (
              <StatusNotice
                tone="error"
                action={
                  <button type="button" onClick={() => void onRefresh()}>
                    Thử lại lịch sử
                  </button>
                }
              >
                Không đọc được lịch sử: {loadError}
              </StatusNotice>
            )}
            {jobs.map((job) => (
              <article key={job.id} className="job-card">
                <div>
                  <strong>{job.scriptName}</strong>
                  <span className={`pill ${job.status}`}>{job.status}</span>
                </div>
                <p className="hint">
                  {job.udids.join(", ")} · {new Date(job.createdAt).toLocaleString()}
                </p>
                <ol>
                  {job.steps.map((step) => (
                    <li key={step.index}>
                      {step.action} — {step.status}
                      {step.error ? ` (${step.error})` : ""}
                    </li>
                  ))}
                </ol>
                {job.status === "running" || job.status === "queued" ? (
                  <button
                    type="button"
                    className="ghost"
                    onClick={async () => {
                      await cancelJob(job.id);
                      onRefresh();
                    }}
                  >
                    Huỷ
                  </button>
                ) : null}
              </article>
            ))}
            {!loading && !loadError && !jobs.length && (
              <EmptyState
                compact
                title="Chưa có tác vụ"
                hint="Chọn máy và chạy một kịch bản để tạo tác vụ đầu tiên."
              />
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
