import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowUpRight, ChevronRight, RefreshCw, Search } from "lucide-react";

import { apiDocs } from "../api";
import { EmptyState, LoadingState, StatusNotice } from "../components/States";
import { StatusChip, SummaryRail } from "../components/WorkspacePrimitives";
import { describeError } from "../describeError";
import { ApiRuntimeStatus } from "../components/settings/ApiRuntimeStatus";

interface ApiDocGroup {
  title: string;
  commands: string[];
}

const HTTP_GROUPS = [
  { title: "HTTP · My Apps", routes: [
    ["GET", "/v1/apps", "Danh sách ứng dụng và phiên bản đã lưu."],
    ["GET", "/v1/apps/{id}?revision=1", "Đọc graph và cấu hình của đúng phiên bản ứng dụng."],
    ["POST", "/v1/apps/{id}/runs", "Chạy bằng revision và target đã chọn; dùng chung bộ chạy với desktop."],
  ] },
  {
    title: "HTTP · Flow và tác vụ",
    routes: [
      ["GET", "/v1/flows/catalog", "Danh mục action và khả năng hỗ trợ hiện tại."],
      ["GET", "/v1/flows?includeArchived=false", "Danh sách Flow đã lưu; có thể chọn includeArchived=true."],
      ["GET", "/v1/flows/{id}?revision=3", "Đọc đúng revision bất biến. Bỏ revision để đọc bản mới nhất."],
      ["POST", "/v1/flows/{id}/runs", "Chạy revision đã chọn trên các máy được chỉ định trong selection."],
      ["GET", "/v1/flow-runs?limit=100", "Lịch sử lượt chạy; limit từ 1 đến 200."],
      ["GET", "/v1/flow-runs/{id}", "Trạng thái từng máy, lần thực hiện và bằng chứng của lượt chạy."],
      ["POST", "/v1/flow-runs/{id}/cancel", "Yêu cầu dừng; đọc lại lượt chạy để biết khi nào đã kết thúc."],
      ["GET", "/v1/jobs", "100 tác vụ mới nhất trong hàng đợi script."],
    ],
  },
  {
    title: "HTTP · Thiết bị và nhóm",
    routes: [
      ["GET", "/v1/devices", "Danh sách thiết bị hiện tại."],
      ["GET", "/v1/groups", "Các nhóm máy đã lưu."],
      ["POST", "/v1/tap", "Chạm: udid, x, y."],
      ["POST", "/v1/swipe", "Vuốt: udid, x1, y1, x2, y2; durationMs mặc định 300."],
      ["POST", "/v1/key", "Phím cứng: udid, key; ví dụ volumeUp."],
      ["POST", "/v1/home", "Về màn hình chính: udid."],
      ["POST", "/v1/text", "Nhập văn bản: udid, text."],
      ["POST", "/v1/lock", "Khóa/mở khóa: udid, locked; mặc định true."],
    ],
  },
] as const;

const FLOW_EXAMPLE = `$base = "http://127.0.0.1:22222"
$headers = @{ Authorization = "Bearer TOKEN" }

# Lấy ID Flow và UDID từ dữ liệu thực tế.
Invoke-RestMethod "$base/v1/flows" -Headers $headers
Invoke-RestMethod "$base/v1/devices" -Headers $headers

$body = @{
  revision = 3
  selection = @{ mode = "selected"; udids = @("UDID_A", "UDID_B") }
} | ConvertTo-Json -Depth 4

$request = @{
  Uri = "$base/v1/flows/FLOW_UUID/runs"
  Method = "Post"; Headers = $headers
  ContentType = "application/json"; Body = $body
}
$run = Invoke-RestMethod @request
$runId = $run.result.id

# Theo dõi kết quả của cùng lượt chạy.
Invoke-RestMethod "$base/v1/flow-runs/$runId" -Headers $headers

# Chỉ gửi khi muốn dừng; tiếp tục đọc trạng thái đến khi kết thúc.
Invoke-RestMethod "$base/v1/flow-runs/$runId/cancel" -Method Post -Headers $headers`;

