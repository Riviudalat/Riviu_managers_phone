import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Download, RefreshCw, Search } from "lucide-react";

import { deviceHealth } from "../api";
import { describeError } from "../describeError";
import {
  fleetHealthJson,
  createHealthLimiter,
  loadFleetHealth,
  normalizeDeviceHealth,
  withHealthDeadline,
  type DeviceHealthCheck,
  type FleetHealthRow,
  type HealthStatus,
} from "../diagnostics";
import type { DeviceInfo, DeviceMeta } from "../types";
import { EmptyState, LoadingState, StatusNotice } from "./States";
import {
  DetailDrawer,
  ResponsiveTable,
  StatusChip,
  type StatusTone,
} from "./WorkspacePrimitives";

interface DisplayRow extends FleetHealthRow {
  loading?: boolean;
}

const STATUS_LABEL: Record<HealthStatus, string> = {
  pass: "Đạt",
  warning: "Cần xem",
  fail: "Lỗi",
  unknown: "Chưa rõ",
  notApplicable: "Không áp dụng",
};

const STATUS_TONE: Record<HealthStatus, StatusTone> = {
  pass: "success",
  warning: "warning",
  fail: "error",
  unknown: "neutral",
  notApplicable: "neutral",
};

const STATUS_WEIGHT: Record<HealthStatus, number> = {
  fail: 4,
  warning: 3,
  unknown: 2,
  pass: 1,
  notApplicable: 0,
};

function deviceLabel(device: DeviceInfo, meta: DeviceMeta | undefined, position: number): string {
  const number = meta?.number ?? position + 1;
  const alias = meta?.alias?.trim() || device.name;
  return `Máy ${number} · ${alias} · ${device.model}`;
}

function primaryDeviceLabel(
  device: DeviceInfo,
  meta: DeviceMeta | undefined,
  position: number,
): { name: string; model: string } {
  return {
    name: `Máy ${meta?.number ?? position + 1} · ${meta?.alias?.trim() || device.name}`,
    model: device.model,
  };
}

