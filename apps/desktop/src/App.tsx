import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  agentBulkRepair,
  agentListStatuses,
  authSession,
  androidUnavailableReason,
  driverDegradedReason,
  listenRiviuEvents,
  installIpa,
  listDevices,
  listGroups,
  listJobs,
  rebootDevice,
  refreshDevices,
  saveGroup,
  screenshot,
  setScreenRotation,
  startupError,
} from "./api";
import { startDevicePreview, startFleetPreview } from "./startPreview";
import { summarizeBulkRepair } from "./agentStatus";
import { requestConfirm } from "./confirmStore";
import { pushToast, toastError } from "./toastStore";
import { ConfirmHost } from "./components/ConfirmHost";
import { ToastHost } from "./components/ToastHost";
import { DeviceTile } from "./components/DeviceTile";
import { FilterToolbar, type ViewMode } from "./components/FilterToolbar";
import { GroupTabs } from "./components/GroupTabs";
import { DeviceContextMenu, type DeviceMenuAction } from "./components/DeviceContextMenu";
import { AdbConsole } from "./components/AdbConsole";
import { ALL_DEVICES_TAB, devicesInTab, groupTabs, withDeviceAdded } from "./deviceGroups";
import { FocusStream } from "./components/FocusStream";
import { IconPhone, IconRefresh, IconUser } from "./components/Icons";
import { Banner, EmptyState, LoadingState } from "./components/States";
import { InteractionPopup } from "./components/InteractionPopup";
import { JobsPanel } from "./components/JobsPanel";
import { NurturePopup } from "./components/NurturePopup";
import { ProfileToolbar } from "./components/ProfileToolbar";
import { ScriptsPanel } from "./components/ScriptsPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { useViewClient } from "./viewStore";
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
  DeviceGroup,
  DeviceInfo,
  JobRecord,
  LocalUser,
  PageId,
} from "./types";
import { deviceModelOsLabel, markDeviceFrameLive } from "./types";
import { pickFile } from "./pickFile";
import { loadZoom, stepZoom, storeZoom, TILE_ZOOM, wheelWantsZoom } from "./zoom";
import "./App.css";

const FlowWorkspace = lazy(async () => {
  const module = await import("./components/flow/FlowWorkspace");
  return { default: module.FlowWorkspace };
});

const PAGE_TITLE: Partial<Record<PageId, string>> = {
  control: "Quản lý cửa sổ",
  material: "Kho nội dung",
  apps: "Trung tâm ứng dụng",
  scripts: "Flow",
  jobs: "Tác vụ",
  sync: "Đồng bộ cửa sổ",
  publish: "Đăng bài",
  data: "Dữ liệu",
  account: "Tài khoản",
  api: "API",
  settings: "Cài đặt",
  login: "Đăng nhập",
  register: "Đăng ký",
};

