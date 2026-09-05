import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import {
  agentBulkRepair,
  agentListStatuses,
  deploymentFrontendReady,
  listDeviceWorkStates,
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
import { ActivityCenter } from "./components/ActivityCenter";
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
import { DeviceDetailsDrawer } from "./components/DeviceDetailsDrawer";
import { FleetDiagnosticsPage } from "./components/FleetDiagnosticsPage";
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
import { SettingsPanel } from "./components/SettingsPanel";
import { Sidebar } from "./components/Sidebar";
import { TargetSelector } from "./components/TargetSelector";
import { PageHeader, StatusChip } from "./components/WorkspacePrimitives";
import { resolveAutomationTarget } from "./automationTargets";
import {
  deviceMatchesFleetFilter,
  deviceOperationalView,
} from "./deviceWork";
import type { DeviceOperationalFilter, DeviceWorkOwnerReadState } from "./deviceWork";
import { forgetDepartedViews, useViewClient } from "./viewStore";
import { ApiPage } from "./pages/ApiPage";
import { AppsPage } from "./pages/AppsPage";
import { DataPage } from "./pages/DataPage";
import { MaterialPage } from "./pages/MaterialPage";
import { PublishPage } from "./pages/PublishPage";
import type { DeviceInfo, DeviceWorkOwner, PageId, TargetRef } from "./types";
import { MoreHorizontal } from "lucide-react";
import { MENU_ICONS } from "./components/menuIcons";
import { loadZoom, stepZoom, storeZoom, TILE_ZOOM, wheelWantsZoom } from "./zoom";
import { useMediaQuery } from "./useMediaQuery";
import "./App.css";

const FlowWorkspace = lazy(async () => {
  const module = await import("./components/flow/FlowWorkspace");
  return { default: module.FlowWorkspace };
});

const OrchestrationWorkspace = lazy(async () => {
  const module = await import("./components/orchestration/OrchestrationWorkspace");
  return { default: module.OrchestrationWorkspace };
});

const PAGE_TITLE: Partial<Record<PageId, string>> = {
  control: "Thiết bị",
  nurture: "Nuôi TikTok",
  interaction: "Tương tác",
  material: "Kho nội dung",
  apps: "Trung tâm ứng dụng",
  scripts: "Flow",
  jobs: "Tác vụ",
  publish: "Đăng bài",
  diagnostics: "Chẩn đoán",
  data: "Dữ liệu",
  api: "API",
  settings: "Cài đặt",
};

type DeviceWorkOwnerProjection =
  | { state: "loading" }
  | { state: "known"; owners: Map<string, DeviceWorkOwner | null> }
  | { state: "error"; message: string };

type PendingNavigation =
  | { kind: "page"; value: PageId; settle: (activated: boolean) => void }
  | {
      kind: "automationView";
      value: "device" | "orchestration";
      settle: (activated: boolean) => void;
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
  const pageRef = useRef(page);
  pageRef.current = page;
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
  const compactViewport = useMediaQuery("(max-width: 1100px)");
  const [asideCollapsedOverride, setAsideCollapsedOverride] = useState<boolean | null>(null);
  const asideCollapsed = asideCollapsedOverride ?? compactViewport;
  const [groupTab, setGroupTab] = useState<string>(ALL_DEVICES_TAB);
  const [tileMenu, setTileMenu] = useState<{ udid: string; x: number; y: number } | null>(null);
  const [adbFor, setAdbFor] = useDeviceSurface(devices, "bảng lệnh adb");
  const [syslogFor, setSyslogFor] = useDeviceSurface(devices, "log của máy");
  const [healthFor, setHealthFor] = useDeviceSurface(devices, "bảng kiểm tra máy");
  const [detailsFor, setDetailsFor] = useDeviceSurface(devices, "chi tiết thiết bị");
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
  const [viewMode, setViewMode] = useState<ViewMode>("window");
  const [tileWidth, setTileWidth] = useState(() => loadZoom(TILE_ZOOM));
  const [groupToolsOpen, setGroupToolsOpen] = useState(false);
  const [groupsOpen, setGroupsOpen] = useState(false);
  const [flowDirty, setFlowDirty] = useState(false);
  const flowDirtyRef = useRef(false);
  const [automationView, setAutomationView] = useState<"device" | "orchestration">("device");
  const automationViewRef = useRef(automationView);
  automationViewRef.current = automationView;
  const pendingNavigationRef = useRef<PendingNavigation | null>(null);
  const navigationDrainRef = useRef<Promise<void> | null>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const [orchestrationTargetRef, setOrchestrationTargetRef] = useState<TargetRef>({ type: "all" });
  const [publishTargetRef, setPublishTargetRef] = useState<TargetRef>({ type: "all" });
  const [nurtureTargetRef, setNurtureTargetRef] = useState<TargetRef>({ type: "all" });
  const [interactionTargetRef, setInteractionTargetRef] = useState<TargetRef>({ type: "all" });
  const [deviceWorkOwners, setDeviceWorkOwners] = useState<DeviceWorkOwnerProjection>({
    state: "loading",
  });
  const [deviceWorkOwnerRetry, setDeviceWorkOwnerRetry] = useState(0);
  const [deviceSearch, setDeviceSearch] = useState("");
  const [deviceStatusFilter, setDeviceStatusFilter] = useState<DeviceOperationalFilter>("all");
  useViewClient();

  const rosterKey = devices.map((device) => device.udid).join("\u0000");
  useEffect(() => {
    if (page !== "control" || !fleetSettled) return;
    let active = true;
    let reading = false;
    setDeviceWorkOwners({ state: "loading" });
    const readOwners = () => {
      if (reading) return;
      reading = true;
      void listDeviceWorkStates()
        .then((states) => {
          if (!active) return;
          setDeviceWorkOwners({
            state: "known",
            owners: new Map(states.map((state) => [state.udid, state.currentOwner])),
          });
        })
        .catch((error) => {
          if (active) {
            setDeviceWorkOwners({ state: "error", message: describeError(error) });
          }
        })
        .finally(() => {
          reading = false;
        });
    };
    readOwners();
    const timer = window.setInterval(readOwners, 2_000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [deviceWorkOwnerRetry, fleetSettled, page, rosterKey]);

  const deviceWorkOwnerReadState: DeviceWorkOwnerReadState = deviceWorkOwners.state;
  const currentDeviceWorkOwner = useCallback(
    (udid: string): DeviceWorkOwner | null =>
      deviceWorkOwners.state === "known"
        ? (deviceWorkOwners.owners.get(udid) ?? null)
        : null,
    [deviceWorkOwners],
  );

  useEffect(() => {
    if (startupIssue !== null || !fleetSettled || bootError) return;
    void deploymentFrontendReady();
  }, [bootError, fleetSettled, startupIssue]);

  const confirmDiscardFlow = useCallback(
    () =>
      requestConfirm({
        title: "Bỏ thay đổi chưa lưu?",
        message: "Bản nháp hiện tại chưa được lưu và sẽ mất khi rời khỏi trang.",
        confirmLabel: "Bỏ thay đổi",
        cancelLabel: "Ở lại",
        danger: true,
      }),
    [],
  );

  const updateFlowDirty = useCallback((dirty: boolean) => {
    flowDirtyRef.current = dirty;
    setFlowDirty(dirty);
  }, []);

  const queueNavigation = useCallback(
    (intent: Omit<PendingNavigation, "settle">): Promise<boolean> =>
      new Promise<boolean>((settle) => {
        // One pending destination is enough. A later click supersedes the destination but
        // shares the open discard dialog, so a stale page cannot open after it is answered.
        pendingNavigationRef.current?.settle(false);
        pendingNavigationRef.current = { ...intent, settle } as PendingNavigation;
        if (navigationDrainRef.current) return;

        const drain = (async () => {
          while (pendingNavigationRef.current) {
            if (flowDirtyRef.current) {
              const confirmed = await confirmDiscardFlow();
              // Dirty state can change while the dialog awaits an answer. Read it again rather
              // than deciding from the render that opened the dialog.
              if (!pendingNavigationRef.current) return;
              if (!confirmed && flowDirtyRef.current) {
                const abandoned = pendingNavigationRef.current;
                pendingNavigationRef.current = null;
                abandoned.settle(false);
                return;
              }
              if (flowDirtyRef.current) updateFlowDirty(false);
            }

            const latest = pendingNavigationRef.current;
            pendingNavigationRef.current = null;
            if (!latest) return;
            if (contentRef.current) {
              contentRef.current.scrollTop = 0;
              contentRef.current.scrollLeft = 0;
            }
            if (latest.kind === "page") {
              pageRef.current = latest.value;
              setPage(latest.value);
            } else {
              automationViewRef.current = latest.value;
              setAutomationView(latest.value);
            }
            latest.settle(true);
          }
        })();
        navigationDrainRef.current = drain;
        void drain.finally(() => {
          if (navigationDrainRef.current === drain) navigationDrainRef.current = null;
        });
      }),
    [confirmDiscardFlow, updateFlowDirty],
  );

  const requestPage = useCallback(
    async (next: PageId) => {
      if (next === pageRef.current) {
        pendingNavigationRef.current?.settle(false);
        pendingNavigationRef.current = null;
        return;
      }
      await queueNavigation({ kind: "page", value: next });
    },
    [queueNavigation],
  );

  const requestAutomationView = useCallback(
    async (next: "device" | "orchestration") => {
      if (next === automationViewRef.current) {
        pendingNavigationRef.current?.settle(false);
        pendingNavigationRef.current = null;
        return true;
      }
      return queueNavigation({ kind: "automationView", value: next });
    },
    [queueNavigation],
  );

  const onAutomationTabKeyDown = (
    event: ReactKeyboardEvent<HTMLButtonElement>,
  ) => {
    let next: "device" | "orchestration" | null = null;
    if (event.key === "ArrowRight" || event.key === "End") next = "orchestration";
    if (event.key === "ArrowLeft" || event.key === "Home") next = "device";
    if (!next) return;
    event.preventDefault();
    void requestAutomationView(next).then((activated) => {
      if (activated) document.getElementById(`flow-mode-tab-${next}`)?.focus();
    });
  };

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
  const orderedDevices = useMemo(
    () => orderDevicesByNumber(devices, metaMap),
    [devices, metaMap],
  );
  const fleetNumberByUdid = useMemo(() => {
    const numbers = new Map<string, number>();
    orderedDevices.forEach((device, index) => {
      numbers.set(device.udid, tileNumber(index + 1, metaMap.get(device.udid)));
    });
    return numbers;
  }, [metaMap, orderedDevices]);
  const automationDeviceLabels = useMemo(() => {
    const labels = new Map<string, string>();
    orderedDevices.forEach((device, index) => {
      const meta = metaMap.get(device.udid);
      labels.set(
        device.udid,
        `Máy ${tileNumber(index + 1, meta)} · ${tileName(device, meta)}`,
      );
    });
    return labels;
  }, [metaMap, orderedDevices]);
  const publishTargetUdids = useMemo(
    () => resolveAutomationTarget(publishTargetRef, devices, groups),
    [publishTargetRef, devices, groups],
  );
  const nurtureTargetUdids = useMemo(
    () => resolveAutomationTarget(nurtureTargetRef, devices, groups),
    [nurtureTargetRef, devices, groups],
  );
  const interactionTargetUdids = useMemo(
    () => resolveAutomationTarget(interactionTargetRef, devices, groups),
    [interactionTargetRef, devices, groups],
  );
  // Numbered phones lead, in number order; an unnumbered fleet is left exactly as the
  // driver listed it. That is the point of a number — a grid position moves when a phone
  // drops off USB, a number does not.
  const visibleDevices = useMemo(
    () =>
      devicesInTab(orderedDevices, groups, groupTab).filter((device) =>
        deviceMatchesFleetFilter(
          device,
          currentDeviceWorkOwner(device.udid),
          fleetNumberByUdid.get(device.udid) ?? 1,
          tileName(device, metaMap.get(device.udid)),
          deviceSearch,
          deviceStatusFilter,
          deviceWorkOwnerReadState,
        ),
      ),
    [
      deviceSearch,
      deviceStatusFilter,
      deviceWorkOwnerReadState,
      currentDeviceWorkOwner,
      fleetNumberByUdid,
      groupTab,
      groups,
      metaMap,
      orderedDevices,
    ],
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
  const detailsDevice = useMemo(
    () => (detailsFor ? (devices.find((device) => device.udid === detailsFor) ?? null) : null),
    [detailsFor, devices],
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
   * Fleet-shaped actions stay in their purpose-built surfaces: text/file distribution and
   * macro recording live in Group Tools, task lists live in Flow, and agent repair lives in
   * Settings.
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
  const PageIcon = MENU_ICONS[page];

  if (startupIssue) {
    return (
      <main className="startup-state">
        <div className="startup-state-card">
          <h1>Riviu Manager</h1>
          <h2>Chưa sẵn sàng khởi động</h2>
          <p>
            Mở Cài đặt và kiểm tra thông tin đăng nhập trong Windows Credential Manager,
            sau đó thử lại.
          </p>
          <details aria-label="Chi tiết lỗi khởi động">
            <summary>Chi tiết lỗi</summary>
            <code>{startupIssue}</code>
          </details>
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
        onToggleCollapse={() => setAsideCollapsedOverride(!asideCollapsed)}
      />

      <div className="main-col">
        <PageHeader
          title={title}
          icon={page === "control" || !PageIcon ? undefined : <PageIcon size={18} />}
          titleTestId="page-title"
          dragRegion
          density={page === "control" ? "compact" : "default"}
          meta={
            <>
              {groupMode && <StatusChip tone="info">Sync</StatusChip>}
              {readyCount > 0 && (
                <StatusChip tone="success">{readyCount} sẵn sàng</StatusChip>
              )}
              {runningJobs > 0 && (
                <StatusChip tone="warning">{runningJobs} tác vụ</StatusChip>
              )}
            </>
          }
          actions={
            <>
              <ActivityCenter />
              <button
              type="button"
              className="icon-btn"
              title="Làm mới danh sách máy"
              aria-label="Làm mới danh sách máy"
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
              <IconRefresh size={16} aria-hidden="true" />
              </button>
            </>
          }
        />

        <div
          ref={contentRef}
          className={`content content-${page} ${page === "scripts" ? "content-flow" : ""}`}
        >
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
                deviceCount={devices.length}
                syncOn={groupMode}
                groupsOpen={groupsOpen}
                onGroups={() => {
                  setGroupToolsOpen(false);
                  setGroupsOpen((v) => !v);
                }}
                groupToolsOpen={groupToolsOpen}
                onGroupTools={() => {
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

              {deviceWorkOwners.state === "error" && (
                <Banner
                  tone="error"
                  action={
                    <button
                      type="button"
                      onClick={() => setDeviceWorkOwnerRetry((value) => value + 1)}
                    >
                      Thử lại
                    </button>
                  }
                >
                  <strong>Không đọc được tác vụ đang chạy trên thiết bị</strong>
                  <span>{deviceWorkOwners.message}</span>
                </Banner>
              )}

              {/* One row, not two: tabs on the left, size and view mode on the right.
                  The tab strip keeps its own horizontal scroll and the controls do not
                  join it — otherwise the slider scrolls away with the tabs. */}
              <div className="device-toolrow">
                <GroupTabs tabs={tabs} active={groupTab} onSelect={setGroupTab} />
                <div className="device-filters" role="search" aria-label="Lọc thiết bị">
                  <input
                    type="search"
                    aria-label="Tìm thiết bị"
                    placeholder="Tìm số máy hoặc tên"
                    value={deviceSearch}
                    onChange={(event) => setDeviceSearch(event.target.value)}
                  />
                  <select
                    aria-label="Trạng thái thiết bị"
                    value={deviceStatusFilter}
                    onChange={(event) =>
                      setDeviceStatusFilter(event.target.value as DeviceOperationalFilter)
                    }
                  >
                    <option value="all">Mọi trạng thái</option>
                    <option value="ready">Sẵn sàng</option>
                    <option value="busy">Bận</option>
                    <option value="warning">Cần xem</option>
                    <option value="offline">Ngoại tuyến</option>
                  </select>
                </div>
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

              {visibleDevices.length > 0 && viewMode === "list" && (
                <table className="device-table" aria-label="Danh sách thiết bị">
                  <caption className="visually-hidden">Danh sách thiết bị đang hiển thị</caption>
                  <thead>
                    <tr>
                      <th scope="col">
                        <span className="visually-hidden">Chọn</span>
                      </th>
                      <th scope="col">Máy</th>
                      <th scope="col">Trạng thái</th>
                      <th scope="col">Kết nối</th>
                      <th scope="col">
                        <span className="visually-hidden">Thao tác</span>
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {visibleDevices.map((device) => {
                      const sel = selected.includes(device.udid);
                      const machineNumber = fleetNumberByUdid.get(device.udid) ?? 1;
                      const meta = metaMap.get(device.udid);
                      const currentOwner = currentDeviceWorkOwner(device.udid);
                      const status = deviceOperationalView(
                        device,
                        currentOwner,
                        deviceWorkOwnerReadState,
                      );
                      const statusLabel = status.ownerLabel
                        ? `${status.label} · ${status.ownerLabel}`
                        : status.label;
                      return (
                        <tr
                          key={device.udid}
                          className={sel ? "selected" : ""}
                          tabIndex={0}
                          aria-label={`Máy ${machineNumber}, ${tileName(device, meta)}, ${statusLabel}${sel ? ", đã chọn" : ""}`}
                          onClick={(e) => onSelect(device.udid, e.metaKey || e.ctrlKey)}
                          onKeyDown={(event) => {
                            if (event.target !== event.currentTarget) return;
                            if (event.key !== "Enter" && event.key !== " ") return;
                            event.preventDefault();
                            onSelect(device.udid, event.metaKey || event.ctrlKey || event.shiftKey);
                          }}
                          onDoubleClick={() => setFocusUdid(device.udid)}
                        >
                          <td>
                            <input
                              type="checkbox"
                              aria-label={`Chọn Máy ${machineNumber}`}
                              checked={sel}
                              onChange={() => onSelect(device.udid, true)}
                              onClick={(e) => e.stopPropagation()}
                            />
                          </td>
                          <td>
                            <strong>Máy {machineNumber}</strong>
                            <span className="device-table-alias">{tileName(device, meta)}</span>
                          </td>
                          <td>
                            <span className={`chip ${status.tone}`}>
                              {statusLabel}
                            </span>
                          </td>
                          <td>{device.connection.toUpperCase()}</td>
                          <td>
                            <div className="device-row-actions">
                              <button
                                type="button"
                                className="link"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  setFocusUdid(device.udid);
                                }}
                              >
                                Mở
                              </button>
                              <button
                                type="button"
                                className="icon-button"
                                aria-label={`Xem chi tiết Máy ${machineNumber}`}
                                title="Xem chi tiết"
                                onClick={(event) => {
                                  event.stopPropagation();
                                  setDetailsFor(device.udid);
                                }}
                              >
                                <MoreHorizontal size={18} />
                              </button>
                            </div>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              )}

              {visibleDevices.length > 0 && viewMode === "window" && (
                <div
                  className="window-canvas"
                  ref={canvasRef}
                  role="listbox"
                  aria-label="Lưới thiết bị"
                  aria-multiselectable="true"
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
                  {visibleDevices.map((device) => (
                    <DeviceTile
                      key={device.udid}
                      device={device}
                      width={tileWidth}
                      index={fleetNumberByUdid.get(device.udid) ?? 1}
                      name={tileName(device, metaMap.get(device.udid))}
                      operational={deviceOperationalView(
                        device,
                        currentDeviceWorkOwner(device.udid),
                        deviceWorkOwnerReadState,
                      )}
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

              {devices.length > 0 && visibleDevices.length === 0 && (
                <EmptyState
                  icon={<IconPhone size={20} />}
                  title="Không có thiết bị phù hợp"
                  hint="Đổi nhóm, từ khóa hoặc trạng thái để xem thiết bị khác."
                />
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
              <div role="tablist" aria-label="Chế độ Flow" className="automation-tabs">
                <button
                  id="flow-mode-tab-device"
                  type="button"
                  role="tab"
                  aria-selected={automationView === "device"}
                  aria-controls="flow-mode-panel-device"
                  tabIndex={automationView === "device" ? 0 : -1}
                  onClick={() => void requestAutomationView("device")}
                  onKeyDown={onAutomationTabKeyDown}
                >
                  Flow thiết bị
                </button>
                <button
                  id="flow-mode-tab-orchestration"
                  type="button"
                  role="tab"
                  aria-selected={automationView === "orchestration"}
                  aria-controls="flow-mode-panel-orchestration"
                  tabIndex={automationView === "orchestration" ? 0 : -1}
                  onClick={() => void requestAutomationView("orchestration")}
                  onKeyDown={onAutomationTabKeyDown}
                >
                  Điều phối
                </button>
              </div>
              <div
                id="flow-mode-panel-device"
                className="automation-mode-panel"
                role="tabpanel"
                aria-labelledby="flow-mode-tab-device"
                hidden={automationView !== "device"}
              >
                {automationView === "device" && (
                  <Suspense fallback={<LoadingState label="Đang tải Flow…" />}>
                    <FlowWorkspace
                      devices={devices}
                      deviceLabel={(device) =>
                        automationDeviceLabels.get(device.udid) ?? device.name
                      }
                      selectedUdids={selected}
                      onDirtyChange={updateFlowDirty}
                    />
                  </Suspense>
                )}
              </div>
              <div
                id="flow-mode-panel-orchestration"
                className="automation-mode-panel"
                role="tabpanel"
                aria-labelledby="flow-mode-tab-orchestration"
                hidden={automationView !== "orchestration"}
              >
                {automationView === "orchestration" && (
                  <div className="automation-page-stack">
                    <TargetSelector
                      devices={devices}
                      groups={groups}
                      selected={selected}
                      onChange={setSelected}
                      targetRef={orchestrationTargetRef}
                      onTargetRefChange={setOrchestrationTargetRef}
                      deviceLabel={(device) =>
                        automationDeviceLabels.get(device.udid) ?? device.name
                      }
                    />
                    <Suspense fallback={<LoadingState label="Đang tải Điều phối…" />}>
                      <OrchestrationWorkspace
                        onDirtyChange={updateFlowDirty}
                        targetRef={orchestrationTargetRef}
                      />
                    </Suspense>
                  </div>
                )}
              </div>
            </section>
          )}
          {page === "jobs" && (
            <JobsPanel
              devices={devices}
              selectedUdids={selected}
              onSelectUdids={setSelected}
              initialScript={null}
              deviceLabels={automationDeviceLabels}
            />
          )}
          {page === "publish" && (
            <div className="automation-page-stack">
              <TargetSelector
                devices={devices}
                groups={groups}
                selected={selected}
                onChange={setSelected}
                targetRef={publishTargetRef}
                onTargetRefChange={setPublishTargetRef}
                deviceLabel={(device) => automationDeviceLabels.get(device.udid) ?? device.name}
              />
              <PublishPage
                devices={devices}
                selected={selected}
                targetUdids={publishTargetUdids}
                targetRef={publishTargetRef}
                metas={metaMap}
                onSelectUdids={setSelected}
              />
            </div>
          )}
          {page === "nurture" && (
            <div className="automation-page-stack">
              <TargetSelector
                devices={devices}
                groups={groups}
                selected={selected}
                onChange={setSelected}
                targetRef={nurtureTargetRef}
                onTargetRefChange={setNurtureTargetRef}
                deviceLabel={(device) => automationDeviceLabels.get(device.udid) ?? device.name}
              />
              <NurturePopup
                devices={devices}
                selected={selected}
                targetUdids={nurtureTargetUdids}
                targetRef={nurtureTargetRef}
                metas={metaMap}
                surface="page"
              />
            </div>
          )}
          {page === "interaction" && (
            <div className="automation-page-stack">
              <TargetSelector
                devices={devices}
                groups={groups}
                selected={selected}
                onChange={setSelected}
                targetRef={interactionTargetRef}
                onTargetRefChange={setInteractionTargetRef}
                deviceLabel={(device) => automationDeviceLabels.get(device.udid) ?? device.name}
              />
              <InteractionPopup
                devices={devices}
                selected={selected}
                targetUdids={interactionTargetUdids}
                targetRef={interactionTargetRef}
                metas={metaMap}
                surface="page"
              />
            </div>
          )}
          {page === "diagnostics" && <FleetDiagnosticsPage devices={devices} metas={metas} />}
          {page === "data" && <DataPage />}
          {page === "api" && <ApiPage />}
          {page === "settings" && (
            <SettingsPanel devices={devices} deviceLabels={automationDeviceLabels} />
          )}
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
      {detailsFor && detailsDevice && (
        <DeviceDetailsDrawer
          device={detailsDevice}
          machineLabel={`Máy ${fleetNumberByUdid.get(detailsDevice.udid) ?? 1}`}
          currentOwner={currentDeviceWorkOwner(detailsDevice.udid)}
          ownerReadFailed={deviceWorkOwners.state !== "known"}
          onClose={() => setDetailsFor(null)}
        />
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

      <ConfirmHost />
    </div>
  );
}

export default App;
