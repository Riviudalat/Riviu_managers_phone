import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import {
  agentBulkRepair,
  agentListStatuses,
  refreshDevices,
  saveGroup,
  viewSetPreset,
} from "./api";
import { startDevicePreview, startFleetPreview } from "./startPreview";
import { summarizeBulkRepair } from "./agentStatus";
import { requestConfirm } from "./confirmStore";
import { describeError, pushToast, toastError } from "./toastStore";
import { ConfirmHost } from "./components/ConfirmHost";
import { ToastHost } from "./components/ToastHost";
import { DeviceTile } from "./components/DeviceTile";
import { FilterToolbar, type ViewMode } from "./components/FilterToolbar";
import { GroupTabs } from "./components/GroupTabs";
import { DeviceContextMenu } from "./components/DeviceContextMenu";
import { DeviceFilesPopup } from "./components/DeviceFilesPopup";
import type { DeviceMenuNode } from "./deviceMenu";
import { buildDeviceActions } from "./deviceActions";
import { useFleet } from "./useFleet";
import { useBoxSelection } from "./useBoxSelection";
import { metaByUdid, orderDevicesByNumber, tileName, tileNumber } from "./deviceNaming";
import { AdbConsole } from "./components/AdbConsole";
import { ALL_DEVICES_TAB, devicesInTab, groupTabs, withDeviceAdded } from "./deviceGroups";
import { FocusStream } from "./components/FocusStream";
import { IconPhone, IconRefresh } from "./components/Icons";
import { Banner, EmptyState, LoadingState } from "./components/States";
import { InteractionPopup } from "./components/InteractionPopup";
import { JobsPanel } from "./components/JobsPanel";
import { NurturePopup } from "./components/NurturePopup";
import { GroupToolsPopup } from "./components/GroupToolsPopup";
import { ProfileToolbar } from "./components/ProfileToolbar";
import { ScriptsPanel } from "./components/ScriptsPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { forgetDepartedViews, useViewClient } from "./viewStore";
import {
  ApiPage,
  AppsPage,
  DataPage,
  MaterialPage,
  PublishPage,
  ScheduleBlock,
} from "./pages/FarmPages";
import type { DeviceInfo, PageId } from "./types";
import { deviceModelOsLabel } from "./types";
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
  publish: "Đăng bài",
  data: "Dữ liệu",
  api: "API",
  settings: "Cài đặt",
};

