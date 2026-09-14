import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type CSSProperties,
} from "react";
import {
  agentBulkRepair,
  agentListStatuses,
  deploymentFrontendReady,
  listDeviceWorkStates,
  refreshDevices,
  saveGroup,
  setScreenRotation,
} from "./api";
import { startDevicePreview, startFleetPreview } from "./startPreview";
import { summarizeBulkRepair } from "./agentStatus";
import { requestConfirm } from "./confirmStore";
import { hasWorkspaceDrafts, requestWorkspaceLeave, useWorkspaceDirty, useWorkspaceDraft } from "./workspaceDraft";
import { readTargetDraft, writeFormDraft } from "./formDraftStorage";
import { useDeviceSurface } from "./features/devices/useDeviceSurface";
import { describeError } from "./describeError";
import { pushToast, toastError } from "./toastStore";
import { ConfirmHost } from "./components/ConfirmHost";
import { ActivityCenter } from "./components/ActivityCenter";
import { OperationSourceDetail } from "./components/OperationSourceDetail";
import { OperationProgressCenter } from "./features/operations/OperationProgressCenter";
import { DeviceTile } from "./components/DeviceTile";
import { FilterToolbar, type ViewMode } from "./components/FilterToolbar";
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
import { useDeviceWindows } from "./components/useDeviceWindows";
import { IconPhone, IconRefresh } from "./components/Icons";
import { Banner, EmptyState, LoadingState } from "./components/States";
import { AutomationWorkspace } from "./components/AutomationWorkspace";
import { isDeviceAutomation } from "./features/devices/deviceAutomation";
import { JobsPanel } from "./components/JobsPanel";
import { GroupManagerPopup } from "./components/GroupManagerPopup";
import { GroupToolsPopup } from "./components/GroupToolsPopup";
import { MacroRecordingBar } from "./components/MacroRecordingBar";
import { stopRecording } from "./macroStore";
import { ProfileToolbar } from "./components/ProfileToolbar";
import { SettingsPanel } from "./components/SettingsPanel";
import { MyAppsPage } from "./pages/MyAppsPage";
import { OperatorRecordsPage } from "./pages/OperatorRecordsPage";
import { OperatorSchedulesPage } from "./pages/OperatorSchedulesPage";
import { SavedTasksPage } from "./pages/SavedTasksPage";
import { ControlCenterRail } from "./components/ControlCenterRail";
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
import type { DeviceInfo, DeviceWorkOwner, PageId, TargetRef } from "./types";
import { MoreHorizontal } from "lucide-react";
import { MENU_ICONS } from "./components/menuIcons";
import { loadZoom, stepZoom, storeZoom, TILE_ZOOM, wheelWantsZoom } from "./zoom";
import { operationSourcePage, type OperationSourceRef } from "./operationSource";
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
  accounts: "Quản lý tài khoản", networks: "Mạng & Router", schedules: "Lịch chạy", savedTasks: "Tác vụ đã lưu", help: "Trợ giúp",
  myApps: "My Apps",
  control: "Thiết bị",
  nurture: "Nuôi TikTok",
  interaction: "Tương tác",
  material: "Kho nội dung",
  apps: "Trung tâm ứng dụng",
  scripts: "Flow",
  jobs: "Lượt chạy",
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

type NavigationIntent =
  | { kind: "page"; value: PageId; clearOperationSource?: boolean }
  | {
      kind: "automationView";
      value: "device" | "orchestration";
    };
type PendingNavigation = NavigationIntent & { settle: (activated: boolean) => void };

