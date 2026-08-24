import { useEffect, useMemo, useRef, useState, type ReactElement } from "react";
import type { DeviceInfo, GroupInputReport, HardwareKey } from "../types";
import { groupInputOutcome } from "../groupInput";
import { getGroupSync } from "../groupSync";
import { recordSwipe, recordTap } from "../macroStore";
import {
  deviceControlBegin,
  deviceControlEnd,
  deviceSwipe,
  deviceSwipePath,
  deviceTap,
  groupInput,
  installIpa,
  viewInjectTouch,
  viewRequestKeyframe,
} from "../api";
import { createLiveDragGroup, liveTap, type LiveDragGroup } from "../liveDrag";

import { InstalledApps } from "./InstalledApps";
import { AdbConsole } from "./AdbConsole";
import { pickFile } from "../pickFile";

import { pushToast, toastError } from "../toastStore";
import { useDeviceKeyboards } from "./focus/useDeviceKeyboards";
import { useFocusActions } from "./focus/useFocusActions";
import { useQuickPhrases } from "./focus/useQuickPhrases";
import { useViewDecodeFailed, useViewLive, useViewSize } from "../viewStore";
import { streamPlaceholder } from "../streamPlaceholder";
import { startDevicePreview } from "../startPreview";
import { StreamPlaceholder } from "./StreamPlaceholder";
import { mapClientToImage, paintedViewBox } from "../viewHit";
import {
  FOCUS_ZOOM,
  loadZoom,
  stepZoom,
  storeZoom,
  wheelWantsZoom,
} from "../zoom";
import { PhoneCanvas } from "./PhoneCanvas";
import {
  IconBack,
  IconBattery,
  IconBell,
  IconCamera,
  IconClose,
  IconCopy,
  IconDownload,
  IconGrid,
  IconHome,
  IconImage,
  IconKeyboard,
  IconPhone,
  IconPower,
  IconRecents,
  IconRefresh,
  IconSync,
  IconText,
  IconUpload,
  IconVolumeDown,
  IconVolumeUp,
} from "./Icons";
import { withoutMenuIds, type DeviceMenuNode } from "../deviceMenu";
import { DeviceFunctionList } from "./DeviceFunctionList";

interface Props {
  device: DeviceInfo;
  /** 1-based index in the visible grid, shown in the sidebar header. */
  index: number;
  onClose: () => void;
  groupUdids: string[];
  groupMode: boolean;
  /**
   * The phones the operator can switch to without closing the overlay.
   *
   * Must be the same array the `index` above is computed from, or the header number and the
   * picker will disagree about which phone is #3.
   */
  devices: DeviceInfo[];
  onSelectDevice: (udid: string) => void;
  /**
   * The shared per-phone function catalog (`App.tsx`), so zooming into a phone does not lose
   * a function the tile's right-click menu offers.
   *
   * Optional, and empty by default: the overlay's own rows below are the ones its tests
   * exercise and the ones it can offer without a parent, so a caller that passes nothing gets
   * exactly the overlay it always had.
   */
  functions?: DeviceMenuNode[];
}

function mapToDevice(
  el: HTMLElement,
  clientX: number,
  clientY: number,
  width: number,
  height: number,
): { x: number; y: number } | null {
  const box = paintedViewBox(el);
  if (!box) return null;
  return mapClientToImage(box, clientX, clientY, width, height, "fill");
}

