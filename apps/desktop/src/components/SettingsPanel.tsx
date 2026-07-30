import { useEffect, useMemo, useState } from "react";
import {
  agentGetSettings,
  agentListStatuses,
  agentPreflight,
  agentRepair,
  agentSaveSettings,
  clearAppleId,
  driverMode,
  getAppleId,
  setAppleId,
} from "../api";
import { agentStatusView } from "../agentStatus";
import type { AgentRuntimeView, AgentStatus, DeviceInfo } from "../types";

interface Props {
  devices: DeviceInfo[];
}

type AgentAction = "check" | "repair";

export function SettingsPanel({ devices }: Props) {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [hasPassword, setHasPassword] = useState(false);
  const [mode, setMode] = useState("...");
  const [legacyMessage, setLegacyMessage] = useState<string | null>(null);
  const [runtime, setRuntime] = useState<AgentRuntimeView | null>(null);
  const [statuses, setStatuses] = useState<Record<string, AgentStatus>>({});
  const [busy, setBusy] = useState<Record<string, AgentAction>>({});
  const [savingSettings, setSavingSettings] = useState(false);
  const [agentMessage, setAgentMessage] = useState<string | null>(null);

  const connectedDevices = useMemo(
    () => devices.filter((device) => device.status !== "disconnected"),
    [devices],
  );
  const connectedUdids = useMemo(
    () => connectedDevices.map((device) => device.udid),
    [connectedDevices],
  );

  useEffect(() => {
    agentGetSettings()
      .then(setRuntime)
      .catch((error) => setAgentMessage(String(error)));
    getAppleId()
      .then((config) => {
        setEmail(config.email);
        setHasPassword(config.hasPassword);
      })
      .catch(() => undefined);
    driverMode().then(setMode).catch(() => setMode("unknown"));
  }, []);

  useEffect(() => {
    if (!connectedUdids.length) {
      setStatuses({});
      return;
    }
    agentListStatuses(connectedUdids)
      .then((items) => {
        setStatuses(Object.fromEntries(items.map((status) => [status.udid, status])));
      })
      .catch((error) => setAgentMessage(String(error)));
  }, [connectedUdids]);

  const runAgentAction = async (udid: string, action: AgentAction) => {
    setBusy((current) => ({ ...current, [udid]: action }));
    setAgentMessage(null);
    try {
      const status =
        action === "check" ? await agentPreflight(udid) : await agentRepair(udid);
      setStatuses((current) => ({ ...current, [udid]: status }));
    } catch (error) {
      setAgentMessage(String(error));
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
    <div className="panel">
      <header className="panel-header">
        <h2>Settings</h2>
      </header>

      <section className="settings-section">
        <div className="settings-section-heading">
          <div>
            <h3>Riviu Agent</h3>
            <p className="hint">Runtime chính cho stream, gesture và bình luận chữ.</p>
          </div>
          <span className={`chip ${runtime?.tokenConfigured ? "ok" : "warn"}`}>
            {runtime?.tokenConfigured ? "Credential stored" : "Credential missing"}
          </span>
        </div>

        <dl className="agent-runtime-meta">
          <div>
            <dt>Active artifact</dt>
            <dd>
              <code>{runtime?.activeArtifactId ?? "..."}</code>
              {runtime?.activeArtifactVersion ? ` v${runtime.activeArtifactVersion}` : ""}
            </dd>
          </div>
          <div>
            <dt>Protocol</dt>
            <dd>{protocolVersion ?? "-"}</dd>
          </div>
          <div>
            <dt>Credential</dt>
            <dd>
              {runtime?.tokenConfigured
                ? "Stored in OS credential store"
                : "Not configured"}
            </dd>
          </div>
        </dl>

        <label className="agent-toggle">
          <input
            type="checkbox"
            checked={runtime?.settings.autoRepair ?? false}
            disabled={!runtime || savingSettings}
            onChange={async (event) => {
              if (!runtime) return;
              const settings = { autoRepair: event.target.checked };
              setSavingSettings(true);
              setAgentMessage(null);
              try {
                setRuntime(await agentSaveSettings(settings));
              } catch (error) {
                setAgentMessage(String(error));
              } finally {
                setSavingSettings(false);
              }
            }}
          />
          Auto repair
        </label>

        {agentMessage && <p className="error">{agentMessage}</p>}

        {!connectedDevices.length ? (
          <p className="hint">Chưa có iPhone đang kết nối.</p>
        ) : (
          <div className="agent-status-table" role="table" aria-label="Agent readiness">
            <div className="agent-status-head" role="row">
              <span>Device</span>
              <span>State</span>
              <span>Build</span>
              <span>Auth</span>
              <span>MJPEG</span>
              <span>Session</span>
              <span>Actions</span>
            </div>
            {connectedDevices.map((device) => {
              const status = statuses[device.udid];
              const view = status
                ? agentStatusView(status)
                : {
                    label: "Chua kiem tra",
                    tone: "info" as const,
                    textCommentsEnabled: false,
                    message: null,
                  };
              const action = busy[device.udid];
              return (
                <div className="agent-status-row" role="row" key={device.udid}>
                  <span className="agent-device-name" title={device.udid}>
                    <strong>{device.name}</strong>
                    <small>{device.model}</small>
                  </span>
                  <span>
                    <span className={`chip ${view.tone}`} title={view.message ?? undefined}>
                      {view.label}
                    </span>
                  </span>
                  <span className="mono">
                    {status?.installedVersion ?? "-"}
                    {status?.installedBuild ? ` (${status.installedBuild})` : ""}
                  </span>
                  <span>{status?.authReady ? "Yes" : "No"}</span>
                  <span>{status?.mjpegReady ? "Yes" : "No"}</span>
                  <span>{status?.sessionReady ? "Yes" : "No"}</span>
                  <span className="agent-status-actions">
                    <button
                      type="button"
                      className="ghost"
                      disabled={Boolean(action)}
                      onClick={() => void runAgentAction(device.udid, "check")}
                    >
                      {action === "check" ? "Checking..." : "Check"}
                    </button>
                    <button
                      type="button"
                      className="primary"
                      disabled={Boolean(action)}
                      onClick={() => void runAgentAction(device.udid, "repair")}
                    >
                      {action === "repair" ? "Repairing..." : "Repair"}
                    </button>
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </section>

      <section className="settings-section">
        <h3>Desktop bridge</h3>
        <p className="hint">
          Active mode: <code>{mode}</code>. Mock chỉ dùng khi phát triển.
        </p>
      </section>

      <section className="settings-section">
        <h3>Legacy stock agent</h3>
        <p className="hint">
          Apple ID signing chỉ dành cho rollback/debug stock WDA; không phải đường bình luận chữ.
        </p>
        <label>
          Email
          <input value={email} onChange={(event) => setEmail(event.target.value)} />
        </label>
        <label>
          Password {hasPassword ? "(saved)" : ""}
          <input
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            placeholder={hasPassword ? "••••••••" : ""}
          />
        </label>
        {legacyMessage && <p className="hint">{legacyMessage}</p>}
        <div className="row">
          <button
            type="button"
            className="primary"
            onClick={async () => {
              try {
                await setAppleId(email, password);
                setHasPassword(true);
                setPassword("");
                setLegacyMessage("Saved to OS credential store");
              } catch (error) {
                setLegacyMessage(String(error));
              }
            }}
          >
            Save
          </button>
          <button
            type="button"
            className="ghost"
            onClick={async () => {
              try {
                await clearAppleId();
                setEmail("");
                setPassword("");
                setHasPassword(false);
                setLegacyMessage("Cleared");
              } catch (error) {
                setLegacyMessage(String(error));
              }
            }}
          >
            Clear
          </button>
        </div>
      </section>
    </div>
  );
}
