import { useCallback, useEffect, useMemo, useState } from "react";
import {
  authSession,
  bulkResignWda,
  getStreamSettings,
  listenRiviuEvents,
  listDevices,
  listJobs,
  prepareDevice,
  refreshDevices,
  setStreamSettings,
} from "./api";
import { DeviceTile } from "./components/DeviceTile";
import { FilterToolbar, type ViewMode } from "./components/FilterToolbar";
import { FocusStream } from "./components/FocusStream";
import { IconRefresh, IconUser } from "./components/Icons";
import { JobsPanel } from "./components/JobsPanel";
import { NurturePopup } from "./components/NurturePopup";
import { ProfileToolbar } from "./components/ProfileToolbar";
import { ScriptsPanel } from "./components/ScriptsPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { pushFrame } from "./frameStore";
import {
  AccountPage,
  ApiPage,
  AppsPage,
  DataPage,
  LoginPage,
  MaterialPage,
  PublishPage,
  RegisterPage,
  ScheduleBlock,
  SyncPage,
} from "./pages/FarmPages";
import type {
  DeviceInfo,
  JobRecord,
  LocalUser,
  PageId,
  StreamSettings,
} from "./types";
import "./App.css";

const PAGE_TITLE: Partial<Record<PageId, string>> = {
  control: "Quản lý cửa sổ",
  material: "Material",
  apps: "App center",
  scripts: "Automation",
  jobs: "Jobs",
  sync: "Đồng bộ cửa sổ",
  publish: "Publish",
  data: "Data center",
  account: "Account",
  api: "API",
  settings: "Settings",
  login: "Login",
  register: "Register",
};

