import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  agentBulkRepair,
  agentListStatuses,
  deploymentFrontendReady,
  refreshDevices,
  saveGroup,
  viewSetPreset,
} from "./api";
import { startDevicePreview, startFleetPreview } from "./startPreview";
import { summarizeBulkRepair } from "./agentStatus";
import { requestConfirm } from "./confirmStore";
import { surfaceDeparted } from "./deviceSurface";
import { describeError } from "./describeError";
import { pushToast, toastError } from "./toastStore";
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
import { DeviceSyslogPopup } from "./components/DeviceSyslogPopup";
import { DeviceHealthPopup } from "./components/DeviceHealthPopup";
import { ALL_DEVICES_TAB, devicesInTab, groupTabs, withDeviceAdded } from "./deviceGroups";
import { FocusStream } from "./components/FocusStream";
import { IconPhone, IconRefresh } from "./components/Icons";
import { Banner, EmptyState, LoadingState } from "./components/States";
import { InteractionPopup } from "./components/InteractionPopup";
import { JobsPanel } from "./components/JobsPanel";
import { NurturePopup } from "./components/NurturePopup";
import { GroupManagerPopup } from "./components/GroupManagerPopup";
import { GroupToolsPopup } from "./components/GroupToolsPopup";
import { ProfileToolbar } from "./components/ProfileToolbar";
import { ScriptsPanel } from "./components/ScriptsPanel";
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { forgetDepartedViews, useViewClient } from "./viewStore";
import { ApiPage } from "./pages/ApiPage";
import { AppsPage } from "./pages/AppsPage";
import { DataPage } from "./pages/DataPage";
import { MaterialPage } from "./pages/MaterialPage";
import { PublishPage } from "./pages/PublishPage";
import { ScheduleBlock } from "./pages/ScheduleBlock";
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

/**
 * State for a surface opened against **one phone**, that closes itself — out loud — when that
 * phone leaves the fleet.
 *
 * **Why this is a hook and not three `useState`s.** `App` held three of these — the adb console,
 * the file browser, and the focus overlay — each resolved through `devices.find(...) ?? null`
 * into a render gated on the result, and none of them cleared the udid when the phone went away.
 * The consequence is not that the panel closes; it is that it closes **silently and then refuses
 * to reopen**: the stale udid is still in state, so clicking the same phone's row is a `setState`
 * with the value already there, React bails out, and the row does nothing at all. Permanently,
 * for that phone, until another phone is clicked or the app restarts.
 *
 * That is the reported bug — *"mở thư mục máy điện thoại còn mở không được"* — and `controlCenter`
 * had the fix for it 470 lines below, with a doc comment making the argument, while the surface
 * that needed it most did not. Extracted so the next per-phone surface cannot be written without
 * it. See `deviceSurface.ts` for why an empty roster is not a departure.
 */
