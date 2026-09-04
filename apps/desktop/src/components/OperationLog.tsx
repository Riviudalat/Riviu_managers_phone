import { useCallback, useEffect, useMemo, useState } from "react";
import { RefreshCw, Search } from "lucide-react";

import { listOpLogs } from "../api";
import { describeError } from "../describeError";
import type { OpLog } from "../types";
import { EmptyState, LoadingState, StatusNotice } from "./States";

const LIMIT = 200;

const ACTION_LABELS: Array<[string, string]> = [
  ["nurture", "Nuôi TikTok"],
  ["interaction", "Tương tác"],
  ["publish", "Đăng bài"],
  ["flow", "Flow thiết bị"],
  ["orchestration", "Điều phối"],
  ["agent", "Riviu Agent"],
  ["material", "Kho nội dung"],
  ["app", "Trung tâm ứng dụng"],
  ["proxy", "Kết nối thiết bị"],
  ["deployment", "Khởi động ứng dụng"],
];

function operationLabel(action: string): string {
  const normalized = action.trim().toLowerCase();
  return ACTION_LABELS.find(([prefix]) => normalized === prefix || normalized.startsWith(`${prefix}.`))?.[1]
    ?? "Thao tác hệ thống";
}

/** Durable application operations, with raw codes kept in the row's detail disclosure. */
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
        row.action.toLowerCase().includes(term)
        || row.detail.toLowerCase().includes(term)
        || operationLabel(row.action).toLowerCase().includes(term),
    );
  }, [rows, needle]);

  return (
    <section className="op-log" aria-label="Nhật ký thao tác">
      <header className="admin-toolbar">
        <label className="search-field">
          <Search size={15} aria-hidden="true" />
          <span className="visually-hidden">Lọc nhật ký thao tác</span>
          <input
            type="search"
            value={needle}
            onChange={(event) => setNeedle(event.target.value)}
            placeholder="Tìm theo thao tác hoặc chi tiết…"
            aria-label="Lọc nhật ký thao tác"
          />
        </label>
        <button type="button" className="ghost" onClick={load} disabled={loading}>
          <RefreshCw size={15} aria-hidden="true" />
          Làm mới
        </button>
      </header>

      {error && (
        <StatusNotice
          tone="error"
          action={<button type="button" className="ghost" onClick={load}>Thử lại nhật ký</button>}
        >
          Không đọc được nhật ký: {error}
        </StatusNotice>
      )}
      {!error && loading && !rows && <LoadingState label="Đang đọc nhật ký…" />}
      {!error && !loading && rows?.length === 0 && (
        <EmptyState
          compact
          title="Chưa có thao tác nào"
          hint="Nhật ký sẽ xuất hiện sau khi ứng dụng thực hiện công việc trên thiết bị."
        />
      )}
      {!error && rows !== null && rows.length > 0 && shown.length === 0 && (
        <EmptyState compact title="Không tìm thấy kết quả" hint={`Không có thao tác nào khớp “${needle.trim()}”.`} />
      )}

      {shown.length > 0 && (
        <ul className="op-log-list">
          {shown.map((row) => (
            <li key={row.id}>
              <strong className="op-log-label">
                <span>{operationLabel(row.action)}</span>
                <code className="visually-hidden">{row.action}</code>
              </strong>
              <time className="op-log-when" dateTime={row.createdAt}>
                {row.createdAt.replace("T", " ").slice(0, 19)}
              </time>
              <details className="admin-detail op-log-detail">
                <summary>Chi tiết</summary>
                <p>{row.detail || "Không có chi tiết bổ sung."}</p>
              </details>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
