import { useCallback, useEffect, useRef, useState } from "react";
import { Braces, RefreshCw, Search, Square, ChevronLeft, ChevronRight } from "lucide-react";

import {
  cancelJob,
  listenRiviuEvents,
  operationGetRun,
  operationQueryRuns,
  runScript,
} from "../api";
import { describeError } from "../describeError";
import { flash } from "../farmToast";
import { targetsOf } from "../selectionTargets";
import type {
  DeviceInfo,
  AppEvent,
  OperationRunDetail,
  OperationRunKind,
  OperationRunState,
  OperationRunSummary,
  PublishRetryScope,
  OperationRunPage,
} from "../types";
import { SelectionStrip } from "./SelectionStrip";
import { EmptyState, LoadingState, StatusNotice } from "./States";
import "../styles/operations.css";

interface Props {
  devices: DeviceInfo[];
  selectedUdids: string[];
  onSelectUdids: (udids: string[]) => void;
  initialScript?: string | null;
  deviceLabels: ReadonlyMap<string, string>;
}

const RUN_LABEL: Record<OperationRunState, string> = {
  queued: "Đang chờ",
  running: "Đang chạy",
  succeeded: "Hoàn tất",
  partial: "Một phần",
  failed: "Thất bại",
  uncertain: "Chưa chắc chắn",
  cancelled: "Đã dừng",
  skipped: "Đã bỏ qua",
};

const KIND_LABEL: Record<OperationRunKind, string> = {
  script: "JSON nâng cao",
  flow: "Flow thiết bị",
  orchestration: "Điều phối",
  nurture: "Nuôi TikTok",
  interaction: "Tương tác",
  publish: "Đăng bài",
  appInstall: "Cài ứng dụng",
  materialTransfer: "Chuyển nội dung",
};

const RETRY_SCOPE_LABEL: Record<PublishRetryScope, string> = {
  fullPipeline: "Chạy lại từ trước khi đăng",
  linkAndSheet: "Lấy lại liên kết và gửi báo cáo",
  sheetOnly: "Chỉ gửi lại báo cáo",
  none: "Không tự động chạy lại",
};

function isActive(run: OperationRunSummary): boolean {
  return run.state === "queued" || run.state === "running";
}

function formatTimestamp(value: string | null): string {
  if (!value) return "Chưa có thời gian";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "Thời gian không hợp lệ" : date.toLocaleString("vi-VN");
}

