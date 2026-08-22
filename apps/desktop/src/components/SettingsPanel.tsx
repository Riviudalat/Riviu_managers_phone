import { useEffect, useMemo, useState } from "react";
import {
  agentGetSettings,
  agentListStatuses,
  agentPreflight,
  agentRepair,
  agentSaveSettings,
  arpScan,
  clearAppleId,
  driverMode,
  getAppleId,
  getStreamSettings,
  localApiGetConfig,
  localApiSetConfig,
  setAppleId,
  setStreamSettings,
  updateCheck,
  updateInstall,
  wifiAdbConnect,
  type ArpEntry,
  type LocalApiConfig,
} from "../api";
import { agentStatusView } from "../agentStatus";
import { describeError } from "../toastStore";
import { updateView } from "../updateView";
import { setGroupSync, useGroupSync } from "../groupSync";
import { EmptyState } from "./States";
import { IconPhone } from "./Icons";
import type {
  AgentRuntimeView,
  AgentStatus,
  DelayPolicy,
  DeviceInfo,
  StreamSettings,
  UpdateStatus,
} from "../types";

interface Props {
  devices: DeviceInfo[];
}

type AgentAction = "check" | "repair";

/// Pinned to `MIN_VIEW_FPS` and `MAX_SETTABLE_VIEW_FPS` on the Rust side by
/// `the_fps_field_offers_exactly_the_range_this_file_clamps_to` in `commands.rs`, which
/// reads these two lines. Change one and that test names the other. Rust clamps regardless — these only stop the field from displaying a number the encoder will
/// never run at while the operator waits to see it take effect.
const MIN_STREAM_FPS = 5;
const MAX_STREAM_FPS = 30;
/// Display only, and it must match `ViewPreset::Tile::max_fps()` in `scrcpy.rs` — the cap
/// is enforced there, not here. It is named in the hint because a field labelled "FPS"
/// that only half the picture obeys is the same disagreement the overlay/encoder mismatch
/// already cost us once.
const TILE_FPS_CEILING = 10;

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
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [installingUpdate, setInstallingUpdate] = useState(false);
  const [streamSettings, setStreamSettingsState] = useState<StreamSettings | null>(null);
  const [savingStream, setSavingStream] = useState(false);
  const [streamMessage, setStreamMessage] = useState<string | null>(null);
  const groupSync = useGroupSync();
  // Normalised locals so the union narrows cleanly in JSX (the store always stores concrete
  // values; the type keeps the fields optional for forward-compat).
  const gsDelay: DelayPolicy = groupSync.delay ?? { mode: "none" };
  const gsMaxPx = groupSync.offset?.maxPx ?? 0;
  const [wifiHost, setWifiHost] = useState("");
  const [arp, setArp] = useState<ArpEntry[]>([]);
  const [arpBusy, setArpBusy] = useState(false);
  const [wifiMessage, setWifiMessage] = useState<string | null>(null);
  const [localApi, setLocalApi] = useState<LocalApiConfig | null>(null);
  const [savingApi, setSavingApi] = useState(false);
  const [apiMessage, setApiMessage] = useState<string | null>(null);

  const connectWifi = async (host: string) => {
    const target = host.includes(":") ? host : `${host}:5555`;
    try {
      await wifiAdbConnect(target);
      setWifiMessage(`Đã kết nối ${target}. Bấm "Làm mới" ở Quản lý cửa sổ để thấy máy.`);
    } catch (error) {
      setWifiMessage(describeError(error));
    }
  };

  const scanArp = async () => {
    setArpBusy(true);
    setWifiMessage(null);
    try {
      setArp(await arpScan());
    } catch (error) {
      setWifiMessage(describeError(error));
    } finally {
      setArpBusy(false);
    }
  };

  /// Send the whole row, not the one field that changed: the command takes a complete
  /// `StreamSettings` and a partial one would reset the fields it omitted to their defaults.
  const saveStream = async (change: Partial<StreamSettings>) => {
    if (!streamSettings) return;
    setSavingStream(true);
    setStreamMessage(null);
    try {
      // The reply is the clamped value Rust actually stored, so the field shows what took
      // effect rather than what was typed.
      setStreamSettingsState(await setStreamSettings({ ...streamSettings, ...change }));
    } catch (error) {
      setStreamMessage(describeError(error));
    } finally {
      setSavingStream(false);
    }
  };

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
      .catch((error) => setAgentMessage(describeError(error)));
    getAppleId()
      .then((config) => {
        setEmail(config.email);
        setHasPassword(config.hasPassword);
      })
      .catch(() => undefined);
    driverMode().then(setMode).catch(() => setMode("unknown"));
    getStreamSettings()
      .then(setStreamSettingsState)
      .catch((error) => setStreamMessage(describeError(error)));
    localApiGetConfig()
      .then(setLocalApi)
      .catch((error) => setApiMessage(describeError(error)));
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
      .catch((error) => setAgentMessage(describeError(error)));
  }, [connectedUdids]);

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

  const updateStatusView = updateView(update, updateError, installingUpdate);

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
                setAgentMessage(describeError(error));
              } finally {
                setSavingSettings(false);
              }
            }}
          />
          Auto repair
        </label>

        {agentMessage && <p className="error">{agentMessage}</p>}

        {!connectedDevices.length ? (
          <EmptyState
            compact
            icon={<IconPhone size={15} />}
            title="Chưa có điện thoại đang kết nối"
            hint="Cắm máy qua USB rồi làm mới ở Quản lý cửa sổ."
          />
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

      {/* The row this panel was missing. `StreamSettings` has been in the Rust command
          surface the whole time with nothing on the frontend calling it, so quality and
          frame rate were unreachable — which is also why "they are lost on restart" was
          only half the story. */}
      <section className="settings-section">
        <h3>Chất lượng stream</h3>
        <p className="hint">
          Áp cho Android. Lưới và overlay mã hoá riêng — overlay là một máy chiếm cả cửa sổ
          nên để cao hơn được. Đổi xong sẽ khởi động lại các tile đang chạy, mất khoảng một
          giây hình đen mỗi máy.
        </p>
        <p className="hint">
          FPS ở đây là của overlay. Tile trong lưới bị chặn ở {TILE_FPS_CEILING} hình/giây:
          hai mươi tile giải mã cùng một chỗ với overlay, và đo trên dàn máy này thì 24
          hình/giây tốn 135% một nhân CPU, còn 5 hình/giây tốn 85%. Chặn tile lại là để
          máy đang điều khiển được mượt.
        </p>
        <div className="row">
          <label>
            Chất lượng lưới
            <select
              value={streamSettings?.gridQuality ?? "medium"}
              disabled={!streamSettings || savingStream}
              onChange={(event) => {
                void saveStream({
                  gridQuality: event.target.value as StreamSettings["gridQuality"],
                });
              }}
            >
              <option value="low">Thấp</option>
              <option value="medium">Vừa</option>
              <option value="high">Cao</option>
              <option value="extra">Rất cao</option>
            </select>
          </label>
          <label>
            Chất lượng overlay
            <select
              value={streamSettings?.focusQuality ?? "high"}
              disabled={!streamSettings || savingStream}
              onChange={(event) => {
                void saveStream({
                  focusQuality: event.target.value as StreamSettings["focusQuality"],
                });
              }}
            >
              <option value="low">Thấp</option>
              <option value="medium">Vừa</option>
              <option value="high">Cao</option>
              <option value="extra">Rất cao</option>
            </select>
          </label>
          <label>
            FPS overlay
            <input
              type="number"
              min={MIN_STREAM_FPS}
              max={MAX_STREAM_FPS}
              value={streamSettings?.fps ?? MAX_STREAM_FPS}
              disabled={!streamSettings || savingStream}
              onChange={(event) => {
                const fps = Number(event.target.value);
                if (!Number.isFinite(fps)) return;
                // Clamped here as well as in Rust, so the field cannot show a number the
                // encoder will never run at while the operator waits for it to take effect.
                void saveStream({
                  fps: Math.min(Math.max(Math.round(fps), MIN_STREAM_FPS), MAX_STREAM_FPS),
                });
              }}
            />
          </label>
        </div>
        {streamMessage && <p className="error">{streamMessage}</p>}
      </section>

      <section className="settings-section">
        <h3>Đồng bộ nhóm (Delay &amp; Offset)</h3>
        <p className="hint">
          Khi một thao tác (chạm/vuốt/gõ/phím) phát ra cả nhóm máy, thêm độ trễ và lệch toạ độ
          ngẫu nhiên cho từng máy để cả nhóm không bấm y hệt cùng lúc, cùng chỗ. Tắt cả hai =
          phát đồng loạt như cũ. Chỉ áp cho điều khiển nhóm (≥2 máy), không áp khi điều khiển
          một máy.
        </p>
        <div className="row">
          <label>
            Độ trễ mỗi máy
            <select
              value={gsDelay.mode}
              onChange={(event) => {
                const mode = event.target.value;
                if (mode === "random") {
                  setGroupSync({
                    ...groupSync,
                    delay: { mode: "random", minMs: 200, maxMs: 800 },
                  });
                } else if (mode === "staggered") {
                  setGroupSync({ ...groupSync, delay: { mode: "staggered", stepMs: 150 } });
                } else {
                  setGroupSync({ ...groupSync, delay: { mode: "none" } });
                }
              }}
            >
              <option value="none">Tắt</option>
              <option value="random">Ngẫu nhiên</option>
              <option value="staggered">So le theo thứ tự</option>
            </select>
          </label>
          {gsDelay.mode === "random" && (
            <>
              <label>
                Tối thiểu (ms)
                <input
                  type="number"
                  min={0}
                  value={gsDelay.minMs}
                  onChange={(event) => {
                    const v = Math.max(0, Math.round(Number(event.target.value) || 0));
                    setGroupSync({
                      ...groupSync,
                      delay: { mode: "random", minMs: v, maxMs: gsDelay.maxMs },
                    });
                  }}
                />
              </label>
              <label>
                Tối đa (ms)
                <input
                  type="number"
                  min={0}
                  value={gsDelay.maxMs}
                  onChange={(event) => {
                    const v = Math.max(0, Math.round(Number(event.target.value) || 0));
                    setGroupSync({
                      ...groupSync,
                      delay: { mode: "random", minMs: gsDelay.minMs, maxMs: v },
                    });
                  }}
                />
              </label>
            </>
          )}
          {gsDelay.mode === "staggered" && (
            <label>
              Bước (ms mỗi máy)
              <input
                type="number"
                min={0}
                value={gsDelay.stepMs}
                onChange={(event) => {
                  const v = Math.max(0, Math.round(Number(event.target.value) || 0));
                  setGroupSync({ ...groupSync, delay: { mode: "staggered", stepMs: v } });
                }}
              />
            </label>
          )}
          <label>
            Lệch toạ độ (± px)
            <input
              type="number"
              min={0}
              value={gsMaxPx}
              onChange={(event) => {
                const v = Math.max(0, Math.round(Number(event.target.value) || 0));
                setGroupSync({ ...groupSync, offset: { maxPx: v } });
              }}
            />
          </label>
        </div>
      </section>

      <section className="settings-section">
        <h3>Kết nối không dây (WIFI adb)</h3>
        <p className="hint">
          Kết nối điện thoại Android qua Wi-Fi thay vì cáp. Bật từ máy đang cắm USB bằng
          menu chuột phải → "Chuyển sang WIFI", hoặc nhập trực tiếp host bên dưới. Máy phải
          cùng mạng LAN với PC.
        </p>
        <div className="row">
          <label style={{ flex: 1 }}>
            Host (ip hoặc ip:cổng)
            <input
              type="text"
              placeholder="192.168.1.42 hoặc 192.168.1.42:5555"
              value={wifiHost}
              onChange={(event) => setWifiHost(event.target.value)}
            />
          </label>
          <button
            type="button"
            className="ghost"
            disabled={!wifiHost.trim()}
            onClick={() => void connectWifi(wifiHost.trim())}
          >
            Kết nối
          </button>
          <button type="button" className="ghost" disabled={arpBusy} onClick={() => void scanArp()}>
            {arpBusy ? "Đang quét…" : "Quét mạng (ARP)"}
          </button>
        </div>
        {arp.length > 0 && (
          <div className="group-tools-preview" style={{ marginTop: "0.4rem" }}>
            {arp.map((entry) => (
              <div className="row-item" key={entry.ip}>
                <span className="who mono">{entry.ip}</span>
                <span className="what mono">{entry.mac}</span>
                <span className="grow" />
                <button type="button" className="ghost" onClick={() => void connectWifi(entry.ip)}>
                  Kết nối
                </button>
              </div>
            ))}
          </div>
        )}
        {wifiMessage && <p className="hint">{wifiMessage}</p>}
      </section>

      <section className="settings-section">
        <h3>API tự động hoá cục bộ (openapi)</h3>
        <p className="hint">
          Máy chủ HTTP chỉ chạy trên loopback (127.0.0.1) để script bên ngoài điều khiển fleet
          — bật/tắt màn, chạm, vuốt, gõ, phím. Mặc định TẮT, luôn cần token Bearer. Đổi cấu
          hình có hiệu lực sau khi khởi động lại ứng dụng.
        </p>
        {localApi && (
          <>
            <label className="agent-toggle" style={{ marginBottom: "0.5rem" }}>
              <input
                type="checkbox"
                checked={localApi.enabled}
                onChange={(event) => setLocalApi({ ...localApi, enabled: event.target.checked })}
              />
              Bật API cục bộ
            </label>
            <div className="row">
              <label>
                Cổng
                <input
                  type="number"
                  min={1}
                  max={65535}
                  value={localApi.port}
                  onChange={(event) =>
                    setLocalApi({ ...localApi, port: Number(event.target.value) || 0 })
                  }
                  style={{ width: "8rem" }}
                />
              </label>
              <label style={{ flex: 1 }}>
                Token (Bearer)
                <input type="text" readOnly value={localApi.token || "(tạo khi lưu)"} className="mono" />
              </label>
              <button
                type="button"
                className="ghost"
                onClick={() => setLocalApi({ ...localApi, token: "" })}
                title="Xoá token hiện tại; lưu sẽ tạo token mới"
              >
                Tạo token mới
              </button>
            </div>
            <div className="row" style={{ marginTop: "0.5rem" }}>
              <button
                type="button"
                className="primary"
                disabled={savingApi}
                onClick={async () => {
                  setSavingApi(true);
                  setApiMessage(null);
                  try {
                    const saved = await localApiSetConfig(localApi);
                    setLocalApi(saved);
                    setApiMessage(
                      saved.enabled
                        ? `Đã lưu. API sẽ chạy ở 127.0.0.1:${saved.port} sau khi khởi động lại ứng dụng.`
                        : "Đã lưu (API đang tắt).",
                    );
                  } catch (error) {
                    setApiMessage(describeError(error));
                  } finally {
                    setSavingApi(false);
                  }
                }}
              >
                {savingApi ? "Đang lưu…" : "Lưu"}
              </button>
            </div>
            {localApi.token && (
              <pre className="group-tools-log" style={{ marginTop: "0.5rem" }}>
                {`# ví dụ: liệt kê máy\ncurl -H "Authorization: Bearer ${localApi.token}" http://127.0.0.1:${localApi.port}/v1/devices\n\n# chạm toạ độ trên một máy\ncurl -X POST -H "Authorization: Bearer ${localApi.token}" \\\n  -d '{"udid":"<udid>","x":540,"y":1200}' http://127.0.0.1:${localApi.port}/v1/tap`}
              </pre>
            )}
          </>
        )}
        {apiMessage && <p className="hint">{apiMessage}</p>}
      </section>

      <section className="settings-section">
        <h3>Bản cập nhật</h3>
        <p>
          <span className={`chip ${updateStatusView.tone}`}>{updateStatusView.headline}</span>
        </p>
        {updateStatusView.detail && <p className="hint">{updateStatusView.detail}</p>}
        <div className="row">
          <button
            type="button"
            className="ghost"
            disabled={checkingUpdate || installingUpdate}
            onClick={async () => {
              setCheckingUpdate(true);
              setUpdateError(null);
              try {
                setUpdate(await updateCheck());
              } catch (error) {
                setUpdate(null);
                setUpdateError(describeError(error));
              } finally {
                setCheckingUpdate(false);
              }
            }}
          >
            {checkingUpdate ? "Đang kiểm..." : "Kiểm bản mới"}
          </button>
          <button
            type="button"
            className="primary"
            disabled={!updateStatusView.canInstall}
            onClick={async () => {
              setInstallingUpdate(true);
              setUpdateError(null);
              try {
                await updateInstall();
                // Reached on macOS only: the archive is unpacked in place and the app has
                // to be reopened. On Windows the process is gone before this line.
                setUpdateError("Đã cài xong — mở lại app để dùng bản mới.");
              } catch (error) {
                setUpdateError(describeError(error));
              } finally {
                setInstallingUpdate(false);
              }
            }}
          >
            Tải và cài đặt
          </button>
        </div>
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
                setLegacyMessage(describeError(error));
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
                setLegacyMessage(describeError(error));
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