function App() {
  const [page, setPage] = useState<PageId>("control");
  const [asideCollapsed, setAsideCollapsed] = useState(false);
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  const [groupTab, setGroupTab] = useState<string>(ALL_DEVICES_TAB);
  const [tileMenu, setTileMenu] = useState<{ udid: string; x: number; y: number } | null>(null);
  const [adbFor, setAdbFor] = useState<string | null>(null);
  const [jobs, setJobs] = useState<JobRecord[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [groupMode, setGroupMode] = useState(false);
  const [focusUdid, setFocusUdid] = useState<string | null>(null);
  const [jobsScriptSeed, setJobsScriptSeed] = useState<string | null>(null);
  const [bootError, setBootError] = useState<string | null>(null);
  const [driverIssue, setDriverIssue] = useState<string | null>(null);
  const [androidIssue, setAndroidIssue] = useState<string | null>(null);
  const [startupIssue, setStartupIssue] = useState<string | null | undefined>(undefined);
  const [user, setUser] = useState<LocalUser | null>(null);
  const [showAuthUi, setShowAuthUi] = useState(false);
  const [authForced, setAuthForced] = useState(false);
  const [viewMode, setViewMode] = useState<ViewMode>("window");
  const [tileWidth, setTileWidth] = useState(() => loadZoom(TILE_ZOOM));
  const canvasRef = useRef<HTMLDivElement | null>(null);
  const [nurtureOpen, setNurtureOpen] = useState(false);
  const [interactionOpen, setInteractionOpen] = useState(false);
  const [flowDirty, setFlowDirty] = useState(false);
  const [automationView, setAutomationView] = useState<"flow" | "legacy">("flow");
  useViewClient();

  const confirmDiscardFlow = useCallback(
    () =>
      requestConfirm({
        title: "Bỏ thay đổi Flow chưa lưu?",
        message: "Bản nháp hiện tại chưa được lưu và sẽ mất khi rời khỏi trang.",
        confirmLabel: "Bỏ thay đổi",
        cancelLabel: "Ở lại",
        danger: true,
      }),
    [],
  );

  const requestPage = useCallback(
    async (next: PageId) => {
      if (next === page) return;
      if (flowDirty && !(await confirmDiscardFlow())) return;
      setPage(next);
    },
    [confirmDiscardFlow, flowDirty, page],
  );

  const requestAutomationView = useCallback(
    async (next: "flow" | "legacy") => {
      if (next === automationView) return;
      if (flowDirty && !(await confirmDiscardFlow())) return;
      setAutomationView(next);
    },
    [automationView, confirmDiscardFlow, flowDirty],
  );

  useEffect(() => {
    if (!flowDirty) return;
    const preventUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
    };
    window.addEventListener("beforeunload", preventUnload);
    return () => window.removeEventListener("beforeunload", preventUnload);
  }, [flowDirty]);

  useEffect(() => {
    storeZoom(TILE_ZOOM, tileWidth);
  }, [tileWidth]);

  // Wheel over the phone grid zooms the tiles. Registered by hand because
  // React's synthetic onWheel is passive and cannot preventDefault the page
  // scroll. Re-runs when the canvas mounts (control page, window view).
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const onWheel = (event: WheelEvent) => {
      if (!wheelWantsZoom(event)) return;
      event.preventDefault();
      setTileWidth((width) => stepZoom(TILE_ZOOM, width, event.deltaY));
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [page, viewMode]);

  const tabs = useMemo(() => groupTabs(devices, groups), [devices, groups]);
  const visibleDevices = useMemo(
    () => devicesInTab(devices, groups, groupTab),
    [devices, groups, groupTab],
  );
  const menuAdbDevice = useMemo(
    () => (adbFor ? (devices.find((d) => d.udid === adbFor) ?? null) : null),
    [adbFor, devices],
  );
  const menuDevice = useMemo(
    () => (tileMenu ? (devices.find((d) => d.udid === tileMenu.udid) ?? null) : null),
    [tileMenu, devices],
  );

  const reload = useCallback(async () => {
    try {
      const [d, j] = await Promise.all([listDevices(), listJobs()]);
      setDevices(d);
      setJobs(j);
      // Groups are auxiliary and load separately, on purpose. Inside the Promise.all
      // above, a group-listing failure rejected the whole reload and left the grid empty
      // — the fleet blanked because a tab strip could not be drawn. Caught by e2e, which
      // had no handler registered for it. Losing the tabs is a smaller loss than losing
      // every phone, so this failure degrades to "no groups".
      setGroups(await listGroups().catch(() => []));
      setBootError(null);
      // An empty list can mean "nothing plugged in" or "the device sidecar never
      // started". Ask which, so the UI does not report the wrong one.
      setDriverIssue(await driverDegradedReason().catch(() => null));
      // Asked separately, because the two halves of the fleet fail for different
      // reasons and an Android phone that never appears used to say nothing at all.
      setAndroidIssue(await androidUnavailableReason().catch(() => null));
    } catch (e) {
      setBootError(String(e));
    }
  }, []);

  /**
   * Tile menu rows, and every one of them is a command this app already has.
   *
   * The reference product also offers an adb command box, rotate, wallpaper, APK
   * install and device deletion. They are absent on purpose: a row that calls a
   * command we never wrote is a button that fails, which is worse than its absence.
   */
  const tileActions = useCallback(
    (device: DeviceInfo): DeviceMenuAction[] => [
      {
        id: "open",
        label: "Mở điều khiển",
        run: () => setFocusUdid(device.udid),
      },
      {
        id: "screenshot",
        label: "Chụp màn hình",
        run: () => {
          void screenshot(device.udid)
            .then((path) => pushToast("ok", "Đã lưu ảnh", path))
            .catch((error) => toastError("Chụp màn hình thất bại", error));
        },
      },
      {
        id: "copy",
        label: "Sao chép ID máy",
        run: () => {
          void navigator.clipboard
            .writeText(device.udid)
            .then(() => pushToast("ok", "Đã sao chép ID máy"))
            .catch((error) => toastError("Sao chép thất bại", error));
        },
      },
      {
        id: "reload",
        label: "Làm mới danh sách",
        run: () => {
          void refreshDevices().then(reload).catch((error) => toastError("Làm mới thất bại", error));
        },
      },
      ...(device.platform === "android"
        ? [
            {
              id: "rotate",
              label: "Quay màn hình",
              run: () => {
                // The backend returns the rotation the phone actually settled at, which
                // is often not the one asked for: a portrait-locked app wins, and on
                // this farm that is TikTok. Saying "rotated" regardless would be the
                // button that lies.
                void setScreenRotation(device.udid, 1)
                  .then((observed) => {
                    if (observed === 1) {
                      pushToast("ok", "Đã quay ngang");
                    } else {
                      pushToast(
                        "warn",
                        "Máy không quay",
                        "App đang mở khoá hướng dọc nên hệ thống bỏ qua yêu cầu.",
                      );
                    }
                  })
                  .catch((error) => toastError("Quay màn hình thất bại", error));
              },
            },
            {
              id: "apk",
              label: "Cài APK...",
              run: () => {
                void (async () => {
                  const path = await pickFile({
                    title: "Chọn APK",
                    filters: [{ name: "APK", extensions: ["apk"] }],
                  });
                  if (!path) return;
                  try {
                    // Same command the iOS path uses; the driver behind it runs
                    // `adb install -r -g` for an Android serial.
                    await installIpa(device.udid, path);
                    pushToast("ok", "Đã cài APK");
                  } catch (error) {
                    toastError("Cài APK thất bại", error);
                  }
                })();
              },
            },
            {
              id: "adb",
              label: "Lệnh adb...",
              run: () => setAdbFor(device.udid),
            },
          ]
        : []),
      {
        id: "reboot",
        label: "Khởi động lại máy",
        danger: true,
        run: () => {
          void requestConfirm({
            title: `Khởi động lại ${device.name}?`,
            message: "Máy sẽ mất kết nối vài phút và mọi phiên đang chạy trên nó sẽ dừng.",
            confirmLabel: "Khởi động lại",
            danger: true,
          }).then((ok) => {
            if (!ok) return;
            void rebootDevice(device.udid)
              .then(() => pushToast("ok", "Đã gửi lệnh khởi động lại"))
              .catch((error) => toastError("Khởi động lại thất bại", error));
          });
        },
      },
    ],
    [reload],
  );


  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    startupError()
      .then((issue) => {
        if (cancelled) return;
        setStartupIssue(issue);
        if (issue) return;

        void authSession()
          .then((s) => {
            if (cancelled) return;
            setShowAuthUi(s.showAuthUi);
            setUser(s.user ?? null);
            if (s.showAuthUi && !s.bypassed) {
              setPage("login");
            }
          })
          .catch(() => undefined);
        void reload();
        void listenRiviuEvents((payload) => {
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
            setDevices((prev) => markDeviceFrameLive(prev, p.udid as string));
          }
        }).then((fn) => {
          if (cancelled) {
            fn();
          } else {
            unlisten = fn;
          }
        });
      })
      .catch((error) => {
        if (cancelled) return;
        setStartupIssue(null);
        setBootError(String(error));
        void reload();
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
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

  if (startupIssue) {
    return (
      <main className="startup-state">
        <div className="startup-state-card">
          <h1>Riviu Manager</h1>
          <h2>Chưa sẵn sàng khởi động</h2>
          <p>{startupIssue}</p>
          <p>
            Kiểm tra cấu hình agent và Keychain của bản đang chạy, sau đó mở lại
            app. Bản Full tự tạo credential cục bộ; bản production yêu cầu token
            RT-MMO được cấu hình một lần.
          </p>
          <button type="button" className="primary" onClick={() => window.location.reload()}>
            Thử lại
          </button>
        </div>
      </main>
    );
  }

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
        onPage={(next) => void requestPage(next)}
        onToggleCollapse={() => setAsideCollapsed((v) => !v)}
      />

      <div className="main-col">
        <header className="topbar">
          <div className="topbar-title">{title}</div>
          <div className="topbar-drag" />
          <div className="topbar-actions">
            {groupMode && <span className="chip primary">Sync</span>}
            {readyCount > 0 && <span className="chip ok">{readyCount} sẵn sàng</span>}
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

        <div className={`content ${page === "scripts" ? "content-flow" : ""}`}>
          {bootError && (
            <Banner
              tone="error"
              action={
                <button type="button" onClick={() => void reload()}>
                  Thử lại
                </button>
              }
            >
              Chưa kết nối được backend: {bootError}
            </Banner>
          )}

          {driverIssue && (
            <Banner tone="error">
              Không đọc được thiết bị thật — danh sách sẽ luôn trống cho tới khi
              sửa xong. Nguyên nhân: {driverIssue}
            </Banner>
          )}

          {/* `warn`, not `error`, and the difference is deliberate: unlike a dead iOS
              sidecar, this is usually simply true — a farm with no Android phones in it.
              A red banner for a correct state trains the operator to ignore banners.

              It is also a boot snapshot, and says so: `MultiplexDriver::new` fixes the
              backend list at construction, so installing adb now cannot make Android join
              without restarting the app. Claiming otherwise would be the worse lie. */}
          {androidIssue && (
            <Banner tone="warn">
              Máy Android không tham gia fleet (kiểm lúc mở app — cài adb xong phải
              khởi động lại app). Nguyên nhân: {androidIssue}
            </Banner>
          )}

          {page === "control" && (
            <>
              <ProfileToolbar
                selected={selectedDevices}
                syncOn={groupMode}
                nurtureOpen={nurtureOpen}
                onNurture={() => {
                  setInteractionOpen(false);
                  setNurtureOpen((v) => !v);
                }}
                interactionOpen={interactionOpen}
                onInteraction={() => {
                  setNurtureOpen(false);
                  setInteractionOpen((v) => !v);
                }}
                onStart={async () => {
                  const targets = selected.length
                    ? devices.filter((device) => selected.includes(device.udid))
                    : devices;
                  if (!targets.length) {
                    pushToast("warn", "Chưa có thiết bị", "Cắm máy qua USB rồi bấm Làm mới.");
                    return;
                  }
                  try {
                    await startFleetPreview(targets);
                    await reload();
                    pushToast("ok", "Đã khởi động", `Chuẩn bị ${targets.length} máy`);
                  } catch (error) {
                    toastError("Khởi động thất bại", error);
                  }
                }}
                onStop={() => setSelected([])}
                onInstall={async () => {
                  const targets = selected.length
                    ? selected
                    : devices
                        .filter((device) => device.status !== "disconnected")
                        .map((device) => device.udid);
                  if (!targets.length) {
                    pushToast("warn", "Chưa có thiết bị", "Cắm iPhone qua USB rồi bấm Làm mới.");
                    return;
                  }
                  const scope = selected.length ? "đã chọn" : "đang kết nối";
                  const proceed = await requestConfirm({
                    title: `Sửa Riviu Agent trên ${targets.length} máy?`,
                    message: `Áp dụng cho ${targets.length} máy ${scope}. Stream trên các máy này sẽ khởi động lại.`,
                    confirmLabel: "Sửa agent",
                  });
                  if (!proceed) return;
                  try {
                    const repaired = await agentBulkRepair(targets);
                    const [, refreshed] = await Promise.all([
                      reload(),
                      agentListStatuses(targets),
                    ]);
                    const summary = summarizeBulkRepair(
                      refreshed.length ? refreshed : repaired,
                    );
                    pushToast(
                      summary.attentionCount > 0 ? "warn" : "ok",
                      summary.heading,
                      summary.message,
                    );
                  } catch (error) {
                    toastError("Sửa agent thất bại", error);
                  }
                }}
                onSync={() => setGroupMode((v) => !v)}
                onRefresh={async () => {
                  await refreshDevices();
                  await reload();
                }}
              />

              {/* One row, not two: tabs on the left, size and view mode on the right.
                  The tab strip keeps its own horizontal scroll and the controls do not
                  join it — otherwise the slider scrolls away with the tabs. */}
              <div className="device-toolrow">
                <GroupTabs tabs={tabs} active={groupTab} onSelect={setGroupTab} />
                <FilterToolbar
                  viewMode={viewMode}
                  onViewMode={setViewMode}
                  tileWidth={tileWidth}
                  onTileWidth={setTileWidth}
                />
              </div>

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
                    {devices.map((device) => {
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
                          <td>{deviceModelOsLabel(device)}</td>
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
                <div className="window-canvas" ref={canvasRef}>
                  {visibleDevices.map((device, i) => (
                    <DeviceTile
                      key={device.udid}
                      device={device}
                      width={tileWidth}
                      index={i + 1}
                      onContextMenu={(udid, x, y) => setTileMenu({ udid, x, y })}
                      selected={selected.includes(device.udid)}
                      focused={focusUdid === device.udid}
                      onSelect={onSelect}
                      onOpen={setFocusUdid}
                      onPrepare={(udid) => {
                        const device = devices.find((item) => item.udid === udid);
                        if (!device) return;
                        void startDevicePreview(device).then(reload);
                      }}
                    />
                  ))}
                </div>
              )}

              {tileMenu && menuDevice && (
                <DeviceContextMenu
                  device={menuDevice}
                  groups={groups}
                  x={tileMenu.x}
                  y={tileMenu.y}
                  onClose={() => setTileMenu(null)}
                  actions={tileActions(menuDevice)}
                  onAddToGroup={async (groupId) => {
                    const next = withDeviceAdded(groups, groupId, menuDevice.udid);
                    // null means the device is already in that group, or the group is
                    // gone. Saving anyway would rewrite the record for nothing.
                    if (!next) return;
                    try {
                      await saveGroup(next);
                      await reload();
                      pushToast("ok", `Đã thêm vào nhóm ${next.name}`);
                    } catch (error) {
                      toastError("Thêm vào nhóm thất bại", error);
                    }
                  }}
                />
              )}

              {adbFor && menuAdbDevice && (
                <AdbConsole device={menuAdbDevice} onClose={() => setAdbFor(null)} />
              )}

              {!devices.length && (
                <EmptyState
                  icon={<IconPhone size={20} />}
                  title="Chưa có điện thoại nào"
                  hint="Cắm máy qua USB, bấm Tin cậy (Trust) trên iPhone nếu được hỏi, rồi làm mới danh sách."
                  action={
                    <button
                      type="button"
                      className="primary"
                      onClick={async () => {
                        await refreshDevices();
                        await reload();
                      }}
                    >
                      Làm mới
                    </button>
                  }
                />
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
            <section className="automation-surface">
              <div role="tablist" aria-label="Automation view" className="automation-tabs">
                <button
                  type="button"
                  role="tab"
                  aria-selected={automationView === "flow"}
                  onClick={() => void requestAutomationView("flow")}
                >
                  Flow
                </button>
                <button
                  type="button"
                  role="tab"
                  aria-selected={automationView === "legacy"}
                  onClick={() => void requestAutomationView("legacy")}
                >
                  Legacy
                </button>
              </div>
              {automationView === "flow" ? (
                <Suspense
                  fallback={(
                    <LoadingState label="Đang tải Flow…" />
                  )}
                >
                  <FlowWorkspace
                    devices={devices}
                    selectedUdids={selected}
                    onDirtyChange={setFlowDirty}
                  />
                </Suspense>
              ) : (
                <div className="automation-legacy">
                  <ScriptsPanel
                    onUseInJobs={(json) => {
                      setJobsScriptSeed(json);
                      void requestPage("jobs");
                    }}
                  />
                  <div className="panel automation-legacy-schedule">
                    <ScheduleBlock
                      devices={devices}
                      selected={selected}
                      onSelectUdids={setSelected}
                    />
                  </div>
                </div>
              )}
            </section>
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
          {page === "settings" && <SettingsPanel devices={devices} />}
        </div>
      </div>

      {focusDevice && (
        <FocusStream
          device={focusDevice}
          index={devices.findIndex((d) => d.udid === focusDevice.udid) + 1 || 1}
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

      {page === "control" && interactionOpen && (
        <InteractionPopup
          devices={devices}
          selected={selected}
          onClose={() => setInteractionOpen(false)}
        />
      )}

      <ToastHost />
      <ConfirmHost />
    </div>
  );
}

export default App;
