import { useEffect, useState } from "react";
import type { DeviceInfo, JobRecord } from "../types";
import { cancelJob, exampleScript, runScript } from "../api";
import { SelectionStrip, flash, targetsOf } from "./SelectionStrip";

interface Props {
  jobs: JobRecord[];
  devices: DeviceInfo[];
  selectedUdids: string[];
  onSelectUdids: (udids: string[]) => void;
  onRefresh: () => void;
  initialScript?: string | null;
}

export function JobsPanel({
  jobs,
  devices,
  selectedUdids,
  onSelectUdids,
  onRefresh,
  initialScript,
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
      <header className="panel-header">
        <h2>Jobs</h2>
        <button type="button" className="ghost" onClick={onRefresh}>
          Refresh
        </button>
      </header>
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
                  flash(`Job queued · ${targets.length} máy`);
                } catch (e) {
                  setError(String(e));
                } finally {
                  setBusy(false);
                }
              }}
            >
              Run ({targets.length})
            </button>
            <button
              type="button"
              className="ghost"
              onClick={async () => setScriptJson(await exampleScript())}
            >
              Load example
            </button>
          </div>
        </section>
        <section>
          <h3>Lịch sử</h3>
          <div className="job-list">
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
                    Cancel
                  </button>
                ) : null}
              </article>
            ))}
            {!jobs.length && <p className="hint">No jobs yet.</p>}
          </div>
        </section>
      </div>
    </div>
  );
}
