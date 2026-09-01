import { useCallback, useEffect, useMemo, useState } from "react";

import { listOpLogs } from "../api";
import { describeError } from "../describeError";
import type { OpLog } from "../types";
import { EmptyState, LoadingState, StatusNotice } from "./States";

/** How many rows to ask for. `analytics_summary` fetches twenty; this is the whole point. */
const LIMIT = 200;

/**
 * Everything the app has recorded itself doing.
 *
 * **`op_logs` had fifteen writers and no reader.** `log_op` is called from the nurture engine,
 * the publish path, the farm commands, the agent install and eight places in `state.rs`, so a
 * working farm writes to this table constantly. `analytics_summary` did select the last twenty
 * rows into `recentLogs` — and `DataPage` rendered eight stat tiles and threw that field away.
 * So the one durable record of what the app did to which phone was, in the app itself,
 * invisible.
 *
 * That is the same failure the repo already wrote down about `nurture_list_comment_attempts`:
 * *"a number nobody reads cannot be checked."* Here it was not even a number — it was the
 * answer to "what happened before it broke".
 *
 * The filter is a substring over action and detail rather than a dropdown of actions: the
 * actions are free-text strings written at fifteen call sites (`proxy.save`, `publish.create`,
 * `agent.install`…), so a fixed list would be a list that goes stale. Typing `nurture` or a
 * udid is what an operator actually reaches for.
 */
export function OperationLog() {
  const [rows, setRows] = useState<OpLog[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [needle, setNeedle] = useState("");

  const load = useCallback(() => {
    setLoading(true);
    setError(null);
    void listOpLogs(LIMIT)
      .then((next) => {
        setRows(next);
        setError(null);
      })
      .catch((cause) => setError(describeError(cause)))
      .finally(() => setLoading(false));
  }, []);

  useEffect(load, [load]);

  const shown = useMemo(() => {
    if (!rows) return [];
    const term = needle.trim().toLowerCase();
    if (!term) return rows;
    return rows.filter(
      (row) =>
        row.action.toLowerCase().includes(term) || row.detail.toLowerCase().includes(term),
    );
  }, [rows, needle]);

  return (
    <section className="op-log">
      <header className="row">
        <h3>Việc app đã làm</h3>
        <input
          type="search"
          value={needle}
          onChange={(event) => setNeedle(event.target.value)}
          placeholder="Lọc theo hành động hoặc chi tiết…"
          aria-label="Lọc nhật ký thao tác"
        />
        <button type="button" className="ghost" onClick={load}>
          Làm mới
        </button>
      </header>

      {error && (
        <StatusNotice
          tone="error"
          action={
            <button type="button" onClick={load}>
              Thử lại nhật ký
            </button>
          }
        >
          Không đọc được nhật ký: {error}
        </StatusNotice>
      )}
      {!error && loading && !rows && <LoadingState label="Đang đọc nhật ký…" />}
      {!error && !loading && rows?.length === 0 && (
        <EmptyState
          compact
          title="Chưa có dòng nào"
          hint="Nhật ký sẽ xuất hiện sau khi app thực hiện một thao tác trên máy."
        />
      )}
      {!error && rows !== null && rows.length > 0 && shown.length === 0 && (
        <p className="hint">Không dòng nào khớp “{needle.trim()}”.</p>
      )}

      {shown.length > 0 && (
        <ul className="op-log-list">
          {shown.map((row) => (
            <li key={row.id}>
              <code className="op-log-action">{row.action}</code>
              <span className="op-log-detail">{row.detail || "—"}</span>
              <time className="op-log-when" dateTime={row.createdAt}>
                {row.createdAt.replace("T", " ").slice(0, 19)}
              </time>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
