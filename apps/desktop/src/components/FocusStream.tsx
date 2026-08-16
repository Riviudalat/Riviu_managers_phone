import { useEffect, useRef, useState, type ReactElement } from "react";
import type { DeviceInfo, HardwareKey } from "../types";
import {
  backupDevice,
  deviceControlBegin,
  deviceControlEnd,
  deviceKey,
  deviceShell,
  deviceSwipe,
  deviceSwipePath,
  deviceTap,
  deviceTypeText,
  exportMedia,
  groupInput,
  importMedia,
  rebootDevice,
  installIpa,
  restoreDevice,
  saveViewSnapshot,
  screenshot,
  setScreenRotation,
  viewRequestKeyframe,
} from "../api";
import {
  parseCurrentInputMethod,
  parseInputMethods,
  type InputMethod,
} from "../imeList";
import {
  addQuickPhrase,
  loadQuickPhrases,
  removeQuickPhrase,
  storeQuickPhrases,
  type QuickPhrase,
} from "../quickPhrases";
import { InstalledApps } from "./InstalledApps";
import { AdbConsole } from "./AdbConsole";
import { pickFile } from "../pickFile";
import { pickDirectory } from "../pickFile";
import { requestConfirm } from "../confirmStore";
import { pushToast, toastError } from "../toastStore";
import { exportViewJpeg, useViewLive, useViewSize } from "../viewStore";
import { mapClientToImage, paintedViewBox } from "../viewHit";
import { FOCUS_ZOOM, loadZoom, stepZoom, storeZoom, wheelWantsZoom } from "../zoom";
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
}: Props) {
  const hasView = useViewLive(device.udid);
  const viewSize = useViewSize(device.udid);
  const [busy, setBusy] = useState(false);
  const [showApps, setShowApps] = useState(false);
  const [showAdb, setShowAdb] = useState(false);
  const [showDevices, setShowDevices] = useState(false);
  const [showPhrases, setShowPhrases] = useState(false);
  const [showKeyboards, setShowKeyboards] = useState(false);
  const [phrases, setPhrases] = useState<QuickPhrase[]>(() => loadQuickPhrases());
  const [phraseName, setPhraseName] = useState("");
  const [phraseContent, setPhraseContent] = useState("");
  const [phraseError, setPhraseError] = useState<string | null>(null);
  const [keyboards, setKeyboards] = useState<InputMethod[] | null>(null);
  const [currentKeyboard, setCurrentKeyboard] = useState<string | null>(null);
  const [frameWidth, setFrameWidth] = useState(() => loadZoom(FOCUS_ZOOM));
  const screenRef = useRef<HTMLDivElement>(null);
  /// The drag in progress: where it started, every sample since, and when the last one was
  /// taken. Held in a ref rather than state because a pointer fires far too often to
  /// re-render on, and none of it is rendered.
  const drag = useRef<{
    start: { x: number; y: number };
    steps: { x: number; y: number; durationMs: number }[];
    lastAt: number;
  } | null>(null);
  const inFlight = useRef(false);
  const targets = groupMode && groupUdids.length > 1 ? groupUdids : [device.udid];
  const targetKey = targets.join("\0");
  const isIos = device.platform === "ios";
  const encodedW = viewSize?.width && viewSize.width > 0 ? viewSize.width : 0;
  const encodedH = viewSize?.height && viewSize.height > 0 ? viewSize.height : 0;
  const aspect = encodedW > 0 && encodedH > 0 ? encodedH / encodedW : 2;

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

  useEffect(() => {
    const udids = targetKey.split("\0").filter(Boolean);
    let cancelled = false;
    void (async () => {
      try {
        await Promise.all(udids.map((udid) => deviceControlBegin(udid)));
      } catch (error) {
        if (!cancelled) toastError("Không mở được điều khiển", error);
      }
      if (cancelled) {
        await Promise.all(udids.map((udid) => deviceControlEnd(udid).catch(() => undefined)));
      }
    })();
    return () => {
      cancelled = true;
      for (const udid of udids) {
        void deviceControlEnd(udid);
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

  const runBusy = async (work: () => Promise<void>) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    try {
      await work();
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };

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
      const x = encodedW / 2;
      const startY = encodedH * 0.55;
      const endY = startY - Math.sign(event.deltaY) * encodedH * 0.18;
      void runExclusive(async () => {
        await deviceSwipe(device.udid, x, startY, x, endY, encodedW, encodedH, 160);
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

  const runGesture = async (
    start: { x: number; y: number },
    end: { x: number; y: number },
    steps: { x: number; y: number; durationMs: number }[],
  ) => {
    const iw = encodedW;
    const ih = encodedH;
    if (!iw || !ih) return;
    const dist = Math.hypot(end.x - start.x, end.y - start.y);
    await runExclusive(async () => {
      if (dist < 10) {
        if (targets.length > 1) {
          await groupInput({
            udids: targets,
            kind: "tap",
            x: end.x,
            y: end.y,
            imageW: iw,
            imageH: ih,
          });
        } else {
          await deviceTap(device.udid, end.x, end.y, iw, ih);
        }
      } else if (targets.length > 1) {
        // Group control has no path command; the endpoints are what every device gets.
        await groupInput({
          udids: targets,
          kind: "swipe",
          x: start.x,
          y: start.y,
          toX: end.x,
          toY: end.y,
          imageW: iw,
          imageH: ih,
        });
      } else if (steps.length >= 2) {
        await deviceSwipePath(device.udid, start, steps, iw, ih);
      } else {
        // Too few samples to be a path -- a fast flick the pointer only reported twice.
        await deviceSwipe(device.udid, start.x, start.y, end.x, end.y, iw, ih, 160);
      }
    });
  };

  const pressKey = async (key: HardwareKey) => {
    try {
      await runExclusive(async () => {
        if (targets.length > 1) {
          await groupInput({ udids: targets, kind: "key", key });
        } else {
          await deviceKey(device.udid, key);
        }
      });
    } catch (error) {
      toastError("Không bấm được phím", error);
    }
  };

  /// Type a saved phrase onto every phone the overlay is driving.
  ///
  /// Goes through the same `group_input` `type` path the keyboard uses, which reaches the
  /// agent's `ACTION_SET_TEXT` — the only route here that carries Vietnamese diacritics.
  /// `adb shell input text` is killed outright by them.
  const sendPhrase = async (phrase: QuickPhrase) => {
    try {
      await runBusy(async () => {
        if (targets.length > 1) {
          await groupInput({ udids: targets, kind: "type", text: phrase.content });
        } else {
          await deviceTypeText(device.udid, phrase.content);
        }
      });
      pushToast("ok", "Đã gõ câu nhanh", phrase.name);
    } catch (error) {
      toastError("Gõ câu nhanh thất bại", error);
    }
  };

  /// Read the phone's keyboards, and which one is current.
  ///
  /// Two shells rather than one round trip because they answer different questions and a
  /// failure of the second should not hide the first. The ids come back from the phone and
  /// are only ever sent back verbatim — see `imeList.ts`.
  const loadKeyboards = async () => {
    try {
      await runBusy(async () => {
        const listed = await deviceShell(device.udid, "ime list -s");
        setKeyboards(parseInputMethods(listed.stdout));
        try {
          const current = await deviceShell(
            device.udid,
            "settings get secure default_input_method",
          );
          setCurrentKeyboard(parseCurrentInputMethod(current.stdout));
        } catch {
          setCurrentKeyboard(null);
        }
      });
    } catch (error) {
      setKeyboards([]);
      toastError("Không đọc được danh sách bàn phím", error);
    }
  };

  const chooseKeyboard = async (method: InputMethod) => {
    // Only an id the phone itself just printed, looked up in the parsed list rather than
    // taken from the event: the value reaches a real shell.
    const known = keyboards?.find((candidate) => candidate.id === method.id);
    if (!known) return;
    try {
      await runBusy(async () => {
        await deviceShell(device.udid, `ime set ${known.id}`);
        setCurrentKeyboard(known.id);
      });
      pushToast("ok", "Đã đổi bàn phím", known.label);
    } catch (error) {
      toastError("Đổi bàn phím thất bại", error);
    }
  };

  const importFile = async () => {
    const path = await pickFile({
      title: "Chọn ảnh hoặc video",
      filters: [{ name: "Ảnh / video", extensions: ["jpg", "jpeg", "png", "webp", "gif", "mp4", "mov", "3gp"] }],
    });
    if (!path) return;
    pushToast("info", "Đang đưa vào máy…", device.name);
    try {
      await runBusy(async () => {
        pushToast("ok", "Đã vào thư viện", await importMedia(device.udid, path));
      });
    } catch (error) {
      toastError("Đưa file vào máy thất bại", error);
    }
  };

  const exportFiles = async () => {
    const dir = await pickDirectory("Chọn thư mục lưu ảnh/video lấy từ máy");
    if (!dir) return;
    // A full camera roll over USB 2.0 takes minutes, so say so before it starts rather than
    // leaving the operator watching a disabled button.
    pushToast("info", "Đang lấy ảnh/video…", `${device.name} — có thể mất vài phút.`);
    try {
      await runBusy(async () => {
        const count = await exportMedia(device.udid, dir);
        if (count === 0) {
          // Not an error: an empty gallery is an answer, and reporting it as a failure
          // would send the operator looking for a bug that is not there.
          pushToast("info", "Máy không có ảnh/video nào", device.name);
        } else {
          pushToast("ok", `Đã lấy ${count} file`, dir);
        }
      });
    } catch (error) {
      toastError("Lấy ảnh/video thất bại", error);
    }
  };

  const savePhrase = () => {
    const { phrases: next, error } = addQuickPhrase(
      phrases,
      phraseName,
      phraseContent,
      `${Date.now()}-${phrases.length}`,
    );
    setPhraseError(error);
    if (error) return;
    setPhrases(next);
    storeQuickPhrases(next);
    setPhraseName("");
    setPhraseContent("");
  };

  const copySerial = async () => {
    try {
      await navigator.clipboard.writeText(device.udid);
      pushToast("ok", "Đã copy serial", device.udid);
    } catch (error) {
      toastError("Không copy được serial", error);
    }
  };

  const capture = async () => {
    try {
      await runBusy(async () => {
        try {
          pushToast("ok", "Đã chụp màn hình", await screenshot(device.udid));
        } catch (first) {
          const bytes = await exportViewJpeg(device.udid);
          if (!bytes) throw first;
          pushToast("ok", "Đã chụp màn hình", await saveViewSnapshot(device.udid, Array.from(bytes)));
        }
      });
    } catch (e) {
      toastError("Chụp màn hình thất bại", e);
    }
  };

  const reboot = async () => {
    const proceed = await requestConfirm({
      title: `Khởi động lại ${device.name}?`,
      message: "Thiết bị sẽ ngắt kết nối vài phút và stream dừng cho tới khi khởi động xong.",
      confirmLabel: "Khởi động lại",
      danger: true,
    });
    if (!proceed) return;
    try {
      await runBusy(async () => {
        await rebootDevice(device.udid);
      });
      pushToast("info", "Đang khởi động lại", device.name);
    } catch (e) {
      toastError("Khởi động lại thất bại", e);
    }
  };

  const backup = async () => {
    const dir = await pickDirectory("Chọn thư mục lưu backup");
    if (!dir) return;
    pushToast("info", "Đang backup…", `${device.name} — có thể mất vài phút.`);
    try {
      await runBusy(async () => {
        await backupDevice(device.udid, dir);
      });
      pushToast("ok", "Backup xong", dir);
    } catch (e) {
      toastError("Backup thất bại", e);
    }
  };

  const restore = async () => {
    const dir = await pickDirectory("Chọn thư mục backup để phục hồi");
    if (!dir) return;
    const proceed = await requestConfirm({
      title: `Phục hồi ${device.name} từ backup?`,
      message: "Toàn bộ dữ liệu hiện tại trên thiết bị sẽ bị ghi đè và máy sẽ khởi động lại.",
      confirmLabel: "Ghi đè & phục hồi",
      danger: true,
    });
    if (!proceed) return;
    pushToast("info", "Đang phục hồi…", device.name);
    try {
      await runBusy(async () => {
        await restoreDevice(device.udid, dir);
      });
      pushToast("ok", "Đã phục hồi", "Thiết bị sẽ khởi động lại.");
    } catch (e) {
      toastError("Phục hồi thất bại", e);
    }
  };

  type MenuRow = {
    id: string;
    label: string;
    Icon: (props: { size?: number }) => ReactElement;
    androidOnly?: boolean;
    danger?: boolean;
    disabled?: boolean;
    run: () => void;
  };

  const menuRows: MenuRow[] = [
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
      // GenFarmer has this row here, in this panel, immediately after Power button —
      // which is why it also lives here and not only in the tile's right-click menu.
      id: "rotate",
      label: "Quay màn hình",
      Icon: IconRefresh,
      androidOnly: true,
      disabled: busy,
      run: () => {
        void setScreenRotation(device.udid, 1)
          .then((observed) =>
            observed === 1
              ? pushToast("ok", "Đã quay ngang")
              : pushToast(
                  "warn",
                  "Máy không quay",
                  "App đang mở khoá hướng dọc nên hệ thống bỏ qua yêu cầu.",
                ),
          )
          .catch((error) => toastError("Quay màn hình thất bại", error));
      },
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
      id: "adb",
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
          if (next && keyboards === null) void loadKeyboards();
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
    {
      // No `androidOnly`. Whether a phone can be enumerated is the backend's answer,
      // arriving as a refusal with a reason; a hardcoded platform gate here would be a
      // guess that goes stale the moment the iOS route lands.
      id: "apps",
      label: showApps ? "Ẩn ứng dụng" : "Ứng dụng",
      Icon: IconGrid,
      run: () => setShowApps((open) => !open),
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

  const navKeys: { key: HardwareKey; title: string; Icon: (props: { size?: number }) => ReactElement }[] =
    isIos
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
          style={{ width: frameWidth, height: frameWidth * aspect }}
          title="Ctrl + lăn chuột để phóng to / thu nhỏ"
          onPointerDown={(e) => {
            if (busy || inFlight.current || e.button !== 0 || !encodedW || !encodedH) return;
            const start = mapToDevice(e.currentTarget, e.clientX, e.clientY, encodedW, encodedH);
            if (!start) return;
            e.preventDefault();
            drag.current = { start, steps: [], lastAt: performance.now() };
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
            const point = mapToDevice(e.currentTarget, e.clientX, e.clientY, encodedW, encodedH);
            if (!point) return;
            const previous = held.steps.at(-1) ?? held.start;
            if (Math.hypot(point.x - previous.x, point.y - previous.y) < 2) return;
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
            if (e.button !== 0 || !drag.current || !encodedW || !encodedH) {
              drag.current = null;
              return;
            }
            e.preventDefault();
            const held = drag.current;
            const end = mapToDevice(e.currentTarget, e.clientX, e.clientY, encodedW, encodedH);
            drag.current = null;
            if (!end) return;
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
              await runGesture(held.start, end, steps);
            } catch (error) {
              toastError("Điều khiển thất bại", error);
            }
          }}
          onPointerCancel={() => {
            drag.current = null;
          }}
        >
          <PhoneCanvas
            udid={device.udid}
            surfaceId="overlay"
            fill
            className={`focus-touch${busy ? " is-busy" : ""}`}
          />
          {!hasView && <div className="screen-empty">Đang chờ stream…</div>}
        </div>
        <aside className="focus-menu" aria-label="Chức năng thiết bị">
          <header className="focus-menu-head">
            <strong title={device.udid}>
              {index} {device.name}
            </strong>
            {groupMode && targets.length > 1 && (
              <span className="focus-menu-group" title={`Đồng bộ ${targets.length} máy`}>
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
            <button type="button" className="close" title="Đóng" aria-label="Đóng" onClick={onClose}>
              <IconClose size={14} />
            </button>
          </header>
          <div className="focus-menu-list">
            {menuRows.map(({ id, label, Icon, androidOnly, danger, disabled, run }) => {
              const blocked = Boolean(androidOnly && isIos);
              return (
                <button
                  key={id}
                  type="button"
                  className={danger ? "danger" : ""}
                  disabled={blocked || disabled}
                  title={blocked ? `${label} — chỉ có trên Android` : label}
                  onClick={run}
                >
                  <Icon size={16} />
                  <span>{label}</span>
                </button>
              );
            })}
          </div>
          {/* Every one of these sits BEFORE the navbar and carries its own height. The menu
              list is `flex: 1`, so a sibling added after the navbar collapses to nothing and
              pushes the navbar out of the column (AGENTS.md §9.57). */}
          {showDevices && (
            <div className="focus-menu-panel" role="group" aria-label="Đổi máy">
              {devices.length <= 1 ? (
                <p className="hint">Chỉ có một máy đang hiển thị.</p>
              ) : (
                devices.map((candidate, position) => (
                  <button
                    key={candidate.udid}
                    type="button"
                    className={candidate.udid === device.udid ? "is-current" : ""}
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
            <div className="focus-menu-panel" role="group" aria-label="Câu nhanh">
              <input
                type="text"
                value={phraseName}
                placeholder="Tên (không bắt buộc)"
                onChange={(event) => setPhraseName(event.target.value)}
              />
              <input
                type="text"
                value={phraseContent}
                placeholder="Nội dung (vd: xin chào)"
                onChange={(event) => setPhraseContent(event.target.value)}
                onKeyUp={(event) => {
                  if (event.key === "Enter") savePhrase();
                }}
              />
              <button type="button" className="ghost" onClick={() => savePhrase()}>
                Lưu câu
              </button>
              {phraseError && <p className="error">{phraseError}</p>}
              {!phrases.length ? (
                <p className="hint">Chưa có câu nào.</p>
              ) : (
                phrases.map((phrase) => (
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
                      onClick={() => {
                        const next = removeQuickPhrase(phrases, phrase.id);
                        setPhrases(next);
                        storeQuickPhrases(next);
                      }}
                    >
                      <IconClose size={12} />
                    </button>
                  </div>
                ))
              )}
            </div>
          )}
          {showKeyboards && (
            <div className="focus-menu-panel" role="group" aria-label="Đổi bàn phím">
              {keyboards === null ? (
                <p className="hint">Đang đọc…</p>
              ) : !keyboards.length ? (
                <p className="hint">Không đọc được bàn phím nào.</p>
              ) : (
                keyboards.map((method) => (
                  <button
                    key={method.id}
                    type="button"
                    disabled={busy}
                    className={method.id === currentKeyboard ? "is-current" : ""}
                    title={method.id}
                    onClick={() => void chooseKeyboard(method)}
                  >
                    {method.label}
                    {method.id === currentKeyboard && <span> ✓</span>}
                  </button>
                ))
              )}
            </div>
          )}
          {showApps && <InstalledApps udid={device.udid} deviceName={device.name} />}
          {showAdb && <AdbConsole device={device} onClose={() => setShowAdb(false)} />}
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