export function JobsPanel({
  devices,
  selectedUdids,
  onSelectUdids,
  initialScript,
  deviceLabels,
}: Props) {
  const [runs, setRuns] = useState<OperationRunSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [detail, setDetail] = useState<OperationRunDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [scriptJson, setScriptJson] = useState(initialScript ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<OperationRunState | "all">("all");
  const [kind, setKind] = useState<OperationRunKind | "all">("all");
  const [period, setPeriod] = useState("24");
  const [offset, setOffset] = useState(0);
  const [page, setPage] = useState<OperationRunPage | null>(null);
  const listTicket = useRef(0);
  const detailTicket = useRef(0);
  const targets = targetsOf(selectedUdids, devices);
  const deviceNames = deviceLabels;
  const selectedRunIdRef = useRef<string | null>(null);

  const reload = useCallback(async () => {
    const ticket = ++listTicket.current;
    setLoading(true);
    setLoadError(null);
    try {
      const nextPage = await operationQueryRuns({ limit: 50, offset,
        state: status === "all" ? undefined : status, kind: kind === "all" ? undefined : kind,
        search: query, since: period === "all" ? null : new Date(Date.now() - Number(period) * 3600000).toISOString() });
      const next = nextPage.runs;
      if (ticket !== listTicket.current) return;
      setPage(nextPage);
      setRuns(next);
      setSelectedRunId((current) =>
        current && next.some((run) => run.id === current) ? current : next[0]?.id ?? null
      );
    } catch (cause) {
      if (ticket === listTicket.current) setLoadError(describeError(cause));
    } finally {
      if (ticket === listTicket.current) setLoading(false);
    }
  }, [kind,offset,period,query,status]);

  const loadDetail = useCallback(async (operationId: string) => {
    const ticket = ++detailTicket.current;
    setDetailLoading(true);
    setDetailError(null);
    try {
      const next = await operationGetRun(operationId);
      if (ticket !== detailTicket.current) return;
      if (!next) {
        setDetail(null);
        setDetailError("Tác vụ không còn trong nguồn dữ liệu.");
        return;
      }
      setDetail(next);
    } catch (cause) {
      if (ticket === detailTicket.current) {
        setDetail(null);
        setDetailError(describeError(cause));
      }
    } finally {
      if (ticket === detailTicket.current) setDetailLoading(false);
    }
  }, []);

  useEffect(() => {
    const timer = window.setTimeout(() => void reload(), 200);
    const polling = window.setInterval(() => {
      void reload();
      const current = selectedRunIdRef.current;
      if (current) void loadDetail(current);
    }, 5000);
    return () => {
      window.clearTimeout(timer);
      window.clearInterval(polling);
      listTicket.current += 1;
      detailTicket.current += 1;
    };
  }, [loadDetail,reload]);

  useEffect(() => {
    selectedRunIdRef.current = selectedRunId;
  }, [selectedRunId]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const refreshForEvent = (event: AppEvent) => {
      if (![
        "jobUpdated",
        "flowRunUpdated",
        "orchestrationUpdated",
        "interactionUpdated",
        "publishUpdated",
        "nurtureStatus",
      ].includes(event.type)) return;
      void reload();
      const current = selectedRunIdRef.current;
      if (current) void loadDetail(current);
    };
    void listenRiviuEvents(refreshForEvent).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [loadDetail, reload]);

  useEffect(() => {
    setScriptJson(initialScript ?? "");
  }, [initialScript]);

  useEffect(() => {
    if (!selectedRunId) {
      setDetail(null);
      setDetailError(null);
      return;
    }
    setDetail(null);
    void loadDetail(selectedRunId);
  }, [loadDetail, selectedRunId]);

  const filtered = runs;
  const selectedRun = filtered.find((run) => run.id === selectedRunId) ?? filtered[0] ?? null;
  const visibleDetail = detail?.summary.id === selectedRun?.id ? detail : null;
  const shownSummary = visibleDetail?.summary ?? selectedRun;
  const interactionActorCount = shownSummary?.kind === "interaction"
    ? new Set(visibleDetail?.items.filter((item) => item.kind === "assignment")
      .map((item) => item.udid?.trim()).filter(Boolean)).size
    : 0;

  useEffect(() => {
    if (selectedRun && selectedRun.id !== selectedRunId) setSelectedRunId(selectedRun.id);
  }, [selectedRun, selectedRunId]);

  return (
    <div className="panel operations-page jobs-page">
      <section className="operations-summary" aria-label="Tổng quan tác vụ">
        <div><span>Đang thực hiện</span><strong>{page?.counts.active ?? "—"}</strong></div>
        <div><span>Hoàn tất</span><strong>{page?.counts.succeeded ?? "—"}</strong></div>
        <div><span>Cần xử lý</span><strong>{page?.counts.attention ?? "—"}</strong></div>
        <button type="button" className="ghost" disabled={loading} onClick={() => void reload()}>
          <RefreshCw size={15} /> Làm mới
        </button>
      </section>

      {loadError && (
        <StatusNotice
          tone="error"
          action={<button type="button" onClick={() => void reload()}>Thử lại</button>}
        >
          Không đọc được danh sách tác vụ: {loadError}
        </StatusNotice>
      )}
      {error && <StatusNotice tone="error">{error}</StatusNotice>}

      <section className="operations-monitor" aria-label="Theo dõi tác vụ">
        <div className="operations-monitor-list">
          <div className="operations-filterbar">
            <label>
              <Search size={15} aria-hidden="true" />
              <span className="visually-hidden">Tìm tác vụ</span>
              <input
                value={query}
                onChange={(event) => { setQuery(event.target.value); setOffset(0); }}
                placeholder="Tìm theo loại hoặc tên tác vụ"
              />
            </label>
            <select
              value={status}
              aria-label="Lọc trạng thái tác vụ"
              onChange={(event) => { setStatus(event.target.value as OperationRunState | "all"); setOffset(0); }}
            >
              <option value="all">Tất cả trạng thái</option>
              {Object.entries(RUN_LABEL).map(([value, label]) => (
                <option key={value} value={value}>{label}</option>
              ))}
            </select>
            <select aria-label="Loại tác vụ" value={kind} onChange={(event) => { setKind(event.target.value as OperationRunKind | "all"); setOffset(0); }}>
              <option value="all">Mọi loại tác vụ</option>
              {Object.entries(KIND_LABEL).map(([value,label]) => <option key={value} value={value}>{label}</option>)}
            </select>
            <select aria-label="Khoảng thời gian tác vụ" value={period} onChange={(event) => { setPeriod(event.target.value); setOffset(0); }}>
              <option value="24">24 giờ qua</option><option value="168">7 ngày qua</option><option value="all">Toàn bộ lịch sử</option>
            </select>
          </div>
          {page && <div className="admin-actions" aria-label="Phân trang tác vụ">
            <span>{page.total ? `${offset + 1}–${offset + runs.length}` : "0"} / {page.total} tác vụ</span>
            <button type="button" className="icon-btn" aria-label="Trang tác vụ trước" disabled={loading || offset === 0} onClick={() => setOffset(Math.max(0,offset - 50))}><ChevronLeft size={16}/></button>
            <button type="button" className="icon-btn" aria-label="Trang tác vụ sau" disabled={loading || !page.hasMore} onClick={() => setOffset(offset + 50)}><ChevronRight size={16}/></button>
          </div>}
          {loading && !runs.length && <LoadingState label="Đang tải tác vụ…" />}
          {!loading && !loadError && filtered.length === 0 && (
            <EmptyState
              compact
              title={runs.length ? "Không có tác vụ phù hợp" : "Chưa có tác vụ"}
              hint={runs.length
                ? "Đổi từ khóa hoặc bộ lọc trạng thái."
                : "Các lần chạy Nuôi, Tương tác, Đăng bài và Flow sẽ xuất hiện tại đây."}
            />
          )}
          <div className="operations-run-list">
            {filtered.map((run) => (
              <button
                type="button"
                key={run.id}
                className={shownSummary?.id === run.id ? "is-active" : ""}
                aria-pressed={shownSummary?.id === run.id}
                onClick={() => setSelectedRunId(run.id)}
              >
                <span>
                  <strong>{run.title}</strong>
                  <small>{KIND_LABEL[run.kind]} · {formatTimestamp(run.updatedAt)}</small>
                </span>
                <span className={`pill ${run.state}`}>{RUN_LABEL[run.state]}</span>
              </button>
            ))}
          </div>
        </div>

        <div className="operations-monitor-detail">
          {!selectedRun ? (
            <EmptyState compact title="Chọn một tác vụ để xem tiến độ" />
          ) : detailLoading && !visibleDetail ? (
            <LoadingState label="Đang tải chi tiết tác vụ…" />
          ) : detailError ? (
            <StatusNotice
              tone="error"
              action={(
                <button type="button" onClick={() => void loadDetail(selectedRun.id)}>
                  Thử lại chi tiết
                </button>
              )}
            >
              Không đọc được chi tiết tác vụ: {detailError}
            </StatusNotice>
          ) : shownSummary && visibleDetail ? (
            <>
              <header>
                <div>
                  <strong>{shownSummary.title}</strong>
                  <span>
                    {KIND_LABEL[shownSummary.kind]} · {shownSummary.targetCount} {shownSummary.kind === "interaction" ? "bài" : "máy"}
                    {interactionActorCount > 0 ? ` · ${interactionActorCount} máy` : ""} · {RUN_LABEL[shownSummary.state]}
                  </span>
                </div>
                {shownSummary.kind === "script" && isActive(shownSummary) && (
                  <button
                    type="button"
                    className="ghost"
                    disabled={busy}
                    onClick={async () => {
                      setBusy(true);
                      setError(null);
                      try {
                        await cancelJob(shownSummary.sourceId);
                        await reload();
                        await loadDetail(shownSummary.id);
                      } catch (cause) {
                        setError(describeError(cause));
                      } finally {
                        setBusy(false);
                      }
                    }}
                  >
                    <Square size={14} /> Dừng tác vụ
                  </button>
                )}
              </header>
              {shownSummary.issueCount > 0 && (
                <StatusNotice tone={shownSummary.state === "uncertain" ? "warning" : "error"}>
                  {shownSummary.issueCount} mục cần kiểm tra. Mở chi tiết từng mục để xem bằng chứng.
                </StatusNotice>
              )}
              {visibleDetail.items.length ? (
                <ol className="operations-timeline">
                  {visibleDetail.items.map((item) => (
                    <li key={item.id} className={item.state}>
                      <span aria-hidden="true" />
                      <div>
                        <strong>{visibleDetail.batch ? item.label : item.udid ? deviceNames.get(item.udid) ?? item.label : item.label}</strong>
                        <small>
                          {RUN_LABEL[item.state]}
                          {item.retryable ? " · Có thể chạy lại từ nguồn gốc" : ""}
                        </small>
                        {(item.udid || item.errorCode || item.detail || item.evidence) && (
                          <details>
                            <summary>Chi tiết kỹ thuật</summary>
                            {item.udid && <code>{item.udid}</code>}
                            {item.detail && <p>{item.detail}</p>}
                            {item.errorCode && <code>{item.errorCode}</code>}
                            {item.evidence && <code>{item.evidence}</code>}
                          </details>
                        )}
                      </div>
                    </li>
                  ))}
                </ol>
              ) : (
                <EmptyState compact title="Tác vụ chưa có bước chi tiết" />
              )}
              <details className="operations-technical-details">
                <summary>Mã tác vụ và tiến độ</summary>
                <code>{shownSummary.sourceId}</code>
                <p>{shownSummary.completedItems}/{shownSummary.totalItems} mục đã kết thúc</p>
                {shownSummary.retryScope && (
                  <p>Phạm vi khôi phục: {RETRY_SCOPE_LABEL[shownSummary.retryScope]}</p>
                )}
                {shownSummary.retryableCount > 0 && <p>{shownSummary.retryableCount} mục có thể chạy lại từ workspace gốc.</p>}
              </details>
            </>
          ) : null}
        </div>
      </section>

      <details className="operations-advanced">
        <summary><Braces size={16} /> Chạy JSON nâng cao</summary>
        <div>
          <SelectionStrip
            devices={devices}
            selected={selectedUdids}
            onSelectAll={() => onSelectUdids(devices.map((device) => device.udid))}
            onClear={() => onSelectUdids([])}
          />
          <label>
            <span>Kịch bản JSON</span>
            <textarea
              rows={12}
              value={scriptJson}
              onChange={(event) => setScriptJson(event.target.value)}
              placeholder="Dán kịch bản JSON đã kiểm tra"
            />
          </label>
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
                  await reload();
                  flash(`Đã xếp tác vụ cho ${targets.length} máy`);
                } catch (cause) {
                  setError(describeError(cause));
                } finally {
                  setBusy(false);
                }
              }}
            >
              Chạy trên {targets.length} máy
            </button>
          </div>
        </div>
      </details>
    </div>
  );
}
