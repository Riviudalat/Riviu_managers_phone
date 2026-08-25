import { useEffect, useState } from "react";
import { analyticsSummary } from "../api";
import type { AnalyticsSummary } from "../types";
import { describeError } from "../describeError";

/** Fleet analytics, read-only. */
export function DataPage() {
  const [data, setData] = useState<AnalyticsSummary | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const load = () =>
    analyticsSummary()
      .then((d) => {
        setData(d);
        setErr(null);
      })
      .catch((e) => setErr(describeError(e)));
  useEffect(() => {
    load();
  }, []);
  if (err) return <div className="panel error">{err}</div>;
  if (!data) return <div className="panel">Loading…</div>;
  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Dữ liệu</h2>
        <button type="button" className="ghost" onClick={load}>
          Làm mới
        </button>
      </header>
      <div className="stats-grid">
        {(
          [
            ["Devices", `${data.deviceReady}/${data.deviceTotal}`],
            ["Jobs ok", String(data.jobsSucceeded)],
            ["Jobs fail", String(data.jobsFailed)],
            ["Running", String(data.jobsRunning)],
            ["Scripts", String(data.scriptsTotal)],
            ["Materials", String(data.materialsTotal)],
            ["Apps", String(data.appsTotal)],
            ["Schedules", String(data.schedulesEnabled)],
          ] as const
        ).map(([k, v]) => (
          <article key={k} className="job-card">
            <div className="hint">{k}</div>
            <strong style={{ fontSize: "1.4rem" }}>{v}</strong>
          </article>
        ))}
      </div>
    </div>
  );
}