function App() {
  const [page, setPage] = useState<PageId>("control");
  const {
    devices,
    groups,
    metas,
    setMetas,
    jobs,
    reload,
    startupIssue,
    bootError,
    driverIssue,
    androidIssue,
    retryingStartup,
    retry: retryStartupAndResubscribe,
  } = useFleet();
  const [asideCollapsed, setAsideCollapsed] = useState(false);
  const [groupTab, setGroupTab] = useState<string>(ALL_DEVICES_TAB);
  const [tileMenu, setTileMenu] = useState<{ udid: string; x: number; y: number } | null>(null);
  const [adbFor, setAdbFor] = useState<string | null>(null);
  /// Which phone's filesystem is open in the browser popup (xiaowei "Preview Mobile Files").
  const [filesFor, setFilesFor] = useState<string | null>(null);
  const [groupMode, setGroupMode] = useState(false);
  /// The phone the operator drives when Sync is on; every other selected phone follows it.
  ///
  /// This used to be `selected[0]` — whichever udid happened to land first in the selection
  /// array — decided on a page of its own that did nothing else. Nothing showed which phone
  /// it was and nothing let the operator choose, so "máy chính" was a label for an accident.
  /// It is a property of the grid, set from the tile's own menu, and it lives here.
  const [controlCenter, setControlCenter] = useState<string | null>(null);
  const [focusUdid, setFocusUdid] = useState<string | null>(null);
  const [jobsScriptSeed, setJobsScriptSeed] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("window");
  const [tileWidth, setTileWidth] = useState(() => loadZoom(TILE_ZOOM));
  const [nurtureOpen, setNurtureOpen] = useState(false);
  const [interactionOpen, setInteractionOpen] = useState(false);
  const [groupToolsOpen, setGroupToolsOpen] = useState(false);
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

  const tabs = useMemo(() => groupTabs(devices, groups), [devices, groups]);
  const metaMap = useMemo(() => metaByUdid(metas), [metas]);
  // Numbered phones lead, in number order; an unnumbered fleet is left exactly as the
  // driver listed it. That is the point of a number — a grid position moves when a phone
  // drops off USB, a number does not.
  const visibleDevices = useMemo(
    () => orderDevicesByNumber(devicesInTab(devices, groups, groupTab), metaMap),
    [devices, groups, groupTab, metaMap],
  );

  const {
    selected,
    setSelected,
    selectedDevices,
    onSelect,
    canvasRef,
    onCanvasMouseDown,
    band,
  } = useBoxSelection(devices, visibleDevices, page === "control" && viewMode === "window");

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
  // `canvasRef` is a ref object and never changes identity, but it now arrives through
  // `useBoxSelection`'s return value where the rule cannot see that. Listing it is free.
  }, [page, viewMode, canvasRef]);
  const menuAdbDevice = useMemo(
    () => (adbFor ? (devices.find((d) => d.udid === adbFor) ?? null) : null),
    [adbFor, devices],
  );
  const menuDevice = useMemo(
    () => (tileMenu ? (devices.find((d) => d.udid === tileMenu.udid) ?? null) : null),
    [tileMenu, devices],
  );
  const menuFilesDevice = useMemo(
    () => (filesFor ? (devices.find((d) => d.udid === filesFor) ?? null) : null),
    [filesFor, devices],
  );

  /**
   * The per-phone function menu, and every row of it is a command this app already has.
   *
   * That rule stands and is the reason this list is long rather than aspirational: a row
   * calling a command we never wrote is a button that fails. What the rule never justified
   * was the *shortfall* — measured against the reference product's own phone menu on
   * 21/08/2026 this had ten rows against its thirty-five, and the honest reading was not
   * "we lay it out differently" but "eight of its rows have no command here". Those eight
   * are the ones written that day: read the clipboard, the phone's Wi-Fi radio, reset
   * DPI/resolution, power off, open the phone's Settings, wake the screen, screenshot into
   * the phone's own gallery, and browse its filesystem.
   *
   * Three of its rows are still deliberately elsewhere rather than here, because they are
   * fleet-shaped rather than phone-shaped and this app already had a better place for them:
   * text/file distribution and the macro recorder live in the group Tools popup, task
   * lists live in the Flow panel, and agent repair lives in Settings. One is genuinely not
   * built: a gesture *recorder* per phone (xiaowei "Action Record"), as distinct from the
   * macro replay that Tools already has.
   */
  const tileActions = useCallback(
    (device: DeviceInfo): DeviceMenuNode[] =>
      buildDeviceActions(device, {
        reload,
        metaMap,
        metas,
        setMetas,
        controlCenter,
        setControlCenter,
        groupMode,
        setFocusUdid,
        setFilesFor,
        setAdbFor,
      }),
    // Setters straight from `useState` are stable and stay out of the list. `setMetas` is
    // in it because it now arrives through `useFleet`'s return object, where the rule cannot
    // see that it is a setter — including it is free and cheaper than an exemption.
    // The stale-closure note that used to sit here still applies and now lives with the
    // catalog: a stale `metas` pre-fills the rename dialog with the value just replaced.
    [reload, controlCenter, groupMode, metaMap, metas, setMetas],
  );

  /// The phone the overlay actually drives.
  ///
  /// With Sync on and a control centre designated, that is the control centre whichever tile
  /// was opened — which is what designating one means. Without Sync it is simply the tile the
  /// operator opened, because a centre with nothing following it would be a surprise rather
  /// than a feature.
  const focusDevice = useMemo(() => {
    const wanted =
      groupMode && controlCenter && devices.some((d) => d.udid === controlCenter)
        ? controlCenter
        : focusUdid;
    return devices.find((d) => d.udid === wanted) ?? null;
  }, [devices, focusUdid, groupMode, controlCenter]);

  /// A designated phone that has left the fleet is not a designation, it is a dangling udid
  /// that would silently redirect the overlay to a device that is not there.
  useEffect(() => {
    if (!controlCenter) return;
    if (devices.length && !devices.some((d) => d.udid === controlCenter)) {
      setControlCenter(null);
    }
  }, [devices, controlCenter]);

  /// The view store keeps one entry per udid and never used to drop one.
  ///
  /// Two of those entries matter. `live` decides whether a tile says the stream is up, so a
  /// phone that goes away while live and comes back is *already* live before a single
  /// packet arrives — its tile shows a white canvas labelled as working. And the paint
  /// counters are what the host's watchdog is handed every two seconds, so it kept
  /// receiving evidence about devices that had left.
  ///
  /// Guarded on a non-empty roster: an empty `devices` is what this app looks like for the
  /// first moment after boot and during a failed scan, and forgetting everything then would
  /// blank every tile that is about to be listed again.
  useEffect(() => {
    if (!devices.length) return;
    forgetDepartedViews(devices.map((device) => device.udid));
  }, [devices]);

  // Ask for the overlay's own encode while it is open, and give it back on close.
  //
  // The overlay CSS-scales one shared stream, so without this it displays the tile encode
  // -- 216x480 on this fleet -- across 400 to 760 px, a 1.8x to 3.5x upscale. The call
  // existed and had no caller, which is why raising the overlay's resolution in an earlier
  // commit changed nothing on a real phone.
  //
  // Keyed on the udid rather than on focusDevice: the memo produces a new object on every
  // device poll, and restarting the encoder a few times a second is worse than a soft
  // picture. Failure is deliberately swallowed to a log -- the tile encode still plays, so
  // a phone that refuses the larger one should look worse, not stop.
  //
  // `focusDevice`, not `focusUdid`: with Sync on and a control centre designated, the
  // overlay drives the control centre whichever tile was opened — that is what designating
  // one means — so keying on `focusUdid` asked a phone nobody is watching for the larger
  // encode and left the phone on screen at the tile preset. Its `.udid` rather than the
  // object, for the reason above: the memo is a new object on every poll.
  const overlayUdid = focusDevice?.udid ?? null;
  useEffect(() => {
    if (!overlayUdid) return;
    void viewSetPreset(overlayUdid, "overlay").catch((error) => {
      console.warn("overlay preset refused", error);
    });
    return () => {
      void viewSetPreset(overlayUdid, "tile").catch(() => {
        // The device may be gone -- that is often what closing the overlay means.
      });
    };
  }, [overlayUdid]);

  const readyCount = useMemo(
    () => devices.filter((d) => d.wdaReady || d.status === "ready").length,
    [devices],
  );

  const runningJobs = useMemo(
    () => jobs.filter((j) => j.status === "running" || j.status === "queued").length,
    [jobs],
  );

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
          <button
            type="button"
            className="primary"
            disabled={retryingStartup}
            onClick={() => void retryStartupAndResubscribe()}
          >
            {retryingStartup ? "Đang thử lại…" : "Thử lại"}
          </button>
        </div>
      </main>
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
          <div className="topbar-title" data-testid="page-title">
            {title}
          </div>
          <div className="topbar-drag" />
          <div className="topbar-actions">
            {groupMode && <span className="chip primary">Sync</span>}
            {readyCount > 0 && <span className="chip ok">{readyCount} sẵn sàng</span>}
            {runningJobs > 0 && <span className="chip warn">{runningJobs} job</span>}
            <button
              type="button"
              className="icon-btn"
              title="Làm mới danh sách máy"
              onClick={async () => {
                // Same missing failure path as the toolbar's copy. Both are guarded now;
                // the titles differ so the two are distinguishable to a reader and to a
                // test, which they were not when both said "Refresh".
                try {
                  await refreshDevices();
                  await reload();
                } catch (error) {
                  toastError("Không làm mới được danh sách máy", error);
                }
              }}
            >
              <IconRefresh size={16} />
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
                  setGroupToolsOpen(false);
                  setNurtureOpen((v) => !v);
                }}
                interactionOpen={interactionOpen}
                onInteraction={() => {
                  setNurtureOpen(false);
                  setGroupToolsOpen(false);
                  setInteractionOpen((v) => !v);
                }}
                groupToolsOpen={groupToolsOpen}
                onGroupTools={() => {
                  setNurtureOpen(false);
                  setInteractionOpen(false);
                  setGroupToolsOpen((v) => !v);
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
                    const failures = await startFleetPreview(targets);
                    await reload();
                    if (failures.length === 0) {
                      pushToast("ok", "Đã khởi động", `Chuẩn bị ${targets.length} máy`);
                    } else if (failures.length === targets.length) {
                      toastError("Không máy nào khởi động được", failures[0].reason);
                    } else {
                      // The count first, the names second. On twenty phones the list is
                      // what an operator acts on, and "Khởi động thất bại" with one
                      // message used to be all they got — for a run where most succeeded.
                      pushToast(
                        "warn",
                        `Khởi động ${targets.length - failures.length}/${targets.length} máy`,
                        `${failures.map((failure) => failure.name).join(", ")} chưa khởi động được: ${describeError(failures[0].reason)}`,
                      );
                    }
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
                  // Refresh had no failure path at all: `onClick={() => void onRefresh()}`
                  // dropped the rejection, so a device scan that failed left the fleet
                  // unchanged and said nothing. Pressing it again did the same nothing.
                  try {
                    await refreshDevices();
                    await reload();
                  } catch (error) {
                    toastError("Không làm mới được danh sách máy", error);
                  }
                }}
              />

              {/* One row, not two: tabs on the left, size and view mode on the right.
                  The tab strip keeps its own horizontal scroll and the controls do not
                  join it — otherwise the slider scrolls away with the tabs. */}
              <div className="device-toolrow">
                <GroupTabs tabs={tabs} active={groupTab} onSelect={setGroupTab} />
                <FilterToolbar viewMode={viewMode} onViewMode={setViewMode} />
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
                <div
                  className="window-canvas"
                  ref={canvasRef}
                  title="Ctrl + lăn chuột để phóng to / thu nhỏ · kéo chuột để quét chọn máy"
                  onMouseDown={onCanvasMouseDown}
                >
                  {band && (
                    <div
                      className="select-band"
                      style={{
                        left: band.left,
                        top: band.top,
                        width: band.right - band.left,
                        height: band.bottom - band.top,
                      }}
                    />
                  )}
                  {visibleDevices.map((device, i) => (
                    <DeviceTile
                      key={device.udid}
                      device={device}
                      width={tileWidth}
                      index={tileNumber(i + 1, metaMap.get(device.udid))}
                      name={tileName(device, metaMap.get(device.udid))}
                      onContextMenu={(udid, x, y) => setTileMenu({ udid, x, y })}
                      selected={selected.includes(device.udid)}
                      focused={focusUdid === device.udid}
                      controlCenter={controlCenter === device.udid}
                      onSelect={onSelect}
                      onOpen={setFocusUdid}
                      onPrepare={(udid) => {
                        const device = devices.find((item) => item.udid === udid);
                        if (!device) return;
                        // `.catch` is the fix. This is the button on a tile that has
                        // already failed once, so it is pressed at the exact moment the
                        // operator is least able to tolerate silence -- and a rejection
                        // here used to go nowhere but the console.
                        startDevicePreview(device)
                          .then(reload)
                          .catch((error) => toastError(`Không mở lại được ${device.name}`, error));
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
                  nodes={tileActions(menuDevice)}
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

              {filesFor && menuFilesDevice && (
                <DeviceFilesPopup device={menuFilesDevice} onClose={() => setFilesFor(null)} />
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
                        try {
                          await refreshDevices();
                          await reload();
                        } catch (error) {
                          toastError("Không làm mới được danh sách máy", error);
                        }
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
          {page === "publish" && (
            <PublishPage
              devices={devices}
              selected={selected}
              onSelectUdids={setSelected}
            />
          )}
          {page === "data" && <DataPage />}
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
          // The same array `index` above is computed from, so the picker's numbering and the
          // header's cannot disagree about which phone is #3.
          devices={devices}
          onSelectDevice={setFocusUdid}
          // The same catalog the tile's right-click menu gets. Zooming into a phone is a
          // different *view* of it, not a smaller set of things you can do to it.
          functions={tileActions(focusDevice)}
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

      {page === "control" && groupToolsOpen && (
        <GroupToolsPopup
          devices={devices}
          selected={selected}
          onClose={() => setGroupToolsOpen(false)}
        />
      )}

      <ToastHost />
      <ConfirmHost />
    </div>
  );
}

export default App;
