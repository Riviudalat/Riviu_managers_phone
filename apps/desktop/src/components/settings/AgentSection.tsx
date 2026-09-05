import { useCallback, useEffect, useRef, useState } from "react";
import { Save } from "lucide-react";
import { useWorkspaceDraft } from "../../workspaceDraft";
import {
  agentGetSettings,
  agentListStatuses,
  agentPreflight,
  agentRepair,
  agentSaveSettings,
} from "../../api";
import { agentStatusView } from "../../agentStatus";
import { describeError } from "../../describeError";
import { EmptyState, StatusNotice } from "../States";
import { IconPhone } from "../Icons";
import type { AgentRuntimeView, AgentStatus, DeviceInfo } from "../../types";

type AgentAction = "check" | "repair";

/** The Riviu agent on each phone: its version, its status, and repairing it. */
export function AgentSection({ connectedDevices, connectedUdids, deviceLabels }: {
  connectedDevices: DeviceInfo[];
  connectedUdids: string[];
  deviceLabels?: ReadonlyMap<string, string>;
}) {
  const [runtime, setRuntime] = useState<AgentRuntimeView | null>();
  const [statuses, setStatuses] = useState<Record<string, AgentStatus>>({});
  const [statusesLoading, setStatusesLoading] = useState(false);
  const [busy, setBusy] = useState<Record<string, AgentAction>>({});
  const [savingSettings, setSavingSettings] = useState(false);
  const [autoRepair, setAutoRepair] = useState(false);
  const editEpoch = useRef(0);
  const savingRef = useRef(false);
  const [agentMessage, setAgentMessage] = useState<string | null>(null);
  const [runtimeError, setRuntimeError] = useState<string | null>(null);
  const [statusesError, setStatusesError] = useState<string | null>(null);

  const loadRuntime = useCallback(() => {
    setRuntime(undefined);
    setRuntimeError(null);
    agentGetSettings()
      .then((next) => { setRuntime(next); setAutoRepair(next.settings.autoRepair); })
      .catch((error) => {
        setRuntime(null);
        setRuntimeError(describeError(error));
      });
  }, []);

  useEffect(loadRuntime, [loadRuntime]);

  const loadStatuses = useCallback(() => {
    if (!connectedUdids.length) {
      setStatuses({});
      setStatusesError(null);
      return;
    }
    setStatusesLoading(true);
    setStatusesError(null);
    agentListStatuses(connectedUdids)
      .then((items) => {
        setStatuses(Object.fromEntries(items.map((status) => [status.udid, status])));
      })
      .catch((error) => setStatusesError(describeError(error)))
      .finally(() => setStatusesLoading(false));
  }, [connectedUdids]);

  useEffect(loadStatuses, [loadStatuses]);

  const dirty = Boolean(runtime && runtime.settings.autoRepair !== autoRepair);
  const saveSettings = async () => {
    if (!runtime || savingRef.current) return false;
    const epoch = editEpoch.current;
    savingRef.current = true;
    setSavingSettings(true);
    setAgentMessage(null);
    try {
      const next = await agentSaveSettings({ autoRepair });
      setRuntime(next);
      if (epoch === editEpoch.current) setAutoRepair(next.settings.autoRepair);
      return epoch === editEpoch.current;
    } catch (error) {
      setAgentMessage(describeError(error));
      return false;
    } finally {
      savingRef.current = false;
      setSavingSettings(false);
    }
  };
  const discard = () => {
    editEpoch.current += 1;
    setAutoRepair(runtime?.settings.autoRepair ?? false);
  };
  useWorkspaceDraft({ id: "settings-agent", label: "Tự khôi phục Agent", dirty, snapshotKey: JSON.stringify(autoRepair), save: saveSettings, discard });

  const runAgentAction = async (udid: string, action: AgentAction) => {
    setBusy((current) => ({ ...current, [udid]: action }));
    setAgentMessage(null);
    try {
      const status =
        action === "check" ? await agentPreflight(udid) : await agentRepair(udid);
      setStatuses((current) => ({ ...current, [udid]: status }));
    } catch (error) {
      setAgentMessage(describeError(error));
    } finally {
      setBusy((current) => {
        const next = { ...current };
        delete next[udid];
        return next;
      });
    }
  };

  const protocolVersion =
    Object.values(statuses).find((status) => status.protocolVersion > 0)?.protocolVersion ?? null;
  return (
    <section className="settings-section">
      <div className="settings-section-heading">
        <div>
          <h3>Riviu Agent</h3>
          <p className="hint">Giữ kết nối hình ảnh, thao tác và bình luận chữ trên từng điện thoại.</p>
        </div>
        <span
          className={`chip ${runtime === undefined ? "info" : runtime?.tokenConfigured ? "ok" : "warn"}`}
        >
          {runtime === undefined
            ? "Đang đọc thông tin xác thực"
            : runtime === null
              ? "Chưa rõ trạng thái xác thực"
              : runtime.tokenConfigured
                ? "Đã lưu thông tin xác thực"
                : "Chưa cấu hình thông tin xác thực"}
        </span>
      </div>

      <dl className="agent-runtime-meta">
        <div>
          <dt>Kết nối</dt>
          <dd>{connectedDevices.length} thiết bị</dd>
        </div>
        <div>
          <dt>Agent đang dùng</dt>
          <dd>{runtime === undefined ? "Đang đọc…" : runtime === null ? "Chưa rõ" : "Đã xác định"}</dd>
        </div>
        <div>
          <dt>Thông tin xác thực</dt>
          <dd>
            {runtime === undefined
              ? "Đang đọc…"
              : runtime === null
                ? "Chưa rõ"
                : runtime.tokenConfigured
                  ? "Đã lưu trong kho thông tin xác thực Windows"
                  : "Chưa cấu hình"}
          </dd>
        </div>
      </dl>

      <details className="settings-details" aria-label="Chi tiết Riviu Agent">
        <summary>Chi tiết Riviu Agent</summary>
        <dl className="agent-runtime-meta">
          <div>
            <dt>Mã gói</dt>
            <dd><code>{runtime?.activeArtifactId ?? "Chưa rõ"}</code></dd>
          </div>
          <div>
            <dt>Phiên bản gói</dt>
            <dd>{runtime?.activeArtifactVersion ?? "Chưa rõ"}</dd>
          </div>
          <div>
            <dt>Giao thức</dt>
            <dd>{protocolVersion ?? "Chưa rõ"}</dd>
          </div>
        </dl>
      </details>

      <label className="agent-toggle">
        <input
          type="checkbox"
          checked={autoRepair}
          disabled={!runtime}
          onChange={(event) => { editEpoch.current += 1; setAutoRepair(event.target.checked); }}
        />
        Tự khôi phục Agent
      </label>
      <div className="row">
        <button type="button" className="primary" disabled={!dirty || savingSettings} onClick={() => void saveSettings()}><Save size={15} />{savingSettings ? "Đang lưu…" : "Lưu tự khôi phục"}</button>
        {dirty && <button type="button" className="ghost" disabled={savingSettings} onClick={discard}>Bỏ thay đổi</button>}
      </div>

      {runtimeError && (
        <StatusNotice
          tone="error"
          action={
            <button type="button" className="secondary" onClick={loadRuntime}>
              Thử lại cấu hình
            </button>
          }
        >
          {runtimeError}
        </StatusNotice>
      )}
      {statusesError && (
        <StatusNotice
          tone="error"
          action={
            <button type="button" className="secondary" onClick={loadStatuses}>
              Thử lại trạng thái
            </button>
          }
        >
          {statusesError}
        </StatusNotice>
      )}
      {agentMessage && <StatusNotice tone="error">{agentMessage}</StatusNotice>}

      {!connectedDevices.length ? (
        <EmptyState
          compact
          icon={<IconPhone size={15} />}
          title="Chưa có điện thoại đang kết nối"
          hint="Cắm máy qua USB rồi làm mới ở Quản lý cửa sổ."
        />
      ) : (
        <div className="agent-status-table" role="table" aria-label="Trạng thái Agent">
          <div className="agent-status-head" role="row">
            <span role="columnheader">Thiết bị</span>
            <span role="columnheader">Trạng thái</span>
            <span role="columnheader">Bản dựng</span>
            <span role="columnheader">Xác thực</span>
            <span role="columnheader">Hình ảnh</span>
            <span role="columnheader">Phiên</span>
            <span role="columnheader">Thao tác</span>
          </div>
          {connectedDevices.map((device) => {
            const status = statuses[device.udid];
            const view = status
              ? agentStatusView(status)
              : {
                  label: statusesLoading ? "Đang đọc…" : "Chưa kiểm tra",
                  tone: "info" as const,
                  textCommentsEnabled: false,
                  message: null,
                };
            const action = busy[device.udid];
            return (
              <div className="agent-status-row" role="row" key={device.udid}>
                <div className="agent-device-name" role="cell">
                  <strong>{deviceLabels?.get(device.udid) ?? device.name}</strong>
                  <details className="agent-device-details">
                    <summary>Chi tiết thiết bị</summary>
                    <dl>
                      <div><dt>Model</dt><dd>{device.model}</dd></div>
                      <div><dt>Serial</dt><dd><code>{device.udid}</code></dd></div>
                    </dl>
                  </details>
                </div>
                <span role="cell">
                  <span className={`chip ${view.tone}`}>
                    {view.label}
                  </span>
                  {view.message && (
                    <details className="agent-status-details">
                      <summary>Chi tiết trạng thái</summary>
                      <span>{view.message}</span>
                    </details>
                  )}
                </span>
                <span className="mono" role="cell">
                  {status?.installedVersion ?? "-"}
                  {status?.installedBuild ? ` (${status.installedBuild})` : ""}
                </span>
                <span className="agent-readiness" role="cell">
                  {readinessValue(status, status?.authReady)}
                </span>
                <span className="agent-readiness" role="cell">
                  {readinessValue(status, status?.mjpegReady)}
                </span>
                <span className="agent-readiness" role="cell">
                  {readinessValue(status, status?.sessionReady)}
                </span>
                <div className="agent-status-actions" role="cell">
                  <button
                    type="button"
                    className="ghost"
                    disabled={Boolean(action)}
                    onClick={() => void runAgentAction(device.udid, "check")}
                  >
                    {action === "check" ? "Đang kiểm tra…" : "Kiểm tra"}
                  </button>
                  <button
                    type="button"
                    className="primary"
                    disabled={Boolean(action)}
                    onClick={() => void runAgentAction(device.udid, "repair")}
                  >
                    {action === "repair" ? "Đang khôi phục…" : "Khôi phục"}
                  </button>
                  <details className="agent-readiness-details">
                    <summary>Chi tiết sẵn sàng</summary>
                    <span>
                      Xác thực: {readinessValue(status, status?.authReady)} · Hình ảnh:{" "}
                      {readinessValue(status, status?.mjpegReady)} · Phiên:{" "}
                      {readinessValue(status, status?.sessionReady)}
                    </span>
                  </details>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}

function readinessValue(status: AgentStatus | undefined, value: boolean | undefined): string {
  if (!status || status.state === "unknown" || status.state === "starting") return "Chưa rõ";
  return value ? "Có" : "Không";
}