function HttpQuickStart() {
  return <details className="api-reference-group">
    <summary><ChevronRight className="api-reference-chevron" size={16} aria-hidden="true" />Ví dụ chạy Flow bằng PowerShell</summary>
    <p>Thay cổng bằng địa chỉ đang lắng nghe và lấy TOKEN trong Cấu hình kết nối. Mỗi request cần <code>Authorization: Bearer TOKEN</code>.</p>
    <p>Chọn revision đã đọc và UDID chính xác. <code>selection</code> là bắt buộc: <code>one</code> chọn một máy, <code>selected</code> chọn danh sách, <code>allEligible</code> chọn mọi máy đủ điều kiện.</p>
    <pre className="admin-raw" aria-label="Ví dụ PowerShell chạy và dừng Flow">{FLOW_EXAMPLE}</pre>
    <p>Phản hồi <code>{'{"ok":true,"result":…}'}</code> khi tạo lượt chạy nghĩa là đã nhận vào hàng đợi. Theo dõi chi tiết để đọc kết quả thực tế; <code>cancellationRequested</code> chỉ xác nhận yêu cầu dừng.</p>
    <p>Khi lỗi, đọc <code>code</code> và <code>details</code>: 400 sai dữ liệu, 401 sai token, 404 không tìm thấy, 409 máy bận hoặc xung đột trạng thái, 503 ứng dụng đang đóng.</p>
    <p>Nếu mất phản hồi POST, tra lịch sử lượt chạy trước khi gửi lại để tránh tạo hai lượt. Body JSON tối đa 64 KiB tính cả header; gửi Content-Length.</p>
  </details>;
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
export function ApiPage({ onOpenSettings }: { onOpenSettings?: () => void } = {}) {
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
  const normalizedQuery = query.trim().toLocaleLowerCase("vi");
  const httpGroups = HTTP_GROUPS.map((group) => ({ ...group,
    routes: group.routes.filter((route) => `${group.title} ${route.join(" ")}`.toLocaleLowerCase("vi").includes(normalizedQuery)),
  })).filter((group) => group.routes.length > 0);

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
      {docs !== null && !isEmpty && (
        <div className="admin-split" aria-busy={loading}>
          <main className="admin-main">
            <div className="admin-toolbar">
              <div className="admin-toolbar-copy">
                <strong>HTTP cục bộ và lệnh runtime</strong>
                {structured && <span>{HTTP_GROUPS.reduce((count,group)=>count+group.routes.length,0)} endpoint HTTP · {commandCount} lệnh runtime</span>}
              </div>
              <div className="admin-toolbar-actions">
                {structured && <label className="search-field"><Search size={15} aria-hidden="true" /><span className="visually-hidden">Tìm lệnh API</span>
                  <input type="search" placeholder="Tìm lệnh hoặc đường dẫn…" value={query} onChange={(event) => setQuery(event.target.value)} />
                </label>}
                <button type="button" className="icon-btn" title="Làm mới tài liệu API" aria-label="Làm mới tài liệu API" onClick={() => void load()} disabled={loading}>
                  <RefreshCw size={15} aria-hidden="true" />
                </button>
              </div>
            </div>
            <div className="api-reference-list">
              {httpGroups.map((group) => <details key={group.title} className="api-reference-group" open={normalizedQuery ? true : undefined}>
                <summary><ChevronRight className="api-reference-chevron" size={16} aria-hidden="true" />{group.title}<StatusChip>{group.routes.length} endpoint</StatusChip></summary>
                <ul className="api-command-list">
                  {group.routes.map(([method, path, description]) => <li key={`${method} ${path}`}>
                    <code>{method} {path}</code><p>{description}</p>
                  </li>)}
                </ul>
              </details>)}
              {!normalizedQuery && <HttpQuickStart />}
              {structured && filteredGroups.length > 0 && <p>Tham chiếu runtime bên dưới dùng qua Tauri invoke trong ứng dụng.</p>}
              {structured && filteredGroups.map((group) => (
                <details key={group.title} className="api-reference-group" open={query.trim() ? true : undefined}>
                  <summary>
                    <ChevronRight className="api-reference-chevron" size={16} aria-hidden="true" />
                    {group.title}
                    <StatusChip>{group.commands.length} lệnh</StatusChip>
                  </summary>
                  <ul className="api-command-list">
                    {group.commands.map((command) => <li key={command}><code>{command}</code></li>)}
                  </ul>
                </details>
              ))}
              {structured && !filteredGroups.length && !httpGroups.length && <EmptyState compact title="Không tìm thấy lệnh" />}
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
            {onOpenSettings && <button type="button" className="ghost api-settings-link" onClick={onOpenSettings}>
              Cấu hình kết nối <ArrowUpRight size={16} aria-hidden="true" />
            </button>}
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
