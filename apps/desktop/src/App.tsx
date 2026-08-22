import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react";
import {
  agentBulkRepair,
  agentListStatuses,
  androidUnavailableReason,
  deviceGetClipboard,
  deviceKey,
  deviceSetClipboard,
  deviceShell,
  deviceSwipe,
  driverDegradedReason,
  getDeviceMeta,
  disableWifiAdb,
  enableWifiAdb,
  exportMedia,
  importMedia,
  launchDeviceApp,
  listenRiviuEvents,
  installIpa,
  listDevices,
  listDeviceMetas,
  listGroups,
  listInstalledApps,
  listJobs,
  openSystemSettings,
  powerOffDevice,
  rebootDevice,
  refreshDevices,
  resetDisplayMetrics,
  saveDeviceMeta,
  saveGroup,
  screenshot,
  screenshotToDevice,
  setInputMethod,
  setScreenLocked,
  setScreenRotation,
  setWifiRadio,
  retryStartup,
  startupError,
  viewSetPreset,
  wakeScreen,
} from "./api";
import { startDevicePreview, startFleetPreview } from "./startPreview";
import { summarizeBulkRepair } from "./agentStatus";
import { requestConfirm, requestPrompt } from "./confirmStore";
import { describeError, pushToast, toastError } from "./toastStore";
import { ConfirmHost } from "./components/ConfirmHost";
import { ToastHost } from "./components/ToastHost";
import { DeviceTile } from "./components/DeviceTile";
import { FilterToolbar, type ViewMode } from "./components/FilterToolbar";
import { GroupTabs } from "./components/GroupTabs";
import { DeviceContextMenu } from "./components/DeviceContextMenu";
import { DeviceFilesPopup } from "./components/DeviceFilesPopup";
import type { DeviceMenuNode } from "./deviceMenu";
import {
  metaByUdid,
  orderDevicesByNumber,
  parseDeviceNumber,
  tileName,
  tileNumber,
} from "./deviceNaming";
import { parseCurrentInputMethod, parseInputMethods } from "./imeList";
import { AdbConsole } from "./components/AdbConsole";
import { ALL_DEVICES_TAB, devicesInTab, groupTabs, withDeviceAdded } from "./deviceGroups";
import { FocusStream } from "./components/FocusStream";
import {
  IconApp,
  IconCamera,
  IconChevronRight,
  IconClock,
  IconCopy,
  IconGrid,
  IconImage,
  IconKeyboard,
  IconPhone,
  IconPower,
  IconRefresh,
  IconSettings,
  IconSync,
  IconText,
  IconUpload,
  IconUsers,
} from "./components/Icons";
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
import type {
  DeviceGroup,
  DeviceInfo,
  DeviceMeta,
  HardwareKey,
  JobRecord,
  PageId,
} from "./types";
import { deviceModelOsLabel } from "./types";
import { pickDirectory, pickFile } from "./pickFile";
import { loadZoom, stepZoom, storeZoom, TILE_ZOOM, wheelWantsZoom } from "./zoom";
import {
  applyBoxSelection,
  isDragMeaningful,
  normalizeBox,
  tilesInBox,
  type Rect,
  type TileRect,
} from "./boxSelect";
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
  const [asideCollapsed, setAsideCollapsed] = useState(false);
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  /// The operator's own records for the fleet: what each phone is called and its number
  /// (xiaowei "Change Name" / "Change Number"). Only edited phones have a row, so this is
  /// normally shorter than the device list and often empty.
  const [metas, setMetas] = useState<DeviceMeta[]>([]);
  const [groupTab, setGroupTab] = useState<string>(ALL_DEVICES_TAB);
  const [tileMenu, setTileMenu] = useState<{ udid: string; x: number; y: number } | null>(null);
  const [adbFor, setAdbFor] = useState<string | null>(null);
  /// Which phone's filesystem is open in the browser popup (xiaowei "Preview Mobile Files").
  const [filesFor, setFilesFor] = useState<string | null>(null);
  const [jobs, setJobs] = useState<JobRecord[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
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
  const [bootError, setBootError] = useState<string | null>(null);
  const [driverIssue, setDriverIssue] = useState<string | null>(null);
  const [androidIssue, setAndroidIssue] = useState<string | null>(null);
  const [startupIssue, setStartupIssue] = useState<string | null | undefined>(undefined);
  /// Bumped by the retry button, and read by the boot effect below as a reason to run
  /// again. A counter rather than `startupIssue` in that effect's dependencies: the
  /// effect *sets* the issue, so depending on it makes every ordinary startup run the
  /// whole thing twice — two `startup_error` calls, two subscriptions, one of them
  /// immediately torn down.
  const [startupAttempt, setStartupAttempt] = useState(0);
  const [retryingStartup, setRetryingStartup] = useState(false);
  const [viewMode, setViewMode] = useState<ViewMode>("window");
  const [tileWidth, setTileWidth] = useState(() => loadZoom(TILE_ZOOM));
  const canvasRef = useRef<HTMLDivElement | null>(null);
  // Rubber-band (box) selection over the window grid (A7). `bandOrigin` holds the mousedown
  // point and modifier; `band` is the live rectangle in client coords, non-null only while
  // dragging (which is also the effect's on/off signal).
  const bandOrigin = useRef<{ x: number; y: number; additive: boolean } | null>(null);
  const [band, setBand] = useState<Rect | null>(null);
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
  const metaMap = useMemo(() => metaByUdid(metas), [metas]);
  // Numbered phones lead, in number order; an unnumbered fleet is left exactly as the
  // driver listed it. That is the point of a number — a grid position moves when a phone
  // drops off USB, a number does not.
  const visibleDevices = useMemo(
    () => orderDevicesByNumber(devicesInTab(devices, groups, groupTab), metaMap),
    [devices, groups, groupTab, metaMap],
  );
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

  // Start a marquee only from empty canvas space with the left button; a mousedown that
  // lands on a tile is that tile's own click, not a selection box.
  const onCanvasMouseDown = (event: ReactMouseEvent<HTMLDivElement>) => {
    if (event.button !== 0 || event.target !== event.currentTarget) return;
    // Stops the browser starting a *text* selection under the marquee. Without it, dragging a
    // box across the grid highlighted the captions it passed over — the tile number, model and
    // OS came back blue — so the gesture that selects phones also looked like it was selecting
    // words. `preventDefault` here covers the whole drag; `user-select: none` on the tile
    // covers a stray drag that begins inside one.
    event.preventDefault();
    bandOrigin.current = {
      x: event.clientX,
      y: event.clientY,
      additive: event.shiftKey || event.ctrlKey || event.metaKey,
    };
    setBand(normalizeBox(event.clientX, event.clientY, event.clientX, event.clientY));
  };

  // While a marquee is live, track the pointer on `window` (not the canvas) so a drag that
  // leaves the grid still updates and still commits on release. Attaches once per drag.
  const dragging = band !== null;
  useEffect(() => {
    if (!dragging) return;
    const onMove = (event: MouseEvent) => {
      const origin = bandOrigin.current;
      if (origin) setBand(normalizeBox(origin.x, origin.y, event.clientX, event.clientY));
    };
    const onUp = (event: MouseEvent) => {
      const origin = bandOrigin.current;
      bandOrigin.current = null;
      setBand(null);
      const canvas = canvasRef.current;
      if (!origin || !canvas) return;
      if (!isDragMeaningful(origin.x, origin.y, event.clientX, event.clientY)) return;
      const box = normalizeBox(origin.x, origin.y, event.clientX, event.clientY);
      // `.dev-phone[data-udid]`, not `[data-udid]`: a tile carries that attribute on three
      // elements — the article, `PhoneCanvas`'s host div, and the canvas once a stream
      // attaches — so the bare selector returned the same phone two or three times and the
      // selection held duplicates. Measured on the 20-phone fleet: a box over three tiles
      // gave the toolbar 3 and the sidebar 6. `tilesInBox` de-duplicates as well, because a
      // duplicated udid reaches `group_input` and sends every group action to that phone twice.
      const tiles: TileRect[] = Array.from(
        canvas.querySelectorAll<HTMLElement>(".dev-phone[data-udid]"),
      ).map((el) => {
        const r = el.getBoundingClientRect();
        return {
          udid: el.dataset.udid ?? "",
          rect: { left: r.left, top: r.top, right: r.right, bottom: r.bottom },
        };
      });
      const hits = tilesInBox(box, tiles);
      setSelected((prev) => applyBoxSelection(prev, hits, origin.additive));
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [dragging]);

  // Ctrl/Cmd+A selects every phone in the current tab while the grid is up — the farm
  // shortcut from xiaowei. Ignored while typing in a field so it never steals the browser's
  // own select-all inside an input.
  useEffect(() => {
    if (page !== "control" || viewMode !== "window") return;
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "a") return;
      const target = event.target as HTMLElement | null;
      const tag = target?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || target?.isContentEditable) return;
      event.preventDefault();
      setSelected(visibleDevices.map((device) => device.udid));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [page, viewMode, visibleDevices]);

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
      // Same reasoning as the groups above, and the same failure mode to avoid: a records
      // read that throws must cost the grid its labels, never its phones.
      setMetas(await listDeviceMetas().catch(() => []));
      setBootError(null);
      // An empty list can mean "nothing plugged in" or "the device sidecar never
      // started". Ask which, so the UI does not report the wrong one.
      setDriverIssue(await driverDegradedReason().catch(() => null));
      // Asked separately, because the two halves of the fleet fail for different
      // reasons and an Android phone that never appears used to say nothing at all.
      setAndroidIssue(await androidUnavailableReason().catch(() => null));
    } catch (e) {
      setBootError(describeError(e));
    }
  }, []);

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
    (device: DeviceInfo): DeviceMenuNode[] => {
      const notifyRotation = (asked: number) => (observed: number) => {
        // The backend returns the rotation the phone actually settled at, which is often
        // not the one asked for: a portrait-locked app wins, and on this farm that is
        // TikTok. Saying "rotated" regardless would be the button that lies.
        if (observed === asked) pushToast("ok", "Đã quay màn hình");
        else
          pushToast(
            "warn",
            "Máy không quay",
            "App đang mở khoá hướng dọc nên hệ thống bỏ qua yêu cầu.",
          );
      };
      /// A direction swipe in a made-up 1000×1000 frame, which the backend scales onto the
      /// phone's real pixels (`swipe_image`). Resolution-independent on purpose: reading
      /// `wm size` first would be a second adb call per swipe for a number the backend
      /// already knows.
      const swipe = (label: string, from: [number, number], to: [number, number]) => ({
        id: `swipe-${label}`,
        label,
        androidOnly: true,
        keywords: "swipe vuot",
        run: () => {
          void deviceSwipe(device.udid, from[0], from[1], to[0], to[1], 1000, 1000)
            .then(() => pushToast("ok", label))
            .catch((error) => toastError(`${label} thất bại`, error));
        },
      });
      const key = (label: string, hardware: HardwareKey, keywords?: string) => ({
        id: `key-${hardware}`,
        label,
        androidOnly: true,
        keywords,
        run: () => {
          void deviceKey(device.udid, hardware)
            .then(() => pushToast("ok", label))
            .catch((error) => toastError(`${label} thất bại`, error));
        },
      });

      /// Save one field of this phone's record, reading the row back first so an edit to the
      /// name cannot wipe the number (or the TikTok handle) that lives in the same row.
      const patchMeta = async (patch: Partial<DeviceMeta>, done: string) => {
        try {
          const current = await getDeviceMeta(device.udid);
          await saveDeviceMeta({ ...current, ...patch });
          setMetas(await listDeviceMetas().catch(() => metas));
          pushToast("ok", done);
        } catch (error) {
          toastError("Lưu không thành công", error);
        }
      };

      return [
        {
          id: "open",
          label: "Mở điều khiển",
          Icon: IconPhone,
          keywords: "control mo",
          run: () => setFocusUdid(device.udid),
        },
        {
          id: "rename",
          label: "Đổi tên máy…",
          Icon: IconText,
          keywords: "change name doi ten",
          run: () => {
            void (async () => {
              const answer = await requestPrompt({
                title: `Đổi tên ${device.name}`,
                // Said plainly, because the reference product's identically-named row does
                // change the phone, and an operator coming from it will expect that.
                message:
                  "Tên này chỉ dùng trong Riviu để phân biệt các máy giống nhau; máy không bị đổi tên. Để trống để dùng lại tên máy báo về.",
                initial: metaMap.get(device.udid)?.alias ?? "",
                placeholder: device.name,
                confirmLabel: "Lưu tên",
              });
              if (answer === null) return;
              await patchMeta(
                { alias: answer },
                answer ? `Đã đổi tên thành “${answer}”` : "Đã bỏ tên riêng",
              );
            })();
          },
        },
        {
          id: "renumber",
          label: "Đổi số máy…",
          Icon: IconGrid,
          keywords: "change number doi so",
          run: () => {
            void (async () => {
              const current = metaMap.get(device.udid)?.number;
              const answer = await requestPrompt({
                title: `Đổi số của ${device.name}`,
                message:
                  "Số này là số ghi trên máy / trên kệ. Máy có số xếp lên đầu lưới theo thứ tự số. Để trống để bỏ số.",
                initial: current === null || current === undefined ? "" : String(current),
                placeholder: "ví dụ: 21",
                numeric: true,
                confirmLabel: "Lưu số",
              });
              if (answer === null) return;
              const parsed = parseDeviceNumber(answer);
              if ("error" in parsed) {
                pushToast("warn", "Số máy không hợp lệ", parsed.error);
                return;
              }
              await patchMeta(
                { number: parsed.number },
                parsed.number === null ? "Đã bỏ số máy" : `Đã đặt số máy ${parsed.number}`,
              );
            })();
          },
        },
        {
          // "Đặt làm trung tâm điều khiển" was asked about directly — "là sao?" — which is the
          // answer: a label naming a *concept* the product invented explains nothing. What it
          // does is pick which phone the overlay drives while Sync is on, so the label says
          // that and the toast says the rest at the moment it can be acted on.
          id: "control-center",
          label:
            controlCenter === device.udid
              ? "Bỏ làm máy chính khi bật Sync"
              : "Đặt làm máy chính khi bật Sync",
          Icon: IconUsers,
          keywords: "sync may chinh trung tam dieu khien master",
          run: () => {
            const taking = controlCenter !== device.udid;
            setControlCenter(taking ? device.udid : null);
            if (taking) {
              pushToast(
                "ok",
                `${device.name} là máy chính`,
                groupMode
                  ? "Bật Sync rồi mở bất kỳ máy nào cũng ra màn hình của máy này; mọi máy đã chọn làm theo thao tác trên đó."
                  : "Sync đang TẮT nên chưa có tác dụng. Bật Sync ở thanh trên, rồi mọi máy đã chọn sẽ làm theo máy này.",
              );
            } else {
              pushToast("ok", "Đã bỏ máy chính", "Mở máy nào thì điều khiển đúng máy đó.");
            }
          },
        },
        {
          id: "apps",
          label: "Ứng dụng trên máy",
          Icon: IconApp,
          keywords: "app list ung dung",
          // Lazy: the list is one adb call per phone and nobody wants twenty of them fired
          // because a menu opened. Opening this row is the operator asking for it.
          loadChildren: async () => {
            const apps = await listInstalledApps(device.udid);
            const rows = apps
              .filter((app) => app.kind === "user")
              .sort((a, b) => a.bundleId.localeCompare(b.bundleId));
            if (rows.length === 0) {
              return [
                {
                  id: "apps-empty",
                  label: "Máy không báo ứng dụng nào do người dùng cài",
                  disabled: true,
                },
              ];
            }
            return rows.map((app) => ({
              id: `app-${app.bundleId}`,
              // The phone's own name when the helper could read one, and the bundle id when
              // it could not — never a prettified guess. See `InstalledApp`.
              label: app.label ?? app.bundleId,
              // The phone's own icon, drawn at the size the row asks for. A row with no icon
              // renders without one rather than with a stand-in.
              Icon: app.iconPngBase64
                ? ({ size = 16 }: { size?: number }) => (
                    <img
                      src={`data:image/png;base64,${app.iconPngBase64}`}
                      alt=""
                      width={size}
                      height={size}
                      style={{ borderRadius: 4, flex: "0 0 auto" }}
                    />
                  )
                : undefined,
              run: () => {
                void launchDeviceApp(device.udid, app.bundleId)
                  .then(() => pushToast("ok", "Đã mở app", app.bundleId))
                  .catch((error) => toastError("Mở app thất bại", error));
              },
            }));
          },
        },
        {
          id: "files",
          label: "Tệp trên máy…",
          Icon: IconUpload,
          androidOnly: true,
          keywords: "file explorer quan ly tep preview",
          run: () => setFilesFor(device.udid),
        },
        {
          id: "screenshot",
          label: "Chụp màn hình về máy tính",
          Icon: IconCamera,
          keywords: "screenshot chup",
          run: () => {
            void screenshot(device.udid)
              .then((path) => pushToast("ok", "Đã lưu ảnh", path))
              .catch((error) => toastError("Chụp màn hình thất bại", error));
          },
        },
        {
          id: "screenshot-device",
          label: "Chụp màn hình lưu vào máy",
          Icon: IconImage,
          androidOnly: true,
          keywords: "screenshot to phone chup",
          run: () => {
            void screenshotToDevice(device.udid)
              .then((path) => pushToast("ok", "Đã lưu ảnh vào máy", path))
              .catch((error) => toastError("Chụp vào máy thất bại", error));
          },
        },
        {
          id: "clipboard",
          label: "Clipboard",
          Icon: IconCopy,
          androidOnly: true,
          keywords: "clipboard bo nho tam",
          children: [
            {
              id: "clipboard-read",
              label: "Đọc clipboard của máy",
              androidOnly: true,
              keywords: "export clipboard",
              run: () => {
                void deviceGetClipboard(device.udid)
                  .then(async (read) => {
                    // Three outcomes, not two. Measured 21/08/2026: a phone with nothing
                    // copied answers `plaintext, 0 byte`, and calling that "not text" — as
                    // the first version did — reads like a fault in the phone rather than an
                    // empty clipboard.
                    if (read.bytes === 0) {
                      pushToast("warn", "Clipboard của máy đang rỗng");
                      return;
                    }
                    if (!read.text) {
                      pushToast(
                        "warn",
                        "Clipboard của máy không phải chữ",
                        `${read.contentType}, ${read.bytes} byte`,
                      );
                      return;
                    }
                    await navigator.clipboard.writeText(read.text);
                    // The content itself in the toast body: the operator asked to see it,
                    // and "copied 41 bytes" is not seeing it.
                    pushToast("ok", "Đã lấy clipboard về máy tính", read.text.slice(0, 200));
                  })
                  .catch((error) => toastError("Đọc clipboard thất bại", error));
              },
            },
            {
              id: "clipboard-write",
              label: "Ghi clipboard máy tính sang máy",
              androidOnly: true,
              run: () => {
                void navigator.clipboard
                  .readText()
                  .then(async (text) => {
                    if (!text) {
                      pushToast("warn", "Clipboard máy tính đang rỗng");
                      return;
                    }
                    await deviceSetClipboard(device.udid, text);
                    pushToast("ok", "Đã ghi clipboard sang máy", text.slice(0, 200));
                  })
                  .catch((error) => toastError("Ghi clipboard thất bại", error));
              },
            },
          ],
        },
        {
          id: "keyboard",
          label: "Đổi bàn phím",
          Icon: IconKeyboard,
          androidOnly: true,
          keywords: "input method ime ban phim",
          loadChildren: async () => {
            // The phone's own list, parsed rather than composed — see `imeList.ts`. The ids
            // are only ever handed back verbatim.
            const listed = await deviceShell(device.udid, "ime list -s");
            const current = parseCurrentInputMethod(
              (await deviceShell(device.udid, "settings get secure default_input_method")).stdout,
            );
            const methods = parseInputMethods(listed.stdout);
            if (methods.length === 0) {
              return [{ id: "ime-empty", label: "Máy không báo bàn phím nào", disabled: true }];
            }
            return methods.map((method) => ({
              id: `ime-${method.id}`,
              label: method.id === current ? `${method.label} (đang dùng)` : method.label,
              run: () => {
                void setInputMethod(device.udid, method.id)
                  .then(() => pushToast("ok", "Đã đổi bàn phím", method.label))
                  .catch((error) => toastError("Đổi bàn phím thất bại", error));
              },
            }));
          },
        },
        {
          id: "gestures",
          label: "Thao tác",
          Icon: IconChevronRight,
          androidOnly: true,
          keywords: "swipe key thao tac",
          children: [
            key("Home", "home"),
            key("Back", "back"),
            key("Đa nhiệm", "recents", "recents"),
            key("Thông báo", "notification", "notification"),
            key("Âm lượng +", "volumeUp", "volume up"),
            key("Âm lượng −", "volumeDown", "volume down"),
            swipe("Vuốt lên", [500, 750], [500, 250]),
            swipe("Vuốt xuống", [500, 250], [500, 750]),
            swipe("Vuốt trái", [750, 500], [250, 500]),
            swipe("Vuốt phải", [250, 500], [750, 500]),
            {
              id: "wake",
              label: "Bật màn hình",
              androidOnly: true,
              keywords: "turn on screen wake",
              run: () => {
                void wakeScreen(device.udid)
                  .then(() => pushToast("ok", "Đã bật màn hình"))
                  .catch((error) => toastError("Bật màn hình thất bại", error));
              },
            },
            {
              id: "lock",
              label: "Khoá màn hình",
              keywords: "lock khoa",
              run: () => {
                void setScreenLocked(device.udid, true)
                  .then(() => pushToast("ok", "Đã khoá màn hình"))
                  .catch((error) => toastError("Khoá màn hình thất bại", error));
              },
            },
            {
              id: "unlock",
              label: "Mở khoá màn hình",
              keywords: "unlock mo khoa",
              run: () => {
                void setScreenLocked(device.udid, false)
                  .then(() => pushToast("ok", "Đã mở khoá"))
                  .catch((error) => toastError("Mở khoá thất bại", error));
              },
            },
          ],
        },
        {
          id: "rotate",
          label: "Quay màn hình",
          Icon: IconSync,
          androidOnly: true,
          keywords: "rotate quay",
          children: [
            {
              id: "rotate-right",
              label: "Quay sang phải",
              androidOnly: true,
              run: () => {
                void setScreenRotation(device.udid, 1)
                  .then(notifyRotation(1))
                  .catch((error) => toastError("Quay màn hình thất bại", error));
              },
            },
            {
              id: "rotate-left",
              label: "Quay sang trái",
              androidOnly: true,
              run: () => {
                void setScreenRotation(device.udid, 3)
                  .then(notifyRotation(3))
                  .catch((error) => toastError("Quay màn hình thất bại", error));
              },
            },
            {
              id: "rotate-portrait",
              label: "Về màn hình dọc",
              androidOnly: true,
              run: () => {
                void setScreenRotation(device.udid, 0)
                  .then(notifyRotation(0))
                  .catch((error) => toastError("Quay màn hình thất bại", error));
              },
            },
          ],
        },
        {
          id: "transfer",
          label: "Cài đặt & truyền tệp",
          Icon: IconUpload,
          androidOnly: true,
          keywords: "apk install import export",
          children: [
            {
              id: "apk",
              label: "Cài APK…",
              androidOnly: true,
              keywords: "install apk",
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
              id: "import-media",
              label: "Đưa ảnh/video vào thư viện…",
              androidOnly: true,
              keywords: "import media anh video",
              run: () => {
                void (async () => {
                  const path = await pickFile({ title: "Chọn ảnh hoặc video" });
                  if (!path) return;
                  try {
                    const note = await importMedia(device.udid, path);
                    pushToast("ok", "Đã đưa vào thư viện", note);
                  } catch (error) {
                    toastError("Đưa vào thư viện thất bại", error);
                  }
                })();
              },
            },
            {
              id: "export-media",
              label: "Lấy ảnh/video từ máy…",
              androidOnly: true,
              keywords: "export media anh video",
              run: () => {
                void (async () => {
                  const dir = await pickDirectory("Lưu ảnh/video vào thư mục nào");
                  if (!dir) return;
                  try {
                    const report = await exportMedia(device.udid, dir);
                    if (report.missed > 0) {
                      pushToast(
                        "warn",
                        `Lấy được ${report.fetched}/${report.found} tệp`,
                        `${report.missed} tệp không copy được.`,
                      );
                    } else {
                      pushToast("ok", `Đã lấy ${report.fetched} tệp`, dir);
                    }
                  } catch (error) {
                    toastError("Lấy ảnh/video thất bại", error);
                  }
                })();
              },
            },
          ],
        },
        {
          id: "adb",
          label: "ADB",
          Icon: IconSettings,
          androidOnly: true,
          keywords: "adb",
          children: [
            {
              id: "adb-console",
              label: "Lệnh adb…",
              androidOnly: true,
              keywords: "shell command",
              run: () => setAdbFor(device.udid),
            },
            {
              id: "wifi-on",
              label: "Bật Wi-Fi trên máy",
              androidOnly: true,
              keywords: "turn on wifi",
              run: () => {
                void setWifiRadio(device.udid, true)
                  .then((on) =>
                    on
                      ? pushToast("ok", "Wi-Fi trên máy đã bật")
                      : pushToast("warn", "Máy vẫn báo Wi-Fi tắt"),
                  )
                  .catch((error) => toastError("Bật Wi-Fi thất bại", error));
              },
            },
            {
              id: "wifi-off",
              label: "Tắt Wi-Fi trên máy",
              androidOnly: true,
              danger: device.connection === "wifi",
              keywords: "turn off wifi",
              run: () => {
                void (async () => {
                  // A phone reached over wireless adb cuts its own link by obeying. The
                  // connection field is what says which, so the warning is only shown to
                  // the phones it is true of.
                  if (device.connection === "wifi") {
                    const ok = await requestConfirm({
                      title: `Tắt Wi-Fi trên ${device.name}?`,
                      message:
                        "Máy này đang kết nối qua Wi-Fi (adb không dây). Tắt Wi-Fi là tự ngắt kết nối — phải cắm cáp mới điều khiển lại được.",
                      confirmLabel: "Tắt Wi-Fi",
                      danger: true,
                    });
                    if (!ok) return;
                  }
                  try {
                    const on = await setWifiRadio(device.udid, false);
                    if (on) pushToast("warn", "Máy vẫn báo Wi-Fi bật");
                    else pushToast("ok", "Wi-Fi trên máy đã tắt");
                  } catch (error) {
                    toastError("Tắt Wi-Fi thất bại", error);
                  }
                })();
              },
            },
            {
              id: "reset-dpi",
              label: "Đặt lại mật độ điểm (DPI)",
              androidOnly: true,
              keywords: "reset dpi density",
              run: () => {
                void resetDisplayMetrics(device.udid, true, false)
                  .then((reading) => pushToast("ok", "Đã đặt lại DPI", reading))
                  .catch((error) => toastError("Đặt lại DPI thất bại", error));
              },
            },
            {
              id: "reset-size",
              label: "Đặt lại độ phân giải",
              androidOnly: true,
              keywords: "reset resolution size",
              run: () => {
                void resetDisplayMetrics(device.udid, false, true)
                  .then((reading) => pushToast("ok", "Đã đặt lại độ phân giải", reading))
                  .catch((error) => toastError("Đặt lại độ phân giải thất bại", error));
              },
            },
            {
              id: "phone-settings",
              label: "Mở Cài đặt của máy",
              androidOnly: true,
              keywords: "phone settings cai dat",
              run: () => {
                void openSystemSettings(device.udid)
                  .then(() => pushToast("ok", "Đã mở Cài đặt trên máy"))
                  .catch((error) => toastError("Mở Cài đặt thất bại", error));
              },
            },
            {
              id: "wifi-adb",
              label: "Chuyển sang WIFI (adb không dây)",
              androidOnly: true,
              danger: true,
              keywords: "wifi mode adb khong day",
              run: () => {
                // Confirmed, and the confirm says what actually happens: `adb tcpip 5555`
                // leaves adbd listening on 0.0.0.0, so the phone becomes drivable by anything
                // on the LAN that gets a host key trusted — and on Android 9 that is the only
                // gate there is. `factory_reset` has always been confirmed for a smaller blast
                // radius than this; this row used to fire on a single click and toast success.
                void requestConfirm({
                  title: "Bật adb không dây cho máy này?",
                  message:
                    "Máy sẽ mở cổng 5555 cho CẢ MẠNG LAN, không riêng máy tính này. Ai trong " +
                    "cùng mạng được máy chấp nhận khoá đều điều khiển được nó. Cổng vẫn mở cho " +
                    "tới khi bấm “Quay lại USB” hoặc khởi động lại máy.",
                  confirmLabel: "Bật",
                  danger: true,
                }).then((ok) => {
                  if (!ok) return;
                  void enableWifiAdb(device.udid)
                    .then((host) => {
                      pushToast("ok", "Đã bật adb không dây", host);
                      void refreshDevices()
                        .then(reload)
                        .catch(() => {});
                    })
                    .catch((error) => toastError("Bật WIFI adb thất bại", error));
                });
              },
            },
            {
              id: "wifi-adb-off",
              label: "Quay lại USB (đóng cổng adb không dây)",
              androidOnly: true,
              keywords: "usb tat wifi adb dong cong",
              run: () => {
                // The way back. `wifiAdbDisconnect` only drops this host's client; the phone
                // keeps listening. This is the only thing that closes the port short of a
                // reboot, which is why it sits next to the row that opens it.
                void disableWifiAdb(device.udid)
                  .then(() => {
                    pushToast("ok", "Đã đóng cổng adb không dây", "Máy quay lại chỉ nhận USB");
                    void refreshDevices()
                      .then(reload)
                      .catch(() => {});
                  })
                  .catch((error) => toastError("Quay lại USB thất bại", error));
              },
            },
          ],
        },
        {
          id: "copy",
          label: "Sao chép ID máy",
          Icon: IconCopy,
          keywords: "copy udid serial",
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
          Icon: IconRefresh,
          keywords: "refresh lam moi",
          run: () => {
            void refreshDevices().then(reload).catch((error) => toastError("Làm mới thất bại", error));
          },
        },
        {
          id: "reboot",
          label: "Khởi động lại máy",
          Icon: IconClock,
          danger: true,
          keywords: "restart reboot khoi dong",
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
        {
          id: "power-off",
          label: "Tắt máy",
          Icon: IconPower,
          androidOnly: true,
          danger: true,
          keywords: "shutdown tat may power off",
          run: () => {
            void requestConfirm({
              title: `Tắt ${device.name}?`,
              // The consequence, stated: nothing in this app can undo it, and on a farm
              // shelf the phone may be somewhere nobody wants to reach.
              message:
                "Máy tắt hẳn. Không có cách nào bật lại từ xa — phải có người bấm nút nguồn trên máy.",
              confirmLabel: "Tắt máy",
              danger: true,
            }).then((ok) => {
              if (!ok) return;
              void powerOffDevice(device.udid)
                .then(() => pushToast("ok", "Đã gửi lệnh tắt máy"))
                .catch((error) => toastError("Tắt máy thất bại", error));
            });
          },
        },
      ];
    },
    // `controlCenter` is read for the row's own label and `groupMode` for its explanation, so
    // a stale closure would leave the menu offering "Đặt" on the phone that is already the
    // main one, or telling the operator Sync is off when they just turned it on. `metaMap`/`metas` for the
    // same reason on the rename and renumber rows: a stale one pre-fills the dialog with the
    // value the operator just replaced.
    [reload, controlCenter, groupMode, metaMap, metas],
  );


  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    startupError()
      .then((issue) => {
        if (cancelled) return;
        setStartupIssue(issue);
        if (issue) return;

        void reload();
        void listenRiviuEvents((event) => {
          if (event.type === "devicesUpdated") {
            setDevices(event.devices);
          } else if (event.type === "deviceUpdated") {
            const { device } = event;
            setDevices((prev) => {
              const idx = prev.findIndex((d) => d.udid === device.udid);
              if (idx === -1) return [...prev, device];
              const next = [...prev];
              next[idx] = device;
              return next;
            });
          } else if (event.type === "jobUpdated") {
            const { job } = event;
            setJobs((prev) => {
              const idx = prev.findIndex((j) => j.id === job.id);
              if (idx === -1) return [job, ...prev];
              const next = [...prev];
              next[idx] = job;
              return next;
            });
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
        setBootError(describeError(error));
        void reload();
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
    // **`startupAttempt` is a dependency so a successful retry gets a subscription.**
    //
    // This effect returns early when startup failed, so nothing is listening. The retry
    // button cleared the issue and the app rendered — but the effect never ran again, so
    // `devicesUpdated`, `deviceUpdated`, `jobUpdated` and `streamFrame` were never
    // subscribed for the rest of the session. The retry handler knew half of this: it
    // replayed `reload()` by hand, with a comment saying the boot effect had already run.
    // It could not replay the subscription, and that is the half that matters — without it
    // the grid moves only on the three-second poll and no tile ever learns a frame arrived.
  }, [reload, startupAttempt]);

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
            onClick={async () => {
              setRetryingStartup(true);
              try {
                const stillBlocked = await retryStartup();
                setStartupIssue(stillBlocked);
                // Came up: run the boot effect again, which loads the fleet *and*
                // subscribes to events. This used to call `reload()` by hand instead,
                // which did the first and could not do the second.
                if (!stillBlocked) setStartupAttempt((attempt) => attempt + 1);
              } catch (error) {
                setStartupIssue(describeError(error));
              } finally {
                setRetryingStartup(false);
              }
            }}
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
