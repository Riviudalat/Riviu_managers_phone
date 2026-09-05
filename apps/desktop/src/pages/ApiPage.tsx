import { useEffect, useMemo, useRef, useState } from "react";
import { RefreshCw, Search } from "lucide-react";

import { apiDocs } from "../api";
import { EmptyState, LoadingState, StatusNotice } from "../components/States";
import { StatusChip, SummaryRail } from "../components/WorkspacePrimitives";
import { describeError } from "../describeError";
import { ApiRuntimeStatus } from "../components/settings/ApiRuntimeStatus";

interface ApiDocGroup {
  title: string;
  commands: string[];
}

const GROUP_LABELS: Record<string, string> = {
  Devices: "Thiết bị",
  "Farm data": "Dữ liệu và tài nguyên",
  Operations: "Tác vụ",
  Sidecar: "Công cụ hỗ trợ",
};

function parseApiDocs(source: string): ApiDocGroup[] {
  const groups: ApiDocGroup[] = [];
  let current: ApiDocGroup | null = null;
  for (const rawLine of source.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line.startsWith("## ")) {
      const title = line.slice(3).trim();
      current = { title: GROUP_LABELS[title] ?? title, commands: [] };
      groups.push(current);
      continue;
    }
    if (!line.startsWith("- ")) continue;
    if (!current) {
      current = { title: "Khác", commands: [] };
      groups.push(current);
    }
    current.commands.push(...line.slice(2).split(" / ").map((command) => command.trim()).filter(Boolean));
  }
  return groups.filter((group) => group.commands.length > 0);
}

function hasStructuredReference(source: string): boolean {
  return /^##\s+/m.test(source);
}

/** The Local API reference returned by the running desktop backend. */
export function ApiPage() {
  const [docs, setDocs] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const loadTicket = useRef(0);

  const load = async () => {
    const ticket = ++loadTicket.current;
    setLoading(true);
    setError(null);
    try {
      const next = await apiDocs();
      if (ticket === loadTicket.current) setDocs(next);
    } catch (cause) {
      if (ticket === loadTicket.current) setError(describeError(cause));
    } finally {
      if (ticket === loadTicket.current) setLoading(false);
    }
  };

  useEffect(() => {
    void load();
    return () => {
      loadTicket.current += 1;
    };
  }, []);

  const groups = useMemo(() => parseApiDocs(docs ?? ""), [docs]);
  const structured = hasStructuredReference(docs ?? "");
  const commandCount = groups.reduce((total, group) => total + group.commands.length, 0);
  const isEmpty = docs !== null && docs.trim() === "";
  const filteredGroups = groups.map((group) => ({ ...group,
    commands: group.commands.filter((command) => `${group.title} ${command}`.toLocaleLowerCase("vi").includes(query.trim().toLocaleLowerCase("vi"))),
  })).filter((group) => group.commands.length > 0);

  return (
    <div className="admin-workspace api-workspace">
      {loading && !docs && <LoadingState label="Đang tải tài liệu API…" />}
      {!loading && error && (
        <StatusNotice
          tone="error"
          action={<button type="button" className="ghost" onClick={() => void load()}>Thử lại</button>}
        >
          Không tải được tài liệu API: {error}
        </StatusNotice>
      )}
      {!loading && !error && isEmpty && (
        <EmptyState
          title="Chưa có tài liệu API"
          hint="Runtime cục bộ chưa trả về nội dung tài liệu."
          action={<button type="button" className="ghost" onClick={() => void load()}>Tải lại</button>}
        />
      )}
      {!error && docs !== null && !isEmpty && (
        <div className="admin-split">
          <main className="admin-main">
            <div className="admin-toolbar">
              <div className="admin-toolbar-copy">
                <strong>Danh mục lệnh</strong>
              </div>
              <div className="admin-toolbar-actions">
                {structured && <label className="search-field"><Search size={15} aria-hidden="true" /><span className="visually-hidden">Tìm lệnh API</span>
                  <input type="search" placeholder="Tìm lệnh…" value={query} onChange={(event) => setQuery(event.target.value)} />
                </label>}
                <button type="button" className="icon-btn" title="Làm mới tài liệu API" aria-label="Làm mới tài liệu API" onClick={() => void load()} disabled={loading}>
                  <RefreshCw size={15} aria-hidden="true" />
                </button>
              </div>
            </div>
            <div className="api-reference-list">
              {structured && filteredGroups.map((group) => (
                <details key={group.title} className="api-reference-group" open={query.trim() ? true : undefined}>
                  <summary>
                    {group.title}
                    <StatusChip>{group.commands.length} lệnh</StatusChip>
                  </summary>
                  <ul className="api-command-list">
                    {group.commands.map((command) => <li key={command}><code>{command}</code></li>)}
                  </ul>
                </details>
              ))}
              {structured && !filteredGroups.length && <EmptyState compact title="Không tìm thấy lệnh" />}
              {!structured && (
                <details className="api-reference-group">
                  <summary>Tài liệu runtime <StatusChip>{groups.length ? `${commandCount} lệnh` : "Văn bản"}</StatusChip></summary>
                  <pre className="admin-raw">{docs}</pre>
                </details>
              )}
            </div>
          </main>
          <SummaryRail title="Trạng thái API">
            <ApiRuntimeStatus />
            {structured && <dl className="admin-metric-grid">
              <div className="admin-metric"><dt>Nhóm</dt><dd>{groups.length}</dd></div>
              <div className="admin-metric"><dt>Lệnh</dt><dd>{commandCount}</dd></div>
            </dl>}
          </SummaryRail>
        </div>
      )}
    </div>
  );
}