function App() {
  const [page, setPage] = useState<PageId>("control");
  const [asideCollapsed, setAsideCollapsed] = useState(false);
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [jobs, setJobs] = useState<JobRecord[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [groupMode, setGroupMode] = useState(false);
  const [focusUdid, setFocusUdid] = useState<string | null>(null);
  const [settings, setSettings] = useState<StreamSettings>({
    fps: 24,
    tileSize: "medium",
    gridQuality: "medium",
    focusQuality: "high",
  });
  const [jobsScriptSeed, setJobsScriptSeed] = useState<string | null>(null);
  const [bootError, setBootError] = useState<string | null>(null);
  const [user, setUser] = useState<LocalUser | null>(null);
  const [showAuthUi, setShowAuthUi] = useState(false);
  const [authForced, setAuthForced] = useState(false);
  const [viewMode, setViewMode] = useState<ViewMode>("window");
  const [filterQuery, setFilterQuery] = useState("");
  const [filterConn, setFilterConn] = useState("");
  const [filterStatus, setFilterStatus] = useState("");
  const [nurtureOpen, setNurtureOpen] = useState(false);

  const reload = useCallback(async () => {
    try {
      const [d, j, s] = await Promise.all([
        listDevices(),
        listJobs(),
        getStreamSettings(),
      ]);
      setDevices(d);
      setJobs(j);
      setSettings(s);
      setBootError(null);
    } catch (e) {
      setBootError(String(e));
    }
  }, []);

  useEffect(() => {
    authSession()
      .then((s) => {
        setShowAuthUi(s.showAuthUi);
        setUser(s.user ?? null);
        if (s.showAuthUi && !s.bypassed) {
          setPage("login");
        }
      })
      .catch(() => undefined);
    reload();
    let unlisten: (() => void) | undefined;
    listenRiviuEvents((payload) => {
      const p = payload as Record<string, unknown>;
      if (p.type === "devicesUpdated" && Array.isArray(p.devices)) {
        setDevices(p.devices as DeviceInfo[]);
      }
      if (p.type === "deviceUpdated" && p.device) {
        const device = p.device as DeviceInfo;
        setDevices((prev) => {
          const idx = prev.findIndex((d) => d.udid === device.udid);
          if (idx === -1) return [...prev, device];
          const next = [...prev];
          next[idx] = device;
          return next;
        });
      }
      if (p.type === "jobUpdated" && p.job) {
        const job = p.job as JobRecord;
        setJobs((prev) => {
          const idx = prev.findIndex((j) => j.id === job.id);
          if (idx === -1) return [job, ...prev];
          const next = [...prev];
          next[idx] = job;
          return next;
        });
      }
      if (p.type === "streamFrame" && typeof p.udid === "string") {
        pushFrame(p.udid as string, String(p.jpegBase64 ?? ""));
      }
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [reload]);

  const focusDevice = useMemo(
    () => devices.find((d) => d.udid === focusUdid) ?? null,
    [devices, focusUdid],
  );

  const readyCount = useMemo(
    () => devices.filter((d) => d.wdaReady || d.status === "ready").length,
    [devices],
  );

  const runningJobs = useMemo(
    () => jobs.filter((j) => j.status === "running" || j.status === "queued").length,
    [jobs],
  );

  const filtered = useMemo(() => {
    const q = filterQuery.trim().toLowerCase();
    return devices.filter((d) => {
      if (filterConn && d.connection !== filterConn) return false;
      if (filterStatus === "ready") {
        if (!(d.wdaReady || d.status === "ready")) return false;
      } else if (filterStatus && d.status !== filterStatus) {
        return false;
      }
      if (!q) return true;
      return (
        d.name.toLowerCase().includes(q) ||
        d.udid.toLowerCase().includes(q) ||
        d.model.toLowerCase().includes(q)
      );
    });
  }, [devices, filterQuery, filterConn, filterStatus]);

  const selectedDevices = useMemo(
    () => devices.filter((d) => selected.includes(d.udid)),
    [devices, selected],
  );

  const onSelect = (udid: string, additive: boolean) => {
    setSelected((prev) => {
      if (additive) {
        return prev.includes(udid) ? prev.filter((x) => x !== udid) : [...prev, udid];
      }
      return prev.includes(udid) && prev.length === 1 ? [] : [udid];
    });
  };

  const authBlocking = (showAuthUi || authForced) && (page === "login" || page === "register");
  const title = PAGE_TITLE[page] ?? page;

  if (authBlocking && page === "login") {
    return (
      <LoginPage
        onDone={(u) => {
          setUser(u);
          setAuthForced(false);
          setPage("control");
        }}
        onRegister={() => setPage("register")}
      />
    );
  }
  if (authBlocking && page === "register") {
    return (
      <RegisterPage
        onDone={(u) => {
          setUser(u);
          setAuthForced(false);
          setPage("control");
        }}
        onLogin={() => setPage("login")}
      />
    );
  }

  return (
    <div className="shell">
      <Sidebar
        page={page}
        collapsed={asideCollapsed}
        selectedCount={selected.length}
        total={devices.length}
        readyCount={readyCount}
        groupMode={groupMode}
        onPage={setPage}
        onToggleCollapse={() => setAsideCollapsed((v) => !v)}
      />

      <div className="main-col">
        <header className="topbar">
          <div className="topbar-title">{title}</div>
          <div className="topbar-drag" />
          <div className="topbar-actions">
            {groupMode && <span className="chip primary">Sync</span>}
            <span className="chip info">{settings.fps} FPS</span>
            {runningJobs > 0 && <span className="chip warn">{runningJobs} job</span>}
            <button
              type="button"
              className="icon-btn"
              title="Refresh"
              onClick={async () => {
                await refreshDevices();
                await reload();
              }}
            >
              <IconRefresh size={16} />
            </button>
            <button type="button" className="icon-btn" title={user?.email ?? "guest"}>
              <IconUser size={18} />
            </button>
          </div>
        </header>

        <div className="content">
          {bootError && (
            <div className="banner">
              Backend chưa sẵn sàng ({bootError}). Bấm Refresh.
            </div>
          )}

          {page === "control" && (
            <>
              <ProfileToolbar
                selected={selectedDevices}
                syncOn={groupMode}
                nurtureOpen={nurtureOpen}
                onNurture={() => setNurtureOpen((v) => !v)}
                onStart={async () => {
                  const targets = selected.length
                    ? selected
                    : filtered.map((d) => d.udid);
                  if (!targets.length) {
                    window.alert("Chưa có thiết bị");
                    return;
                  }
                  for (const u of targets) await prepareDevice(u);
                  await reload();
                  window.alert(`Start/Prepare: ${targets.length} máy`);
                }}
                onStop={() => setSelected([])}
                onInstall={async () => {
                  const targets = selected.length
                    ? selected
                    : devices.map((d) => d.udid);
                  if (!targets.length) {
                    window.alert("Chưa có thiết bị");
                    return;
                  }
                  const results = await bulkResignWda(targets);
                  await reload();
                  window.alert(results.join("\n") || `Agent: ${targets.length} máy`);
                }}
                onSync={() => setGroupMode((v) => !v)}
                onRefresh={async () => {
                  await refreshDevices();
                  await reload();
                }}
              />

              <FilterToolbar
                query={filterQuery}
                connection={filterConn}
                status={filterStatus}
                viewMode={viewMode}
                tileSize={settings.tileSize}
                onQuery={setFilterQuery}
                onConnection={setFilterConn}
                onStatus={setFilterStatus}
                onViewMode={setViewMode}
                onTileSize={async (v) => {
                  const saved = await setStreamSettings({
                    ...settings,
                    tileSize: v as StreamSettings["tileSize"],
                  });
                  setSettings(saved);
                }}
              />

              {!!devices.length && !devices.some((d) => d.wdaReady) && (
                <div className="banner">
                  Stream cần WDA. Chọn máy rồi bấm <strong>Start</strong> hoặc{" "}
                  <strong>Agent</strong>. Bấm vào màn hình nhỏ để phóng to.
                </div>
              )}

              {viewMode === "list" && (
                <table className="device-table">
                  <thead>
                    <tr>
                      <th />
                      <th>Name</th>
                      <th>Status</th>
                      <th>Code</th>
                      <th>Model</th>
                      <th>Link</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {filtered.map((device) => {
                      const sel = selected.includes(device.udid);
                      const running = device.wdaReady || device.status === "ready";
                      return (
                        <tr
                          key={device.udid}
                          className={sel ? "selected" : ""}
                          onClick={(e) => onSelect(device.udid, e.metaKey || e.ctrlKey)}
                          onDoubleClick={() => setFocusUdid(device.udid)}
                        >
                          <td>
                            <input
                              type="checkbox"
                              checked={sel}
                              onChange={() => onSelect(device.udid, true)}
                              onClick={(e) => e.stopPropagation()}
                            />
                          </td>
                          <td>{device.name}</td>
                          <td>
                            <span className={`chip ${running ? "ok" : "info"}`}>
                              {running ? "Running" : device.status}
                            </span>
                          </td>
                          <td className="mono">{device.udid.slice(0, 12)}…</td>
                          <td>
                            {device.model} · {device.iosVersion}
                          </td>
                          <td>{device.connection.toUpperCase()}</td>
                          <td>
                            <button
                              type="button"
                              className="link"
                              onClick={(e) => {
                                e.stopPropagation();
                                setFocusUdid(device.udid);
                              }}
                            >
                              Open
                            </button>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              )}

              {viewMode === "window" && (
                <div className="window-canvas">
                  {filtered.map((device) => (
                    <DeviceTile
                      key={device.udid}
                      device={device}
                      tileSize={settings.tileSize}
                      selected={selected.includes(device.udid)}
                      focused={focusUdid === device.udid}
                      onSelect={onSelect}
                      onOpen={setFocusUdid}
                      groupUdids={selected}
                      groupMode={groupMode}
                      onPrepare={(udid) => prepareDevice(udid).then(reload)}
                    />
                  ))}
                </div>
              )}

              {!devices.length && (
                <div className="empty-state">
                  <h2>Chưa có iPhone</h2>
                  <p className="hint">Cắm USB, Trust, rồi Refresh.</p>
                  <button
                    type="button"
                    className="primary"
                    onClick={async () => {
                      await refreshDevices();
                      await reload();
                    }}
                  >
                    Refresh devices
                  </button>
                </div>
              )}
            </>
          )}

          {page === "material" && (
            <MaterialPage
              devices={devices}
              selected={selected}
              onSelectUdids={setSelected}
            />
          )}
          {page === "apps" && (
            <AppsPage
              devices={devices}
              selected={selected}
              onSelectUdids={setSelected}
            />
          )}
          {page === "scripts" && (
            <div>
              <ScriptsPanel
                onUseInJobs={(json) => {
                  setJobsScriptSeed(json);
                  setPage("jobs");
                }}
              />
              <div className="panel" style={{ marginTop: 12 }}>
                <ScheduleBlock
                  devices={devices}
                  selected={selected}
                  onSelectUdids={setSelected}
                />
              </div>
            </div>
          )}
          {page === "jobs" && (
            <JobsPanel
              jobs={jobs}
              devices={devices}
              selectedUdids={selected}
              onSelectUdids={setSelected}
              onRefresh={reload}
              initialScript={jobsScriptSeed}
            />
          )}
          {page === "sync" && (
            <SyncPage
              devices={devices}
              selected={selected}
              groupMode={groupMode}
              onToggleGroup={() => setGroupMode((v) => !v)}
              onSelect={onSelect}
              onSelectUdids={setSelected}
            />
          )}
          {page === "publish" && (
            <PublishPage
              devices={devices}
              selected={selected}
              onSelectUdids={setSelected}
            />
          )}
          {page === "data" && <DataPage />}
          {page === "account" && (
            <AccountPage
              user={user}
              onShowAuth={() => {
                setAuthForced(true);
                setPage("login");
              }}
            />
          )}
          {page === "api" && <ApiPage />}
          {page === "settings" && <SettingsPanel />}
        </div>
      </div>

      {focusDevice && (
        <FocusStream
          device={focusDevice}
          onClose={() => setFocusUdid(null)}
          groupUdids={selected}
          groupMode={groupMode}
        />
      )}

      {page === "control" && nurtureOpen && (
        <NurturePopup
          devices={devices}
          selected={selected}
          onClose={() => setNurtureOpen(false)}
        />
      )}
    </div>
  );
}

export default App;