function useDeviceSurface(
  devices: DeviceInfo[],
  /// Names the thing in the message — "đã đóng trình quản lý tệp". Not the component name.
  label: string,
): [string | null, (udid: string | null) => void] {
  const [openFor, setOpenFor] = useState<string | null>(null);
  /// The phone's display name, captured **when the surface opened**. At clear time the device is
  /// already out of the roster, so its name is unreachable then — and "một máy đã rời" is a
  /// worse message than naming it.
  const nameRef = useRef<string>("");
  /// A ref rather than a dependency, so `open` keeps a stable identity across roster updates:
  /// it is handed to `tileActions`, and a new function on every scan would churn every consumer.
  const devicesRef = useRef(devices);
  useEffect(() => {
    devicesRef.current = devices;
  }, [devices]);

  const open = useCallback((udid: string | null) => {
    if (udid) {
      nameRef.current =
        devicesRef.current.find((device) => device.udid === udid)?.name ?? udid;
    }
    setOpenFor(udid);
  }, []);

  useEffect(() => {
    if (!surfaceDeparted(devices, openFor)) return;
    setOpenFor(null);
    // Silence is what made this a bug report rather than an annoyance: an operator three
    // folders deep watched the panel evaporate with no word. `controlCenter` clears quietly
    // because a designation vanishing is invisible anyway; a panel closing under someone's
    // hands is not.
    pushToast(
      "warn",
      "Máy đã rời khỏi danh sách",
      `${nameRef.current} không còn kết nối — đã đóng ${label}.`,
    );
  }, [devices, openFor, label]);

  return [openFor, open];
}

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
    fleetSettled,
    driverIssue,
    androidIssue,
    androidToolProblems,
    logDirectory,
    retryingStartup,
    retry: retryStartupAndResubscribe,
  } = useFleet();
  const [asideCollapsed, setAsideCollapsed] = useState(false);
  const [groupTab, setGroupTab] = useState<string>(ALL_DEVICES_TAB);
  const [tileMenu, setTileMenu] = useState<{ udid: string; x: number; y: number } | null>(null);
  const [adbFor, setAdbFor] = useDeviceSurface(devices, "bảng lệnh adb");
  const [syslogFor, setSyslogFor] = useDeviceSurface(devices, "log của máy");
  const [healthFor, setHealthFor] = useDeviceSurface(devices, "bảng kiểm tra máy");
  /// Which phone's filesystem is open in the browser popup (xiaowei "Preview Mobile Files").
  const [filesFor, setFilesFor] = useDeviceSurface(devices, "trình quản lý tệp");
  const [groupMode, setGroupMode] = useState(false);
  /// The phone the operator drives when Sync is on; every other selected phone follows it.
  ///
  /// This used to be `selected[0]` — whichever udid happened to land first in the selection
  /// array — decided on a page of its own that did nothing else. Nothing showed which phone
  /// it was and nothing let the operator choose, so "máy chính" was a label for an accident.
  /// It is a property of the grid, set from the tile's own menu, and it lives here.
  const [controlCenter, setControlCenter] = useState<string | null>(null);
  const [focusUdid, setFocusUdid] = useDeviceSurface(devices, "màn phóng to");
  const [jobsScriptSeed, setJobsScriptSeed] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("window");
  const [tileWidth, setTileWidth] = useState(() => loadZoom(TILE_ZOOM));
  const [nurtureOpen, setNurtureOpen] = useState(false);
  const [interactionOpen, setInteractionOpen] = useState(false);
  const [groupToolsOpen, setGroupToolsOpen] = useState(false);
  const [groupsOpen, setGroupsOpen] = useState(false);
  const [flowDirty, setFlowDirty] = useState(false);
  const [automationView, setAutomationView] = useState<"flow" | "legacy">("flow");
  useViewClient();

  useEffect(() => {
    if (startupIssue !== null || !fleetSettled || bootError) return;
    void deploymentFrontendReady();
  }, [bootError, fleetSettled, startupIssue]);

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
  const menuSyslogDevice = useMemo(
    () => (syslogFor ? (devices.find((d) => d.udid === syslogFor) ?? null) : null),
    [syslogFor, devices],
  );
  const menuHealthDevice = useMemo(
    () => (healthFor ? (devices.find((d) => d.udid === healthFor) ?? null) : null),
    [healthFor, devices],
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
        setSyslogFor,
        setHealthFor,
      }),
    // Setters straight from `useState` are stable and stay out of the list. `setMetas` is
    // in it because it now arrives through `useFleet`'s return object, where the rule cannot
    // see that it is a setter — including it is free and cheaper than an exemption.
    // The stale-closure note that used to sit here still applies and now lives with the
    // catalog: a stale `metas` pre-fills the rename dialog with the value just replaced.
    // The three surface openers come from `useDeviceSurface` now, not from `useState`, so the
    // hooks lint cannot see that they are stable. They are — each is a `useCallback` with an
    // empty dependency list — but listing them is free and keeps the gate honest.
    [
      reload,
      controlCenter,
      groupMode,
      metaMap,
      metas,
      setMetas,
      setAdbFor,
      setSyslogFor,
      setHealthFor,
      setFilesFor,
      setFocusUdid,
    ],
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
          <h1 className="topbar-title" data-testid="page-title">
            {title}
          </h1>
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

          {driverIssue && page === "control" && (
            <Banner tone="warn">
              Nhánh iOS không sẵn sàng; các máy Android vẫn hoạt động độc lập. Nguyên nhân: {driverIssue}
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

          {/* **The bundle is there and broken, which is not the same as absent.** Nine files
              are verified against `android-tools-manifest.json` at boot; adb is one of them, so
              a bundle that lost the agent APKs still resolves adb — the fleet lists phones and
              every attempt to drive one fails. Reported from a real install as "lên app rồi,
              nhận điện thoại rồi, nhưng điều khiển không được", with nothing on screen and the
              only record a `log::warn!` in a file nobody knew about.

              `warn` and not `error`: the app is still usable for everything that does not drive
              an Android phone, and the remedy is reinstalling rather than anything in here. */}
          {androidToolProblems.length > 0 && (
            <Banner tone="warn">
              Bộ công cụ Android trong bản cài không khớp bản kê — máy vẫn hiện trong
              danh sách nhưng <strong>điều khiển sẽ không chạy</strong>. Cài lại app;
              nếu vẫn vậy, gửi file log ở <code>{logDirectory ?? "thư mục log của bản cài"}</code>.
              Nguyên nhân: {androidToolProblems.join("; ")}
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
                  setGroupsOpen(false);
                  setInteractionOpen((v) => !v);
                }}
                groupsOpen={groupsOpen}
                onGroups={() => {
                  setNurtureOpen(false);
                  setInteractionOpen(false);
                  setGroupToolsOpen(false);
                  setGroupsOpen((v) => !v);
                }}
                groupToolsOpen={groupToolsOpen}
                onGroupTools={() => {
                  setNurtureOpen(false);
                  setInteractionOpen(false);
                  setGroupsOpen(false);
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
                {/* **Selects what is on screen, and says how many.**
                    `visibleDevices`, not `devices`: this sits beside the group tabs, so
                    "tất cả" has to mean the tab the operator is looking at. Saying the
                    number in the label is what keeps that honest — a bare "Chọn tất cả"
                    next to a filtered tab is the kind of button that quietly picks eight
                    when the operator meant twenty, and with Sync on, the next thing they
                    press reaches every one of them. */}
                <div className="device-selectall">
                  <button
                    type="button"
                    className="ghost"
                    disabled={!visibleDevices.length}
                    onClick={() =>
                      setSelected(visibleDevices.map((device) => device.udid))
                    }
                  >
                    Chọn tất cả ({visibleDevices.length})
                  </button>
                  <button
                    type="button"
                    className="ghost"
                    disabled={!selected.length}
                    onClick={() => setSelected([])}
                  >
                    Bỏ chọn
                  </button>
                </div>
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
              loading={!fleetSettled}
              loadError={bootError}
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

      {/* **Outside the control-page block, and that placement is the fix.** Both of these are
          opened from `tileActions`, which `FocusStream` renders — and `FocusStream` is mounted
          here, not inside `{page === "control"}`. While they lived in that block, opening the
          zoom overlay on any other page and clicking "Tệp trên máy…" or "Lệnh adb" set the udid,
          rendered nothing, and then `useDeviceSurface`'s stale-udid trap made the row dead for
          that phone.

          The sibling popups below (nurture, interaction, groups, tools) stay page-gated on
          purpose: they act on `selected`, which is a control-grid concept. These two act on one
          phone and read nothing but its udid and name. */}
      {syslogFor && menuSyslogDevice && (
        <DeviceSyslogPopup device={menuSyslogDevice} onClose={() => setSyslogFor(null)} />
      )}
      {healthFor && menuHealthDevice && (
        <DeviceHealthPopup device={menuHealthDevice} onClose={() => setHealthFor(null)} />
      )}
      {adbFor && menuAdbDevice && (
        <AdbConsole device={menuAdbDevice} onClose={() => setAdbFor(null)} />
      )}

      {filesFor && menuFilesDevice && (
        <DeviceFilesPopup device={menuFilesDevice} onClose={() => setFilesFor(null)} />
      )}

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
          metas={metaMap}
          onClose={() => setNurtureOpen(false)}
        />
      )}

      {page === "control" && interactionOpen && (
        <InteractionPopup
          devices={devices}
          selected={selected}
          metas={metaMap}
          onClose={() => setInteractionOpen(false)}
        />
      )}

      {page === "control" && groupsOpen && (
        <GroupManagerPopup
          devices={devices}
          groups={groups}
          metas={metaMap}
          onChanged={reload}
          onClose={() => setGroupsOpen(false)}
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