export function FocusStream({
  device,
  index,
  onClose,
  groupUdids,
  groupMode,
  devices,
  onSelectDevice,
  functions = [],
}: Props) {
  const hasView = useViewLive(device.udid);
  const viewSize = useViewSize(device.udid);
  const [busy, setBusy] = useState(false);
  const [showAdb, setShowAdb] = useState(false);
  const [showDevices, setShowDevices] = useState(false);
  const [showPhrases, setShowPhrases] = useState(false);
  const [showKeyboards, setShowKeyboards] = useState(false);
  const [frameWidth, setFrameWidth] = useState(() => loadZoom(FOCUS_ZOOM));
  const screenRef = useRef<HTMLDivElement>(null);
  /// The drag in progress: where it started, every sample since, and when the last one was
  /// taken. Held in a ref rather than state because a pointer fires far too often to
  /// re-render on, and none of it is rendered.
  const drag = useRef<{
    start: { x: number; y: number };
    steps: { x: number; y: number; durationMs: number }[];
    lastAt: number;
    /// Non-null once the gesture has travelled far enough to be a drag rather than a tap.
    /// Until then nothing is injected, so a tap keeps the uiautomator2 path it has always
    /// had and the control socket only ever sees gestures that benefit from being live.
    live: LiveDragGroup | null;
  } | null>(null);
  const inFlight = useRef(false);
  /// Devices whose overlay control session (`deviceControlBegin`) has finished opening.
  ///
  /// A gesture fired before this — the reflex scroll during a slow open on a phone whose
  /// agent is struggling — collides with the ManualControl lease the still-opening session
  /// already holds but has not yet registered, and `with_manual_session` cannot find the
  /// session to reuse, so it tries to acquire the lease again and is refused ("busy with
  /// ManualControl; ManualControl cannot acquire it"). Gestures gate on this instead. A ref,
  /// not state: the gesture handlers read it imperatively and must see the current value, and
  /// readiness changing should not force a re-render.
  const controlReady = useRef<Set<string>>(new Set());
  const targets =
    groupMode && groupUdids.length > 1 ? groupUdids : [device.udid];
  const targetKey = targets.join("\0");
  const isIos = device.platform === "ios";

  /// Report a group action that did not reach every phone.
  ///
  /// `quiet` is for the gesture rows. A drag across twenty phones is many calls, and a toast
  /// per partial result would bury the screen the operator is trying to watch — so a gesture
  /// speaks up only when it reached NOBODY, which is the case they genuinely cannot see. The
  /// explicit rows (a key, a typed phrase) report either way, because the operator pressed
  /// once and deserves one answer.
  /// Report a fleet action, and say whether it reached anybody.
  ///
  /// The return value exists because `sendPhrase` pushed "Đã gõ câu nhanh" straight after
  /// calling this — so an action that reached *none* of the selected phones put an error
  /// and a success on screen together, one under the other. Nothing else here claims an
  /// outcome afterwards, which is why it was the only one wrong.
  const reportGroup = (report: GroupInputReport, quiet: boolean): boolean => {
    const outcome = groupInputOutcome(report);
    if (outcome.kind === "ok") return true;
    if (outcome.kind === "none") {
      pushToast("error", outcome.title, outcome.detail);
      return false;
    }
    if (!quiet) pushToast("warn", outcome.title, outcome.detail);
    return true;
  };
  /// Whether this gesture may go down the scrcpy control socket.
  ///
  /// iOS has no such socket. Group mode used to be excluded too, on the reasoning that it
  /// "has no such thing as *one* socket" — but every phone has a socket of its own, and one
  /// live drag each off the same pointer stream is precisely what one-controls-many is. The
  /// exclusion meant the normal working mode on this farm, a multi-selection, never used the
  /// live path at all: the drag was decided at release from two samples and replayed as a
  /// straight line at constant speed. That is what "not smooth" was.
  const canDragLive = !isIos;
  const encodedW = viewSize?.width && viewSize.width > 0 ? viewSize.width : 0;
  const encodedH =
    viewSize?.height && viewSize.height > 0 ? viewSize.height : 0;
  const aspect = encodedW > 0 && encodedH > 0 ? encodedH / encodedW : 2;
  const decodeFailed = useViewDecodeFailed(device.udid);
  const placeholder = streamPlaceholder({
    hasView,
    hasGeometry: encodedW > 0 && encodedH > 0,
    decodeFailed,
    tileStreamState: device.tileStreamState,
    lastError: device.lastError,
  });

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    storeZoom(FOCUS_ZOOM, frameWidth);
  }, [frameWidth]);

  /// Open a manual session while the overlay is up, and give it back when it goes.
  ///
  /// **Each `end` is chained behind its own `begin`, and that ordering is the fix.** The
  /// cleanup used to fire `deviceControlEnd` immediately while `deviceControlBegin` was
  /// still in flight. `end_overlay_session` is a no-op when there is no session yet, so an
  /// `end` that overtook its `begin` succeeded quietly — and the `begin` landing a moment
  /// later left a live manual session with nobody to close it. That lease is then held
  /// until the app restarts, and every background job that wants the phone is refused for
  /// as long as it lasts.
  ///
  /// Opening, closing and reopening the overlay quickly is exactly the timing that produces
  /// it, which is why it could not be dismissed as unlikely.
  useEffect(() => {
    const udids = targetKey.split("\0").filter(Boolean);
    let cancelled = false;
    // Control is reopening for a new target set; nothing is ready until each begin lands.
    controlReady.current = new Set();
    // One promise per device, kept so the cleanup queues behind the right one rather than
    // behind all of them: a slow phone must not delay releasing a fast one.
    const opening = new Map(
      udids.map((udid) => [udid, deviceControlBegin(udid)] as const),
    );
    for (const [udid, begin] of opening) {
      void begin
        .then(() => {
          // Registered now, so `with_manual_session` will reuse it — gestures may fire.
          if (!cancelled) controlReady.current.add(udid);
        })
        .catch((error) => {
          if (!cancelled) toastError("Không mở được điều khiển", error);
        });
    }
    return () => {
      cancelled = true;
      for (const [udid, begin] of opening) {
        // `.catch` before `.then`, so a device whose begin rejected is still asked to
        // close: the failure may have come after the session was created.
        void begin
          .catch(() => undefined)
          .then(() => deviceControlEnd(udid))
          .catch(() => undefined);
      }
    };
  }, [targetKey]);

  const runExclusive = async (work: () => Promise<void>) => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      await work();
    } finally {
      inFlight.current = false;
    }
  };

  /// Run one thing at a time on this phone, and say so when that means not running.
  ///
  /// Refusing while another operation holds the device is right — two of these on one
  /// phone is the contention this project spent a week removing. Refusing *silently* was
  /// not: `backup`, `restore` and `reboot` put their success toast after the `await`, and
  /// those three menu rows are the only ones with no `disabled: busy`, so they are
  /// clickable during exactly the window in which this does nothing. Clicking Backup a
  /// second time while the first is still running picked a second folder, skipped the
  /// work, and reported "Backup xong" against a folder that stays empty. `inFlight` is
  /// also set by `runExclusive`, which never sets `busy` at all, so any tap or swipe on
  /// the phone screen opens the same window.
  ///
  /// Returns whether the work ran, so a caller cannot claim an outcome it did not get.
  const runBusy = async (work: () => Promise<void>): Promise<boolean> => {
    if (inFlight.current) {
      pushToast(
        "warn",
        "Máy đang bận",
        `${device.name} đang chạy một thao tác khác — chờ xong rồi thử lại.`,
      );
      return false;
    }
    inFlight.current = true;
    setBusy(true);
    try {
      await work();
      return true;
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };

  const quick = useQuickPhrases();
  const ime = useDeviceKeyboards(device.udid, runBusy);

  // Ctrl+wheel zooms the preview. Plain wheel scrolls the phone. Registered
  // by hand because React's synthetic onWheel is passive.
  useEffect(() => {
    const screen = screenRef.current;
    if (!screen) return;
    const onWheel = (event: WheelEvent) => {
      if (wheelWantsZoom(event)) {
        event.preventDefault();
        setFrameWidth((width) => stepZoom(FOCUS_ZOOM, width, event.deltaY));
        return;
      }
      if (!encodedW || !encodedH || inFlight.current) return;
      event.preventDefault();
      // Control is still opening — silently drop this tick rather than race the lease its
      // own begin holds. Wheel ticks are plentiful; the operator loses nothing and gets no
      // error toast, and the scroll works the moment control is up.
      if (!controlReady.current.has(device.udid)) return;
      const x = encodedW / 2;
      const startY = encodedH * 0.55;
      const endY = startY - Math.sign(event.deltaY) * encodedH * 0.18;
      void runExclusive(async () => {
        await deviceSwipe(
          device.udid,
          x,
          startY,
          x,
          endY,
          encodedW,
          encodedH,
          160,
        );
      }).catch((error) => toastError("Không cuộn được", error));
    };
    screen.addEventListener("wheel", onWheel, { passive: false });
    return () => screen.removeEventListener("wheel", onWheel);
  }, [device.udid, encodedH, encodedW]);

  /// The most samples a single drag is allowed to carry.
  ///
  /// A pointer fires at 60-120 Hz, so a two-second drag would otherwise become a couple of
  /// hundred `pointerMove`s. Past this the oldest movement is merged forward rather than
  /// dropped, which keeps the total duration and both endpoints exact and only coarsens the
  /// middle -- the part a human cannot see anyway.
  const MAX_PATH_STEPS = 64;

  /// Below this a gesture is a tap, above it a drag. One constant for both decisions on
  /// purpose: if the live path started before `runGesture` stopped calling it a tap, a short
  /// drag would be injected live *and* replayed as a tap on release.
  const TAP_SLOP = 10;

  const runGesture = async (
    start: { x: number; y: number },
    end: { x: number; y: number },
    steps: { x: number; y: number; durationMs: number }[],
    /// Which phones still need this gesture, defaulting to the whole selection.
    ///
    /// The live path passes the subset that could **not** take it live, so a phone that
    /// already ran the drag sample by sample does not run it again as a swipe. One verdict
    /// for the whole group has to be wrong in one direction or the other: replay on a phone
    /// that went live and it scrolls twice; skip one that fell back and it does nothing.
    only: string[] = targets,
  ) => {
    const iw = encodedW;
    const ih = encodedH;
    if (!iw || !ih || only.length === 0) return;
    const dist = Math.hypot(end.x - start.x, end.y - start.y);
    // Macro recording (A8): capture the logical gesture once, in reference-image space, the
    // same coordinates group_input replays. No-ops unless recording is armed in Group Tools.
    if (dist < TAP_SLOP) recordTap(end.x, end.y, iw, ih);
    else recordSwipe(start.x, start.y, end.x, end.y, iw, ih);
    await runExclusive(async () => {
      let remaining = only;
      if (dist < TAP_SLOP) {
        // Down the control socket first, and not for the milliseconds: it is the one input
        // path that does not depend on uiautomator2, so it still works on a phone whose
        // agent has lost UiAutomation -- the state that otherwise costs tens of seconds per
        // tap, or refuses every one of them (AGENTS.md 9.79).
        if (canDragLive) {
          const outcomes = await Promise.all(
            remaining.map(async (udid) => ({
              udid,
              outcome: await liveTap(
                (action, x, y) => viewInjectTouch(udid, action, x, y, iw, ih),
                end.x,
                end.y,
                (reason) =>
                  console.warn(`live tap fell back on ${udid}: ${reason}`),
              ),
            })),
          );
          remaining = outcomes
            .filter((row) => row.outcome !== "live")
            .map((row) => row.udid);
          if (remaining.length === 0) return;
        }
        if (remaining.length > 1) {
          reportGroup(
            await groupInput({
              udids: remaining,
              kind: "tap",
              x: end.x,
              y: end.y,
              imageW: iw,
              imageH: ih,
              sync: getGroupSync(),
            }),
            true,
          );
        } else {
          await deviceTap(remaining[0], end.x, end.y, iw, ih);
        }
      } else if (remaining.length > 1) {
        // Group control has no path command; the endpoints are what every device gets.
        //
        // Which is why this is now the *fallback* rather than the road every group gesture
        // took. Two endpoints replayed at constant speed is a drag, and a drag is measurably
        // weaker than the flick the operator drew: 13 of 40 against 19 of 19 on a TikTok
        // carousel, measured 19/08/2026. The phones that could take it live already have the
        // real shape; these are the ones that could not.
        reportGroup(
          await groupInput({
            udids: remaining,
            kind: "swipe",
            x: start.x,
            y: start.y,
            toX: end.x,
            toY: end.y,
            imageW: iw,
            imageH: ih,
            sync: getGroupSync(),
          }),
          true,
        );
      } else if (steps.length >= 2) {
        await deviceSwipePath(remaining[0], start, steps, iw, ih);
      } else {
        // Too few samples to be a path -- a fast flick the pointer only reported twice.
        await deviceSwipe(
          remaining[0],
          start.x,
          start.y,
          end.x,
          end.y,
          iw,
          ih,
          160,
        );
      }
    });
  };

  // The nine actions live in `focus/useFocusActions` — 195 lines for six symbols. They are
  // destructured back into the same names so nothing below this line had to change.
  const {
    pressKey,
    sendPhrase,
    importFile,
    exportFiles,
    copySerial,
    capture,
    reboot,
    backup,
    restore,
  } = useFocusActions({
    device,
    targets,
    controlReady,
    reportGroup,
    runExclusive,
    runBusy,
  });

  /// The overlay's own rows. Typed as `DeviceMenuNode` rather than a local shape, because
  /// they are concatenated with the shared catalog below and drawn by the same component: one
  /// list means one search box, one platform gate, and no heading telling the operator which
  /// half of the panel a function lives in.
  const menuRows: DeviceMenuNode[] = [
    {
      // First, because switching phone is navigation rather than an action on this one —
      // and because it is the row that stops the operator closing and reopening the overlay
      // twenty times to walk the fleet.
      id: "switchDevice",
      label: showDevices ? "Ẩn danh sách máy" : "Đổi máy",
      Icon: IconPhone,
      run: () => setShowDevices((open) => !open),
    },
    {
      id: "volumeUp",
      label: "Vol+",
      Icon: IconVolumeUp,
      androidOnly: true,
      disabled: busy,
      run: () => void pressKey("volumeUp"),
    },
    {
      id: "volumeDown",
      label: "Vol−",
      Icon: IconVolumeDown,
      androidOnly: true,
      disabled: busy,
      run: () => void pressKey("volumeDown"),
    },
    {
      // Before "restart the stream", because it is the cheaper half of the same complaint:
      // one byte and a fresh keyframe against ~11.5 s of black tile. The watchdog tries this
      // first for the same reason.
      id: "refreshPicture",
      label: "Làm mới hình",
      Icon: IconSync,
      androidOnly: true,
      disabled: busy,
      run: () => {
        void viewRequestKeyframe(device.udid)
          .then((asked) =>
            asked
              ? pushToast("ok", "Đã xin hình mới")
              : pushToast("warn", "Máy chưa có stream để làm mới"),
          )
          .catch((error) => toastError("Không xin được hình mới", error));
      },
    },
    {
      id: "screenshot",
      label: "Chụp màn hình",
      Icon: IconCamera,
      disabled: busy,
      run: () => void capture(),
    },
    {
      id: "power",
      label: "Nút nguồn",
      Icon: IconPower,
      androidOnly: true,
      disabled: busy,
      run: () => void pressKey("power"),
    },
    {
      id: "installApk",
      label: "Cài APK",
      Icon: IconUpload,
      androidOnly: true,
      disabled: busy,
      run: () => {
        void (async () => {
          const path = await pickFile({
            title: "Chọn APK",
            filters: [{ name: "APK", extensions: ["apk"] }],
          });
          if (!path) return;
          try {
            await installIpa(device.udid, path);
            pushToast("ok", "Đã cài APK");
          } catch (error) {
            toastError("Cài APK thất bại", error);
          }
        })();
      },
    },
    {
      // Beside Cài APK, because both are "put a file on this phone" — and GenFarmer keeps
      // ImportFile / ExportFile adjacent in its own menu.
      id: "importMedia",
      label: "Đưa ảnh/video vào máy",
      Icon: IconImage,
      androidOnly: true,
      disabled: busy,
      run: () => void importFile(),
    },
    {
      id: "exportMedia",
      label: "Lấy ảnh/video từ máy",
      Icon: IconDownload,
      androidOnly: true,
      disabled: busy,
      run: () => void exportFiles(),
    },
    {
      // **`adb-inline`, not `adb`.** `panelNodes` concatenates these rows with the shared
      // per-phone catalog, and that catalog's ADB *submenu* is also `id: "adb"` —
      // `withoutMenuIds` below drops only its `adb-console` child, not the parent. Two nodes
      // with one id in one list is a duplicate React key, and worse than the warning:
      // `DeviceFunctionList` keys its open-flyout state on `node.id`, so hovering this leaf
      // opened the submenu's flyout. The catalog's id is a contract pinned by
      // `deviceMenu.test.ts` and `DeviceContextMenu.test.tsx`, so this is the side that moves.
      id: "adb-inline",
      label: "Lệnh adb",
      Icon: IconGrid,
      androidOnly: true,
      run: () => setShowAdb(true),
    },
    {
      // GenFarmer keeps these two next to Adb command, and both are text-input adjacent, so
      // they sit here rather than at the end of the list.
      id: "quickPhrase",
      label: showPhrases ? "Ẩn câu nhanh" : "Câu nhanh",
      Icon: IconText,
      androidOnly: true,
      disabled: busy,
      run: () => setShowPhrases((open) => !open),
    },
    {
      id: "switchKeyboard",
      label: showKeyboards ? "Ẩn bàn phím" : "Đổi bàn phím",
      Icon: IconKeyboard,
      androidOnly: true,
      disabled: busy,
      run: () => {
        setShowKeyboards((open) => {
          const next = !open;
          if (next && ime.keyboards === null) void ime.load();
          return next;
        });
      },
    },
    {
      id: "notification",
      label: "Thông báo",
      Icon: IconBell,
      androidOnly: true,
      disabled: busy,
      run: () => void pressKey("notification"),
    },
    {
      id: "reboot",
      label: "Khởi động lại",
      Icon: IconRefresh,
      run: () => void reboot(),
    },
    ...(isIos
      ? [
          {
            id: "backup",
            label: "Backup",
            Icon: IconDownload,
            run: () => void backup(),
          },
          {
            id: "restore",
            label: "Restore",
            Icon: IconUpload,
            danger: true,
            run: () => void restore(),
          },
        ]
      : []),
  ];

  /// The shared catalog minus what this panel already offers.
  ///
  /// Every id here is a row the overlay does better in place: the app list, the keyboard
  /// picker and the adb console open as inline panels beside the phone; screenshot, install
  /// APK and the two media transfers are icon rows above; Home/Back/Recents are the navbar
  /// below; volume and notification are icon rows too. `open` is dropped because the overlay
  /// *is* the thing that row opens.
  const overlayFunctions = useMemo(
    () =>
      withoutMenuIds(functions, [
        "open",
        "apps",
        "keyboard",
        "adb-console",
        "screenshot",
        "transfer",
        "reboot",
        "key-home",
        "key-back",
        "key-recents",
        "key-volumeUp",
        "key-volumeDown",
        "key-notification",
      ]),
    [functions],
  );

  /// Everything the panel offers, in one list: its own rows first (the ones it does better
  /// in place — inline panels, the picture refresh, switching phone), then the shared catalog
  /// minus whatever those already cover.
  ///
  /// Not memoised, deliberately: `menuRows` is rebuilt on every render by design — its labels
  /// read `busy`, `showDevices`, `showPhrases` — so a memo keyed on anything less than the
  /// array itself would hand back stale rows, and one keyed on the array would never hit.
  /// Concatenating forty objects costs nothing next to the render it happens inside.
  const panelNodes = [...menuRows, ...overlayFunctions];

  const navKeys: {
    key: HardwareKey;
    title: string;
    Icon: (props: { size?: number }) => ReactElement;
  }[] = isIos
    ? [{ key: "home", title: "Home", Icon: IconHome }]
    : [
        { key: "recents", title: "Recents", Icon: IconRecents },
        { key: "home", title: "Home", Icon: IconHome },
        { key: "back", title: "Back", Icon: IconBack },
      ];

  return (
    <div
      className="focus-overlay"
      role="dialog"
      aria-modal="true"
      aria-busy={busy}
      aria-label={`Điều khiển ${device.name}`}
      onClick={onClose}
    >
      <div className="focus-stage" onClick={(event) => event.stopPropagation()}>
        {/* Exact pixel size from the wheel zoom — never shrink-to-fit the
            viewport, or zooming in looks like it did nothing. */}
        <div
          ref={screenRef}
          className={`focus-phone-screen${busy ? " is-busy" : ""}`}
          // Tests target this, not the class. A class name is a styling decision and the
          // restyle is about to change a lot of them; a test that breaks because a colour
          // moved is a test that stops meaning anything.
          data-testid="focus-screen"
          style={{ width: frameWidth, height: frameWidth * aspect }}
          title="Ctrl + lăn chuột để phóng to / thu nhỏ"
          onPointerDown={(e) => {
            if (busy || inFlight.current || e.button !== 0) return;
            // Said out loud rather than dropped. A gesture needs the encoded frame size to
            // map through, and without it this handler used to return in silence -- so on a
            // phone that had not painted yet the operator could click the picture as long as
            // they liked and receive no toast, no log and no reaction. "Nothing happens" is
            // the worst thing a control surface can do.
            if (!encodedW || !encodedH) {
              pushToast(
                "warn",
                placeholder.view.kind === "failed"
                  ? "Chưa điều khiển được: stream đang lỗi."
                  : "Chưa có hình từ máy, chưa gửi được thao tác.",
              );
              return;
            }
            const start = mapToDevice(
              e.currentTarget,
              e.clientX,
              e.clientY,
              encodedW,
              encodedH,
            );
            if (!start) return;
            e.preventDefault();
            drag.current = {
              start,
              steps: [],
              lastAt: performance.now(),
              live: null,
            };
            e.currentTarget.setPointerCapture(e.pointerId);
          }}
          onPointerMove={(e) => {
            // The whole point of this handler: without it the gesture is decided at release
            // from two samples, so every drag reaches the phone as a straight line at
            // constant speed no matter what the finger did.
            const held = drag.current;
            if (!held || !encodedW || !encodedH) return;
            const now = performance.now();
            const elapsed = now - held.lastAt;
            // Ignore a sample that is neither far enough nor late enough to carry
            // information; a pointer reports far more often than a gesture changes.
            if (elapsed < 8) return;
            const point = mapToDevice(
              e.currentTarget,
              e.clientX,
              e.clientY,
              encodedW,
              encodedH,
            );
            if (!point) return;
            const previous = held.steps.at(-1) ?? held.start;
            if (Math.hypot(point.x - previous.x, point.y - previous.y) < 2)
              return;
            // Past the tap threshold this is a drag, and a drag the phone can follow while
            // it happens instead of replaying it after release. Started from `held.start`,
            // not from here: the finger has to land where the operator put it.
            if (
              !held.live &&
              canDragLive &&
              Math.hypot(point.x - held.start.x, point.y - held.start.y) >=
                TAP_SLOP
            ) {
              held.live = createLiveDragGroup(
                targets.map((udid) => ({
                  udid,
                  send: (action, x, y) =>
                    viewInjectTouch(udid, action, x, y, encodedW, encodedH),
                })),
                // Not a toast: the gesture still reaches the phone the old way, so there is
                // nothing for the operator to do about it. It goes to the console so that a
                // silently dead live path is findable.
                (reason) => console.warn(`live drag fell back: ${reason}`),
                // Same anti-detection jitter the batch path applies, so a live group-drag is
                // not twenty pixel-identical paths. 0 (the default policy) is a no-op.
                getGroupSync().offset?.maxPx ?? 0,
              );
              held.live.begin(held.start.x, held.start.y);
            }
            held.live?.move(point.x, point.y);
            held.lastAt = now;
            if (held.steps.length >= MAX_PATH_STEPS) {
              // Merge forward: the endpoint and the total duration stay exact.
              const last = held.steps[held.steps.length - 1];
              last.x = point.x;
              last.y = point.y;
              last.durationMs += elapsed;
              return;
            }
            held.steps.push({ x: point.x, y: point.y, durationMs: elapsed });
          }}
          onPointerUp={async (e) => {
            // **Every exit from here lifts the finger.** `onPointerDown` captures the
            // pointer, so a drag that leaves the canvas still delivers its `pointerup` here
            // — and `mapToDevice` returns null for a point outside the painted rect. Both
            // early returns below used to drop out with an injected DOWN and a stream of
            // MOVEs already on the control socket and no UP behind them, which is a phone
            // holding a pointer down forever. Releasing past the edge of the preview is the
            // natural end of a fast flick, so this was not a corner case.
            const held = drag.current;
            drag.current = null;
            const lift = () => {
              const last = held?.steps.at(-1) ?? held?.start;
              if (held?.live && last) void held.live.end(last.x, last.y);
            };
            if (e.button !== 0 || !held || !encodedW || !encodedH) {
              lift();
              return;
            }
            e.preventDefault();
            const end = mapToDevice(
              e.currentTarget,
              e.clientX,
              e.clientY,
              encodedW,
              encodedH,
            );
            if (!end) {
              lift();
              return;
            }
            // The release point is always the last step, so the gesture ends exactly where
            // the operator let go even if that sample was filtered out above.
            const steps = [...held.steps];
            const lastElapsed = Math.max(1, performance.now() - held.lastAt);
            const tail = steps.at(-1);
            if (tail && tail.x === end.x && tail.y === end.y) {
              tail.durationMs += lastElapsed;
            } else {
              steps.push({ x: end.x, y: end.y, durationMs: lastElapsed });
            }
            try {
              // A live drag has already happened on the phone, sample by sample. Replaying
              // it as a swipe would scroll everything a second time.
              if (held.live) {
                // The split, not a single verdict: the phones that ran it live already have
                // the gesture, and replaying it on them would scroll everything twice.
                const split = await held.live.end(end.x, end.y);
                if (split.fallback.length === 0) return;
                await runGesture(held.start, end, steps, split.fallback);
                return;
              }
              await runGesture(held.start, end, steps);
            } catch (error) {
              toastError("Điều khiển thất bại", error);
            }
          }}
          onPointerCancel={() => {
            const held = drag.current;
            drag.current = null;
            // A cancelled drag has a finger on the phone that nothing else will lift, and a
            // pointer left down joins itself to whatever the operator does next.
            const last = held?.steps.at(-1) ?? held?.start;
            if (held?.live && last) void held.live.end(last.x, last.y);
          }}
        >
          <PhoneCanvas
            udid={device.udid}
            surfaceId="overlay"
            fill
            className={`focus-touch${busy ? " is-busy" : ""}`}
          />
          <StreamPlaceholder
            view={placeholder.view}
            deviceName={device.name}
            onRetry={() => {
              void startDevicePreview(device).catch((error) =>
                toastError("Không mở lại được stream", error),
              );
            }}
          />
        </div>
        {/* Exactly as tall as the phone picture beside it, and that is the whole fix for a
            visible bug: `.focus-stage` is a flex row that stretches to its **tallest** child,
            so a panel taller than the picture grew the stage and left a band of white under
            the phone. Pinning the height here (the same `frameWidth * aspect` the pane uses)
            makes the picture the reference and the panel scroll inside it. The CSS
            `max-height` stays as the cap for a very large zoom, where the picture is the
            taller one again and there is no gap either way. */}
        <aside
          className="focus-menu"
          aria-label="Chức năng thiết bị"
          style={{ height: frameWidth * aspect }}
        >
          <header className="focus-menu-head">
            <strong title={device.udid}>
              {index} {device.name}
            </strong>
            {groupMode && targets.length > 1 && (
              <span
                className="focus-menu-group"
                title={`Đồng bộ ${targets.length} máy`}
              >
                ×{targets.length}
              </span>
            )}
            {/* Read-only, from the device poll that already carries it. `—` rather than a
                made-up 0% or 100% when the value is absent: the driver returns None for a
                phone it could not read, and a battery chip that invents a number is the
                same lying button the Rotate row was written to avoid. */}
            <span
              className="focus-menu-battery"
              title={
                device.battery == null
                  ? "Chưa đọc được mức pin"
                  : `Pin ${device.battery}%`
              }
            >
              <IconBattery size={13} />
              {device.battery == null ? "—" : `${device.battery}%`}
            </span>
            <button
              type="button"
              className="ghost"
              title="Copy serial"
              aria-label="Copy serial"
              onClick={() => void copySerial()}
            >
              <IconCopy size={14} />
            </button>
            <button
              type="button"
              className="close"
              title="Đóng"
              aria-label="Đóng"
              onClick={onClose}
            >
              <IconClose size={14} />
            </button>
          </header>
          {/* Why every row is greyed out. `disabled={busy}` alone is silent, and a row that
              cannot be clicked and does not say why reads exactly like a row that does
              nothing — which is how three working rows came to be reported as broken. */}
          {busy && (
            <p className="focus-menu-busy">
              Đang chạy một thao tác trên máy này…
            </p>
          )}
          {/* ONE scroll region for the whole column: the function rows, whatever panel is
              open, and the App List all live in here, so a wheel anywhere in the panel moves
              the same list. Two scroll boxes stacked (which is what a `flex: 1` list plus a
              `flex: 1 1 45%` App List gave) means the wheel does different things depending on
              which half the pointer is over, and the operator has to find the seam. */}
          <div className="focus-menu-scroll">
            {/* ONE list, and no "other functions" heading: the panel's own rows and the shared
              per-phone catalog are the same kind of thing, so splitting them under a heading
              only made the operator learn which half a function lived in. The search box is
              the first row of the list and is `position: sticky`, so it stays at the top of
              the panel while everything under it scrolls — and it filters *everything*, which
              a box above only half the rows could not do. */}
            <div className="focus-menu-list">
              <DeviceFunctionList
                nodes={panelNodes}
                platform={device.platform}
              />
            </div>
            {/* Every one of these sits BEFORE the navbar and carries its own height. The menu
              list is `flex: 1`, so a sibling added after the navbar collapses to nothing and
              pushes the navbar out of the column (AGENTS.md §9.57). */}
            {showDevices && (
              <div
                className="focus-menu-panel"
                role="group"
                aria-label="Đổi máy"
              >
                {devices.length <= 1 ? (
                  <p className="hint">Chỉ có một máy đang hiển thị.</p>
                ) : (
                  devices.map((candidate, position) => (
                    <button
                      key={candidate.udid}
                      type="button"
                      className={
                        candidate.udid === device.udid ? "is-current" : ""
                      }
                      title={candidate.udid}
                      onClick={() => {
                        if (candidate.udid === device.udid) return;
                        // Lift the swap to the parent rather than swapping a local device:
                        // the overlay's preset effect and this component's control lease are
                        // both keyed on the udid, so changing it there releases the old phone
                        // and claims the new one with no extra code.
                        onSelectDevice(candidate.udid);
                        setShowDevices(false);
                      }}
                    >
                      <span className="focus-device-index">{position + 1}</span>
                      <span>{candidate.name}</span>
                    </button>
                  ))
                )}
              </div>
            )}
            {showPhrases && (
              <div
                className="focus-menu-panel"
                role="group"
                aria-label="Câu nhanh"
              >
                <input
                  type="text"
                  value={quick.name}
                  placeholder="Tên (không bắt buộc)"
                  onChange={(event) => quick.setName(event.target.value)}
                />
                <input
                  type="text"
                  value={quick.content}
                  placeholder="Nội dung (vd: xin chào)"
                  onChange={(event) => quick.setContent(event.target.value)}
                  onKeyUp={(event) => {
                    if (event.key === "Enter") quick.save();
                  }}
                />
                <button
                  type="button"
                  className="ghost"
                  onClick={() => quick.save()}
                >
                  Lưu câu
                </button>
                {quick.error && <p className="error">{quick.error}</p>}
                {!quick.phrases.length ? (
                  <p className="hint">Chưa có câu nào.</p>
                ) : (
                  quick.phrases.map((phrase) => (
                    <div className="focus-phrase-row" key={phrase.id}>
                      <button
                        type="button"
                        disabled={busy}
                        title={phrase.content}
                        onClick={() => void sendPhrase(phrase)}
                      >
                        {phrase.name}
                      </button>
                      <button
                        type="button"
                        className="danger"
                        aria-label={`Xoá ${phrase.name}`}
                        onClick={() => quick.remove(phrase.id)}
                      >
                        <IconClose size={12} />
                      </button>
                    </div>
                  ))
                )}
              </div>
            )}
            {showKeyboards && (
              <div
                className="focus-menu-panel"
                role="group"
                aria-label="Đổi bàn phím"
              >
                {ime.keyboards === null ? (
                  <p className="hint">Đang đọc…</p>
                ) : !ime.keyboards.length ? (
                  <p className="hint">Không đọc được bàn phím nào.</p>
                ) : (
                  ime.keyboards.map((method) => (
                    <button
                      key={method.id}
                      type="button"
                      disabled={busy}
                      className={method.id === ime.current ? "is-current" : ""}
                      title={method.id}
                      onClick={() => void ime.choose(method)}
                    >
                      {method.label}
                      {method.id === ime.current && <span> ✓</span>}
                    </button>
                  ))
                )}
              </div>
            )}
            {/* Always here, never behind a toggle — the reference product's App List sits
              under its Functions list and so does this one. `launchable` makes a row a
              button: finding an app and being unable to open it was the other half of the
              complaint. */}
            <InstalledApps
              udid={device.udid}
              deviceName={device.name}
              launchable
            />
          </div>
          {showAdb && (
            <AdbConsole device={device} onClose={() => setShowAdb(false)} />
          )}
          <nav className="focus-navbar" aria-label="Phím điều hướng">
            {navKeys.map(({ key, title, Icon }) => (
              <button
                key={key}
                type="button"
                disabled={busy}
                title={title}
                onClick={() => void pressKey(key)}
              >
                <Icon size={18} />
              </button>
            ))}
          </nav>
        </aside>
      </div>
    </div>
  );
}