function defaultExport(json: string): void {
  const blob = new Blob([json], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = "riviu-fleet-diagnostics.json";
  anchor.click();
  URL.revokeObjectURL(url);
}

function clusterStatus(
  row: DisplayRow,
  ids: DeviceHealthCheck["id"][],
): HealthStatus | null {
  if (!row.checks) return null;
  return row.checks
    .filter((check) => ids.includes(check.id))
    .map((check) => check.status)
    .sort((left, right) => STATUS_WEIGHT[right] - STATUS_WEIGHT[left])[0] ?? null;
}

function statusCell(status: HealthStatus | null, loading?: boolean) {
  if (loading) return <StatusChip>Đang kiểm tra</StatusChip>;
  if (!status) return <StatusChip tone="error">Không đọc được</StatusChip>;
  return <StatusChip tone={STATUS_TONE[status]}>{STATUS_LABEL[status]}</StatusChip>;
}

/** Fleet-wide read-only health snapshot. Every retry remains scoped to one row. */
export function FleetDiagnosticsPage({
  devices,
  metas,
  onExport = defaultExport,
}: {
  devices: DeviceInfo[];
  metas: DeviceMeta[];
  onExport?: (json: string) => void;
}) {
  const metasByUdid = useMemo(() => new Map(metas.map((meta) => [meta.udid, meta])), [metas]);
  const [rows, setRows] = useState<DisplayRow[] | null>(null);
  const [detailUdid, setDetailUdid] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [attentionOnly, setAttentionOnly] = useState(false);
  const generation = useRef(0);
  const activeRosterKey = useRef("");
  const limiter = useRef(createHealthLimiter());
  const devicesRef = useRef(devices);
  devicesRef.current = devices;
  const rosterKey = devices.map((device) => device.udid).join("\u0000");

  const hydrate = useCallback((key = rosterKey) => {
    const snapshot = devicesRef.current;
    const current = generation.current + 1;
    generation.current = current;
    activeRosterKey.current = key;
    setRows(snapshot.map((device) => ({ device, loading: true })));
    void loadFleetHealth(snapshot, deviceHealth, (row) => {
      if (generation.current !== current || activeRosterKey.current !== key) return;
      setRows((previous) => previous?.map((candidate) =>
        candidate.device.udid === row.device.udid ? { ...row, loading: false } : candidate,
      ) ?? null);
    }, 4, limiter.current).then(() => undefined);
  }, [rosterKey]);

  useEffect(() => {
    hydrate();
    return () => { generation.current += 1; };
  }, [hydrate]);

  const retry = useCallback(async (device: DeviceInfo) => {
    const retryGeneration = generation.current;
    const retryRosterKey = activeRosterKey.current;
    setRows((previous) => previous?.map((row) =>
      row.device.udid === device.udid ? { device, loading: true } : row,
    ) ?? null);
    try {
      const report = await limiter.current.run(() => withHealthDeadline(() => deviceHealth(device.udid)));
      if (
        generation.current !== retryGeneration ||
        activeRosterKey.current !== retryRosterKey
      ) return;
      setRows((previous) => previous?.map((row) =>
        row.device.udid === device.udid
          ? { device, report, checks: normalizeDeviceHealth(device, report), loading: false }
          : row,
      ) ?? null);
    } catch (cause) {
      if (
        generation.current !== retryGeneration ||
        activeRosterKey.current !== retryRosterKey
      ) return;
      const errorDetail = describeError(cause);
      setRows((previous) => previous?.map((row) =>
        row.device.udid === device.udid
          ? { device, error: "Không đọc được trạng thái máy. Hãy kiểm lại.", errorDetail, loading: false }
          : row,
      ) ?? null);
    }
  }, []);

  if (!devices.length) {
    return (
      <EmptyState
        title="Chưa có điện thoại nào"
        hint="Cắm máy qua USB rồi làm mới danh sách thiết bị trước khi kiểm tra."
      />
    );
  }

  if (rows === null) return <LoadingState label={`Đang kiểm tra ${devices.length} máy…`} />;

  const complete = rows.filter((row) => !row.loading).length;
  const allErrors = rows.every((row) => Boolean(row.error));
  const countByStatus = (status: HealthStatus) => rows.reduce(
    (total, row) => total + (row.checks?.filter((check) => check.status === status).length ?? 0),
    0,
  );
  const filteredRows = rows.filter((row, position) =>
    (!attentionOnly || Boolean(row.error) || row.checks?.some((check) => ["warning", "fail", "unknown"].includes(check.status))) &&
    deviceLabel(row.device, metasByUdid.get(row.device.udid), position)
      .toLowerCase()
      .includes(query.trim().toLowerCase()),
  );
  const detailRow = rows.find((row) => row.device.udid === detailUdid) ?? null;
  const detailPosition = detailRow ? rows.indexOf(detailRow) : -1;
  const detailLabel = detailRow
    ? deviceLabel(detailRow.device, metasByUdid.get(detailRow.device.udid), detailPosition)
    : "Chi tiết chẩn đoán";

  return (
    <section className="admin-workspace fleet-diagnostics" aria-label="Chẩn đoán thiết bị">
      <div className="admin-toolbar">
        <div className="admin-toolbar-copy">
          <strong>{complete}/{rows.length} máy đã có kết quả</strong>
        </div>
        <div className="admin-toolbar-actions">
          <label className="agent-toggle"><input type="checkbox" checked={attentionOnly} onChange={(event) => setAttentionOnly(event.target.checked)} />Chỉ máy cần xem</label>
          <label className="search-field">
            <Search size={15} aria-hidden="true" />
            <span className="visually-hidden">Tìm thiết bị</span>
            <input
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Tìm số máy, alias, model…"
            />
          </label>
          <button type="button" className="ghost" onClick={() => hydrate()} disabled={complete < rows.length}>
            <RefreshCw size={15} aria-hidden="true" />
            Kiểm lại tất cả
          </button>
          <button type="button" className="ghost" onClick={() => onExport(fleetHealthJson(rows))} disabled={complete === 0}>
            <Download size={15} aria-hidden="true" />
            Xuất JSON
          </button>
        </div>
      </div>

      <div className="diagnostics-summary" aria-label="Tổng hợp kiểm tra">
        <StatusChip tone="success">{countByStatus("pass")} đạt</StatusChip>
        <StatusChip tone="warning">{countByStatus("warning")} cần xem</StatusChip>
        <StatusChip tone="error">{countByStatus("fail")} lỗi</StatusChip>
        <StatusChip>{countByStatus("unknown")} chưa rõ</StatusChip>
      </div>

      {complete < rows.length && <LoadingState label={`Đang kiểm tra ${rows.length - complete} máy còn lại…`} />}
      {allErrors && (
        <StatusNotice tone="error">
          Chưa đọc được kết quả từ máy nào. Kiểm lại từng hàng để giữ nguyên kết quả của máy khác.
        </StatusNotice>
      )}

      <ResponsiveTable
        label="Kết quả chẩn đoán thiết bị"
        viewKey="diagnostics"
        rows={filteredRows}
        keyForRow={(row) => row.device.udid}
        labelForRow={(row) => {
          const position = rows.indexOf(row);
          return deviceLabel(row.device, metasByUdid.get(row.device.udid), position);
        }}
        empty={(
          <EmptyState
            compact
            title={attentionOnly && !query.trim() ? "Không có máy cần xem thêm" : "Không tìm thấy thiết bị"}
            hint={attentionOnly ? "Bỏ bộ lọc để xem toàn bộ kết quả chẩn đoán." : "Đổi từ khóa để xem lại toàn bộ kết quả chẩn đoán."}
          />
        )}
        columns={[
          {
            id: "device",
            label: "Thiết bị",
            sortValue: (row) => metasByUdid.get(row.device.udid)?.number ?? rows.indexOf(row) + 1,
            render: (row) => {
              const position = rows.indexOf(row);
              const label = primaryDeviceLabel(row.device, metasByUdid.get(row.device.udid), position);
              return <span className="admin-device-name"><strong>{label.name}</strong><small>{label.model}</small></span>;
            },
          },
          {
            id: "transport",
            label: "Kết nối",
            render: (row) => statusCell(clusterStatus(row, ["roster", "transport", "adb"]), row.loading),
          },
          {
            id: "agent",
            label: "Điều khiển",
            render: (row) => statusCell(clusterStatus(row, ["agentCache", "agentLive", "agentCapabilities"]), row.loading),
          },
          {
            id: "helper",
            label: "Kết nối phụ trợ",
            render: (row) => statusCell(clusterStatus(row, ["helperInstalled", "helperReachable"]), row.loading),
          },
          {
            id: "runtime",
            label: "TikTok / luồng",
            render: (row) => statusCell(clusterStatus(row, ["tiktok", "geometry", "stream"]), row.loading),
          },
          {
            id: "actions",
            label: "Thao tác",
            required: true,
            render: (row) => {
              const position = rows.indexOf(row);
              const label = deviceLabel(row.device, metasByUdid.get(row.device.udid), position);
              return (
                <span className="admin-actions">
                  <button
                    type="button"
                    className="link"
                    onClick={() => setDetailUdid(row.device.udid)}
                    disabled={row.loading}
                    aria-label={`Xem chi tiết ${label}`}
                  >
                    Chi tiết
                  </button>
                  <button
                    type="button"
                    className="link"
                    onClick={() => void retry(row.device)}
                    disabled={row.loading}
                    aria-label={`Kiểm lại ${label}`}
                  >
                    Kiểm lại
                  </button>
                </span>
              );
            },
          },
        ]}
      />

      <DetailDrawer
        open={detailRow !== null}
        title={detailLabel}
        description="Kết quả và bằng chứng của lần kiểm tra gần nhất"
        onClose={() => setDetailUdid(null)}
        footer={detailRow ? (
          <button type="button" className="primary" onClick={() => void retry(detailRow.device)} disabled={detailRow.loading}>
            <RefreshCw size={15} aria-hidden="true" />
            Kiểm lại máy này
          </button>
        ) : undefined}
      >
        {detailRow?.loading && <LoadingState label="Đang hỏi máy…" />}
        {detailRow?.error && <StatusNotice tone="error">{detailRow.error}</StatusNotice>}
        {detailRow?.errorDetail && (
          <details className="admin-detail">
            <summary>Chi tiết lỗi</summary>
            <pre>{detailRow.errorDetail}</pre>
          </details>
        )}
        {detailRow?.checks && (
          <ul className="diagnostics-drawer-list" aria-label={`Trạng thái kiểm tra ${detailLabel}`}>
            {detailRow.checks.map((item) => (
              <li key={item.id} data-health-status={item.status}>
                <strong>{item.label}</strong>
                <StatusChip tone={STATUS_TONE[item.status]}>{STATUS_LABEL[item.status]}</StatusChip>
                <p>{item.summary}</p>
                {item.detail && (
                  <details className="admin-detail">
                    <summary>Bằng chứng kỹ thuật</summary>
                    <pre>{item.detail}</pre>
                  </details>
                )}
              </li>
            ))}
          </ul>
        )}
      </DetailDrawer>
    </section>
  );
}
