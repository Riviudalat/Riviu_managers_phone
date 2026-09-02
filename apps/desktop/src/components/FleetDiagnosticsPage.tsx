import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { deviceHealth } from "../api";
import { describeError } from "../describeError";
import {
  fleetHealthJson,
  createHealthLimiter,
  loadFleetHealth,
  normalizeDeviceHealth,
  withHealthDeadline,
  type FleetHealthRow,
  type HealthStatus,
} from "../diagnostics";
import type { DeviceInfo, DeviceMeta } from "../types";
import { EmptyState, LoadingState, StatusNotice } from "./States";

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

function deviceLabel(device: DeviceInfo, meta: DeviceMeta | undefined, position: number): string {
  const number = meta?.number ?? position + 1;
  const alias = meta?.alias?.trim() || device.name;
  return `Máy ${number} · ${alias} · ${device.model}`;
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

/**
 * Fleet-wide, read-only health snapshot. The command behind each row takes no lease and this
 * page never calls a repair, prepare, install, or any other phone-mutating route.
 */
export function FleetDiagnosticsPage({
  devices,
  metas,
  onExport = defaultExport,
}: {
  devices: DeviceInfo[];
  metas: DeviceMeta[];
  /** Injected in tests; production downloads a JSON file. */
  onExport?: (json: string) => void;
}) {
  const metasByUdid = useMemo(() => new Map(metas.map((meta) => [meta.udid, meta])), [metas]);
  const [rows, setRows] = useState<DisplayRow[] | null>(null);
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
    setRows((previous) => previous?.map((row) =>
      row.device.udid === device.udid ? { device, loading: true } : row,
    ) ?? null);
    try {
      const report = await limiter.current.run(() => withHealthDeadline(() => deviceHealth(device.udid)));
      setRows((previous) => previous?.map((row) =>
        row.device.udid === device.udid
          ? { device, report, checks: normalizeDeviceHealth(device, report), loading: false }
          : row,
      ) ?? null);
    } catch (cause) {
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
  return (
    <section className="fleet-diagnostics" aria-label="Chẩn đoán fleet">
      <header className="panel-header">
        <div>
          <p className="hint">{complete}/{rows.length} máy đã có kết quả</p>
        </div>
        <span className="grow" />
        <button type="button" className="ghost" onClick={() => hydrate()} disabled={complete < rows.length}>
          Kiểm lại tất cả
        </button>
        <button type="button" className="ghost" onClick={() => onExport(fleetHealthJson(rows))} disabled={complete === 0}>
          Xuất JSON
        </button>
      </header>

      {complete < rows.length && <LoadingState label={`Đang kiểm tra ${devices.length} máy…`} />}
      {allErrors && (
        <StatusNotice tone="error">
          Chưa đọc được kết quả từ máy nào. Mỗi hàng bên dưới có thể kiểm lại riêng.
        </StatusNotice>
      )}

      <div className="fleet-diagnostics-rows">
        {rows.map((row, position) => {
          const label = deviceLabel(row.device, metasByUdid.get(row.device.udid), position);
          return (
            <article key={row.device.udid} className="fleet-diagnostics-row" aria-label={label}>
              <header>
                <strong>{label}</strong>
                <button
                  type="button"
                  className="link"
                  onClick={() => void retry(row.device)}
                  disabled={row.loading}
                  aria-label={`Kiểm lại ${label}`}
                >
                  {row.loading ? "Đang kiểm…" : "Kiểm lại"}
                </button>
              </header>
              {row.loading && <LoadingState label="Đang hỏi máy…" />}
              {row.error && (
                <>
                  <p role="alert">{row.error}</p>
                  {row.errorDetail && (
                    <details aria-label={`Chi tiết lỗi ${label}`}>
                      <summary>Chi tiết lỗi</summary>
                      <pre>{row.errorDetail}</pre>
                    </details>
                  )}
                </>
              )}
              {row.checks && (
                <>
                  <ul className="health-check-overview" aria-label={`Trạng thái kiểm tra ${label}`}>
                    {row.checks.map((item) => (
                      <li key={item.id} data-health-status={item.status}>
                        <span>{item.label}</span>
                        <strong className={`health-status health-${item.status}`}>{STATUS_LABEL[item.status]}</strong>
                      </li>
                    ))}
                  </ul>
                  <details role="group" aria-label={`Bằng chứng kỹ thuật ${label}`}>
                    <summary>Bằng chứng kỹ thuật</summary>
                    <ul>
                      {row.checks.map((item) => (
                        <li key={item.id}>
                          <strong>{item.label}:</strong> <span>{item.summary}</span>
                          {item.detail && <small>{item.detail}</small>}
                        </li>
                      ))}
                    </ul>
                  </details>
                </>
              )}
            </article>
          );
        })}
      </div>
    </section>
  );
}
