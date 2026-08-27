import {
  deviceGetClipboard,
  deviceKey,
  deviceSetClipboard,
  deviceShell,
  deviceSwipe,
  getDeviceMeta,
  disableWifiAdb,
  enableWifiAdb,
  exportMedia,
  importMedia,
  launchDeviceApp,
  installIpa,
  listDeviceMetas,
  listInstalledApps,
  openSystemSettings,
  powerOffDevice,
  rebootDevice,
  refreshDevices,
  resetDisplayMetrics,
  saveDeviceMeta,
  screenshot,
  screenshotToDevice,
  setInputMethod,
  setScreenLocked,
  setScreenRotation,
  setWifiRadio,
  wakeScreen,
} from "./api";
import { requestConfirm, requestPrompt } from "./confirmStore";
import { pushToast, toastError } from "./toastStore";
import type { DeviceMenuNode } from "./deviceMenu";
import { parseDeviceNumber } from "./deviceNaming";
import { parseCurrentInputMethod, parseInputMethods } from "./imeList";
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
import type { DeviceInfo, DeviceMeta, HardwareKey } from "./types";
import { pickDirectory, pickFile } from "./pickFile";

/**
 * Everything the catalog needs from the shell that renders it.
 *
 * Ten values, which is the whole reason this could move: the catalog reads five pieces of
 * state and calls five setters, and touches nothing else in `App`. Passing them explicitly
 * is what makes the catalog testable — before this it could only be reached by rendering the
 * entire application.
 */
export interface DeviceActionDeps {
  /** Re-read devices and jobs from the backend. */
  reload: () => Promise<void>;
  metaMap: Map<string, DeviceMeta>;
  metas: DeviceMeta[];
  setMetas: (next: DeviceMeta[]) => void;
  /** Which device has the control centre open, if any. */
  controlCenter: string | null;
  setControlCenter: (udid: string | null) => void;
  /** True while the grid is selecting devices for a group action. */
  groupMode: boolean;
  setFocusUdid: (udid: string | null) => void;
  setFilesFor: (udid: string | null) => void;
  setAdbFor: (udid: string | null) => void;
  setSyslogFor: (udid: string | null) => void;
}

/**
 * The right-click menu and the focus-view function list for one device.
 *
 * Lifted out of `App.tsx`, where it was 696 of 1,788 lines — 38% of the shell — and where
 * the only way to exercise a single row was to mount the whole app. It sits beside
 * `deviceMenu.ts`, which owns the shape it returns.
 */
export function buildDeviceActions(
  device: DeviceInfo,
  deps: DeviceActionDeps,
): DeviceMenuNode[] {
  const {
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
  } = deps;

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
          id: "device-syslog",
          label: "Log của máy…",
          // Not `androidOnly`: `syslog_tail` is on the driver trait and both backends
          // implement it, which is the whole reason it is worth reading from here.
          keywords: "syslog logcat log nhat ky",
          run: () => setSyslogFor(device.udid),
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
}