function App() {
  const [page, setPage] = useState<PageId>("control");
  const [operationSource, setOperationSource] = useState<OperationSourceRef>();
  const pageRef = useRef(page);
  pageRef.current = page;
  const activeAutomation = isDeviceAutomation(page) ? page : null;
  const activeAutomationRef = useRef(activeAutomation);
  activeAutomationRef.current = activeAutomation;
  const operationSourceRef = useRef(operationSource);
  operationSourceRef.current = operationSource;
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
  const [connectionFilter,setConnectionFilter]=useState<"all"|"usb"|"wifi">("all");
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
  const deviceWindows = useDeviceWindows(devices);
  const focusUdid = deviceWindows.activeUdid;
  const overlayUdid = focusUdid;
  const openDeviceWindow = deviceWindows.open;
  const setFocusUdid = useCallback((udid: string | null) => {
    const target = udid && groupMode && controlCenter && devices.some(device => device.udid === controlCenter) ? controlCenter : udid;
    openDeviceWindow(target);
  }, [openDeviceWindow, groupMode, controlCenter, devices]);
  const [viewMode, setViewMode] = useState<ViewMode>("window");
  const [settingsSection, setSettingsSection] = useState<"control" | "integration" | "maintenance" | undefined>();
  const [tileWidth, setTileWidth] = useState(() => loadZoom(TILE_ZOOM));
  const [displayPinned, setDisplayPinned] = useState(() => {
    try { return localStorage.getItem("riviu.control.displayPinned") === "true"; } catch { return false; }
  });
  const [groupToolsView, setGroupToolsView] = useState<"closed" | "dialog" | "recording">("closed");
  const [macroSession, setMacroSession] = useState<{ udids: string[] } | null>(null);
  const closeGroupTools = useCallback(() => {
    setGroupToolsView("closed");
    setMacroSession(null);
  }, []);
  const [groupsOpen, setGroupsOpen] = useState(false);
  const workspaceDirty = useWorkspaceDirty();
  const [automationView, setAutomationView] = useState<"device" | "orchestration">("device");
  const automationViewRef = useRef(automationView);
  automationViewRef.current = automationView;
  const pendingNavigationRef = useRef<PendingNavigation | null>(null);
  const navigationDrainRef = useRef<Promise<void> | null>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const [orchestrationTargetRef, setOrchestrationTargetRef] = useState<TargetRef>({ type: "explicit", udids: [] });
  const [publishTargetRef, setPublishTargetRef] = useState<TargetRef>(() => readTargetDraft("publish-target", { type: "all" }));
  const [nurtureTargetRef, setNurtureTargetRef] = useState<TargetRef>(() => readTargetDraft("nurture-target", { type: "explicit", udids: [] }));
  const [interactionTargetRef, setInteractionTargetRef] = useState<TargetRef>(() => readTargetDraft("interaction-target", { type: "explicit", udids: [] }));
  const [draftStorageError, setDraftStorageError] = useState<string | null>(null);
  useWorkspaceDraft({
    id: "automation-targets", label: "Phạm vi thiết bị", dirty: true,
    snapshotKey: JSON.stringify([publishTargetRef, nurtureTargetRef, interactionTargetRef]),
    save: async () => true, discard: () => {},
    autoSave: () => {
      writeFormDraft("publish-target", publishTargetRef);
      writeFormDraft("nurture-target", nurtureTargetRef);
      writeFormDraft("interaction-target", interactionTargetRef);
      setDraftStorageError(null);
    },
    onAutoSaveError: () => setDraftStorageError("Chưa lưu được phạm vi thiết bị. Kiểm tra dung lượng ổ đĩa rồi thử chuyển tab lại."),
  });
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

  const updateFlowDirty = useCallback((_dirty: boolean) => {}, []);

  const queueNavigation = useCallback(
    (intent: NavigationIntent): Promise<boolean> =>
      new Promise<boolean>((settle) => {
        // One pending destination is enough. A later click supersedes the destination but
        // shares the open discard dialog, so a stale page cannot open after it is answered.
        pendingNavigationRef.current?.settle(false);
        pendingNavigationRef.current = { ...intent, settle } as PendingNavigation;
        if (navigationDrainRef.current) return;

        const drain = (async () => {
          while (pendingNavigationRef.current) {
            const pending = pendingNavigationRef.current;
            const destination = pending.kind === "page" && isDeviceAutomation(pending.value) ? pending.value : null;
            // Re-selecting the same workspace preserves its mounted editor.
            const retainsEditor = destination !== null && destination === activeAutomationRef.current
              && !(pending.kind === "page" && pending.clearOperationSource && operationSourceRef.current);
            if (!retainsEditor && hasWorkspaceDrafts()) {
              const confirmed = await requestWorkspaceLeave();
              // Dirty state can change while the dialog awaits an answer. Read it again rather
              // than deciding from the render that opened the dialog.
              if (!pendingNavigationRef.current) return;
              if (!confirmed) {
                const abandoned = pendingNavigationRef.current;
                pendingNavigationRef.current = null;
                abandoned.settle(false);
                return;
              }
            }

            const latest = pendingNavigationRef.current;
            pendingNavigationRef.current = null;
            if (!latest) return;
            if (contentRef.current) {
              contentRef.current.scrollTop = 0;
              contentRef.current.scrollLeft = 0;
            }
            if (latest.kind === "page") {
              if (latest.clearOperationSource) setOperationSource(undefined);
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
    [],
  );

  const requestPage = useCallback(
    async (next: PageId, clearOperationSource = false) => {
      // Leaving history is a real navigation even when the sidebar destination is this page.
      if (next === pageRef.current && !(clearOperationSource && operationSource)) {
        pendingNavigationRef.current?.settle(false);
        pendingNavigationRef.current = null;
        return true;
      }
      return queueNavigation({ kind: "page", value: next, clearOperationSource });
    },
    [operationSource, queueNavigation],
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



  const openOperationSource = useCallback(async (source: OperationSourceRef) => {
    const destination = operationSourcePage(source);
    if (!(await requestPage(destination)) || pageRef.current !== destination) return;
    if (source.kind === "flow" || source.kind === "orchestration") {
      if (!(await requestAutomationView(source.kind === "flow" ? "device" : "orchestration"))) return;
    }
    if (pageRef.current !== destination) return;
    setOperationSource(source);
  }, [requestPage, requestAutomationView]);

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
    if (!workspaceDirty) return;
    const preventUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
    };
    window.addEventListener("beforeunload", preventUnload);
    return () => window.removeEventListener("beforeunload", preventUnload);
  }, [workspaceDirty]);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    let stopped = false;
    let closing = false;
    void import("@tauri-apps/api/window").then(async ({ getCurrentWindow }) => {
      const window = getCurrentWindow();
      const unlisten = await window.onCloseRequested(async (event) => {
        if (closing || !hasWorkspaceDrafts()) return;
        event.preventDefault();
        if (await requestWorkspaceLeave()) {
          closing = true;
          await window.close();
        }
      });
      if (stopped) unlisten(); else dispose = unlisten;
    }).catch(() => { /* Browser-only fixture uses beforeunload. */ });
    return () => { stopped = true; dispose?.(); };
  }, []);

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
  const workspaceScope = activeAutomation === "nurture"
    ? { targetRef: nurtureTargetRef, targetUdids: nurtureTargetUdids, setTarget: setNurtureTargetRef }
    : activeAutomation === "interaction"
      ? { targetRef: interactionTargetRef, targetUdids: interactionTargetUdids, setTarget: setInteractionTargetRef }
      : { targetRef: publishTargetRef, targetUdids: publishTargetUdids, setTarget: setPublishTargetRef };
  // Numbered phones lead, in number order; an unnumbered fleet is left exactly as the
  // driver listed it. That is the point of a number — a grid position moves when a phone
  // drops off USB, a number does not.
  const visibleDevices = useMemo(
    () =>
      devicesInTab(orderedDevices, groups, groupTab).filter((device) => (connectionFilter==="all"||device.connection===connectionFilter) &&
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
      connectionFilter,
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
  const selectDevice = (udid:string,additive:boolean) => {
    onSelect(udid,additive);
    if(!additive&&focusUdid&&focusUdid!==udid)setFocusUdid(udid);
  };
  const hasVisibleDevices = visibleDevices.length > 0;

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
  }, [page, viewMode, canvasRef, hasVisibleDevices]);
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

  const beginMacroSession = (targets: string[]) => {
    setMacroSession(current => current ?? {
      udids: [...targets],
    });
    setGroupToolsView("recording");
  };
  const stopMacroSession = useCallback(() => {
    stopRecording();
    setGroupToolsView("dialog");
  }, []);
  const restoreToolsFocus = () => document.querySelector<HTMLElement>(".focus-menu-head > button.close")
    ?? document.querySelector<HTMLElement>(".profile-toolbar button[data-group-tools]")
    ?? document.querySelector<HTMLElement>('.menu-item[aria-current="page"]');
  const recordingControls = groupToolsView === "recording" ? <MacroRecordingBar onStop={stopMacroSession} /> : null;

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
        selectedCount={selected.length}
        total={devices.length}
        readyCount={readyCount}
        groupMode={groupMode}
        onPage={(next) => void requestPage(next, true)}
      />

      <div className="main-col">
        <PageHeader
          title={title}
          icon={!PageIcon ? undefined : <PageIcon size={18} />}
          titleTestId="page-title"
          dragRegion
          density="default"
          meta={
            <>
              <span className="fleet-status-label">Toàn hệ thống</span>
              {groupMode && <StatusChip>Đồng bộ bật</StatusChip>}
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
              {page !== "control" && <button
              type="button"
              className="icon-btn"
              title="Làm mới danh sách máy"
              aria-label="Làm mới danh sách máy"
              onClick={async () => {
                try {
                  await refreshDevices();
                  await reload();
                } catch (error) {
                  toastError("Không làm mới được danh sách máy", error);
                }
              }}
            >
              <IconRefresh size={16} aria-hidden="true" />
              </button>}
            </>
          }
        />

        {!focusDevice && recordingControls}
        <OperationProgressCenter deviceLabels={automationDeviceLabels} />
        <div
          ref={contentRef}
          className={`content content-${page} ${page === "scripts" ? "content-flow" : ""}`}
        >
          {draftStorageError && <Banner tone="error">{draftStorageError}</Banner>}
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

          {driverIssue && page === "control" && devices.some(device => device.platform === "ios" && device.status !== "disconnected") && (
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
            <section className={`device-browser${displayPinned ? "" : " rail-unpinned"}`} aria-label="Danh sách và màn hình thiết bị">
              <ControlCenterRail tileWidth={tileWidth} onTileWidth={setTileWidth} connection={connectionFilter} onConnection={setConnectionFilter}
                pinned={displayPinned} onPinnedChange={next => { setDisplayPinned(next); try { localStorage.setItem("riviu.control.displayPinned", String(next)); } catch { /* optional preference */ } }}
                groups={tabs.map(tab => ({ ...tab, udids: groups.find(group => group.id === tab.id)?.udids }))} group={groupTab} onGroup={setGroupTab}
                machines={devices.map(d=>({id:d.udid,number:fleetNumberByUdid.get(d.udid)??1,name:tileName(d,metaMap.get(d.udid)),selected:selected.includes(d.udid)}))}
                onSelect={id=>onSelect(id,true)} onSettings={()=>{setSettingsSection("control");void requestPage("settings");}}
                onGroups={()=>setGroupsOpen(true)} onRotate={()=>void(async()=>{const targets=selectedDevices.filter(device=>device.platform==="android");if(!targets.length){pushToast("info","Chọn máy Android để xoay");return;}const results=await Promise.allSettled(targets.map(device=>setScreenRotation(device.udid,1)));const confirmed=results.filter(result=>result.status==="fulfilled"&&result.value===1).length;pushToast(confirmed===targets.length?"ok":"warn",`Đã xác nhận xoay ${confirmed}/${targets.length} máy`);})()}/>
              <div className="device-browser-toolbar">
              <ProfileToolbar
                selected={selectedDevices}
                deviceCount={devices.length}
                syncOn={groupMode}
                groupsOpen={groupsOpen}
                onGroups={() => {
                  if (groupToolsView !== "recording") closeGroupTools();
                  setGroupsOpen((v) => !v);
                }}
                groupToolsOpen={groupToolsView === "dialog"}
                onGroupTools={() => {
                  if (groupToolsView === "recording") {
                    document.querySelector<HTMLElement>(".macro-recording-bar button")?.focus();
                    return;
                  }
                  setGroupsOpen(false);
                  if (groupToolsView === "dialog") closeGroupTools();
                  else setGroupToolsView("dialog");
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

              {/* The rail owns group selection; the dock keeps its compact tab strip. */}
              <div className="device-toolrow">
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
              </div>

              <div className="device-browser-content">

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
                          onClick={(e) => selectDevice(device.udid, e.metaKey || e.ctrlKey)}
                          onKeyDown={(event) => {
                            if (event.target !== event.currentTarget) return;
                            if (event.key !== "Enter" && event.key !== " ") return;
                            event.preventDefault();
                            selectDevice(device.udid, event.metaKey || event.ctrlKey || event.shiftKey);
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
                  style={{ "--device-tile-min-width": `${tileWidth}px` } as CSSProperties}
                  ref={canvasRef}
                  role="grid"
                  aria-label="Lưới thiết bị"
                  aria-multiselectable="true"
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
                      focused={overlayUdid === device.udid}
                      controlCenter={controlCenter === device.udid}
                      onSelect={selectDevice}
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
              </div>
            </section>
          )}

          {page === "material" && (
            <MaterialPage
              operationSource={operationSource?.kind === "materialTransfer" ? operationSource : undefined}
              devices={devices}
              selected={selected}
              onSelectUdids={setSelected}
            />
          )}
          {page === "apps" && (
            <AppsPage
              operationSource={operationSource?.kind === "appInstall" ? operationSource : undefined}
              devices={devices}
              selected={selected}
              onSelectUdids={setSelected}
            />
          )}
          {page === "myApps" && <MyAppsPage devices={devices} onOpenApp={(next) => void requestPage(next)}/>}
          {page === "scripts" && (
            <section className="automation-surface">
              {(operationSource?.kind === "flow" || operationSource?.kind === "orchestration") && (
                <details className="admin-detail" open>
                  <summary>Tác vụ được chọn</summary>
                  <OperationSourceDetail source={operationSource} />
                </details>
              )}
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
                      requireChoice
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
              onOpenSource={(source) => void openOperationSource(source)}
              devices={devices}
              selectedUdids={selected}
              onSelectUdids={setSelected}
              initialScript={null}
              deviceLabels={automationDeviceLabels}
            />
          )}
          <section key="automation-workspace" hidden={!activeAutomation} className="automation-host">
            {activeAutomation && <>
              <AutomationWorkspace key={activeAutomation} kind={activeAutomation} docked={false} devices={devices} groups={groups}
                selected={selected} targetRef={workspaceScope.targetRef} targetUdids={workspaceScope.targetUdids}
                onTargetRefChange={workspaceScope.setTarget} metas={metaMap} labels={automationDeviceLabels}
                onSelectUdids={setSelected}
                operationSource={operationSource?.kind === activeAutomation ? operationSource : undefined} />
            </>}
          </section>
          {page === "diagnostics" && <FleetDiagnosticsPage devices={devices} metas={metas} />}
          {page === "accounts" && <OperatorRecordsPage kind="account" devices={devices} />}
          {page === "networks" && <OperatorRecordsPage kind="network" devices={devices} />}
          {page === "schedules" && <OperatorSchedulesPage />}
          {page === "savedTasks" && <SavedTasksPage devices={devices} />}
          {page === "help" && <section className="operator-help"><h2>Bắt đầu với Riviu Manager</h2><ol><li>Control Center: kết nối, chia nhóm và mở các cửa sổ điện thoại.</li><li>My Apps: mở ứng dụng, chỉnh cấu hình và quy trình từng bước.</li><li>Tác vụ đã lưu: giữ cấu hình và phạm vi máy để dùng lại.</li><li>Lịch chạy và Lượt chạy: đặt giờ và theo dõi kết quả từng thiết bị.</li></ol><button type="button" onClick={()=>void requestPage("diagnostics")}>Kiểm tra thiết bị</button><button type="button" onClick={()=>void requestPage("api")}>Tham chiếu API</button></section>}
          {page === "data" && <DataPage />}
          {page === "api" && <ApiPage onOpenSettings={() => {
            setSettingsSection("integration");
            void requestPage("settings", true);
          }} />}
          {page === "settings" && (
            <SettingsPanel devices={devices} deviceLabels={automationDeviceLabels} initialSection={settingsSection} iosRuntimeIssue={driverIssue} />
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

      {deviceWindows.openUdids.map((udid, windowOrder) => {
        const device = devices.find(candidate => candidate.udid === udid);
        if (!device) return null;
        return (
        <FocusStream
          key="active-device-window"
          device={device}
          active={focusUdid === udid}
          windowOrder={windowOrder}
          onActivate={() => deviceWindows.activate(udid)}
          index={fleetNumberByUdid.get(udid) ?? 1}
          onClose={() => deviceWindows.close(udid)}
          groupUdids={selected}
          groupMode={groupMode && focusDevice?.udid === udid}
          // The same array `index` above is computed from, so the picker's numbering and the
          // header's cannot disagree about which phone is #3.
          devices={devices}
          onSelectDevice={setFocusUdid}
          // The same catalog the tile's right-click menu gets. Zooming into a phone is a
          // different *view* of it, not a smaller set of things you can do to it.
          functions={tileActions(device)}
          recordingControls={focusUdid === udid ? recordingControls : null}
        />
      );})}

      {page === "control" && groupsOpen && (
        <GroupManagerPopup
          devices={devices}
          groups={groups}
          metas={metaMap}
          onChanged={reload}
          onClose={() => setGroupsOpen(false)}
        />
      )}

      {groupToolsView !== "closed" && (page === "control" || macroSession !== null) && (
        <GroupToolsPopup
          devices={devices}
          selected={selected}
          onClose={closeGroupTools}
          visible={groupToolsView === "dialog"}
          macroTargets={macroSession?.udids}
          macroOnly={page !== "control" && macroSession !== null}
          onBeginMacro={beginMacroSession}
          restoreFocus={restoreToolsFocus}
        />
      )}

      <ConfirmHost />
    </div>
  );
}

export default App;
