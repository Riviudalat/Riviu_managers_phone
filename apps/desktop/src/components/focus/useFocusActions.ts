import type { MutableRefObject } from "react";
import type { HardwareKey } from "../../types";
import { getGroupSync } from "../../groupSync";
import { recordKey } from "../../macroStore";
import {
  backupDevice,
  deviceKey,
  deviceTypeText,
  exportMedia,
  groupInput,
  importMedia,
  rebootDevice,
  restoreDevice,
  saveViewSnapshot,
  screenshot,
} from "../../api";
import { type QuickPhrase } from "../../quickPhrases";
import { pickFile } from "../../pickFile";
import { pickDirectory } from "../../pickFile";
import { requestConfirm } from "../../confirmStore";
import { pushToast, toastError } from "../../toastStore";
import { exportViewJpeg } from "../../viewStore";
import type { DeviceInfo, GroupInputReport } from "../../types";

/// What the nine actions need from the component, and nothing more.
///
/// Measured: six symbols for 195 lines. `runExclusive` and `runBusy` are the two ways this
/// phone serialises work — one silent, one that tells the operator it refused — and they stay
/// in the component because the busy flag they drive is component state.
export type FocusActionDeps = {
  device: DeviceInfo;
  /// This phone alone, or the whole group when group mode is on.
  targets: string[];
  /// Phones whose manual-session lease is already open, so a gesture need not wait again.
  controlReady: MutableRefObject<Set<string>>;
  reportGroup: (report: GroupInputReport, quiet: boolean) => boolean;
  runExclusive: (work: () => Promise<void>) => Promise<void>;
  runBusy: (work: () => Promise<void>) => Promise<boolean>;
};

/// The nine things the Focus window can do *to* a phone, as opposed to *with* its screen.
///
/// Lifted out of `FocusStream` because they are the part of it that has nothing to do with
/// drawing: each one is a confirm, an API call, and a toast. The component keeps the gestures,
/// the zoom and the overlay — the things that only make sense next to a live frame.
///
/// Same shape as `useQuickPhrases` and `useDeviceKeyboards` next door.
export function useFocusActions({
  device,
  targets,
  controlReady,
  reportGroup,
  runExclusive,
  runBusy,
}: FocusActionDeps) {
  const pressKey = async (key: HardwareKey) => {
    recordKey(key); // A8: no-op unless a macro recording is armed.
    // Single-device gestures go through the manual-session lease; wait for control to open
    // rather than race it. The group path (`group_input`) skips and reports per device, so
    // it needs no gate.
    if (targets.length <= 1 && !controlReady.current.has(device.udid)) {
      pushToast("warn", "Đang mở điều khiển", "Đợi một giây rồi thử lại.");
      return;
    }
    try {
      await runExclusive(async () => {
        if (targets.length > 1) {
          reportGroup(
            await groupInput({
              udids: targets,
              kind: "key",
              key,
              sync: getGroupSync(),
            }),
            false,
          );
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
    if (targets.length <= 1 && !controlReady.current.has(device.udid)) {
      pushToast("warn", "Đang mở điều khiển", "Đợi một giây rồi thử lại.");
      return;
    }
    try {
      let delivered = false;
      const ran = await runBusy(async () => {
        if (targets.length > 1) {
          delivered = reportGroup(
            await groupInput({
              udids: targets,
              kind: "type",
              text: phrase.content,
              sync: getGroupSync(),
            }),
            false,
          );
        } else {
          await deviceTypeText(device.udid, phrase.content);
          delivered = true;
        }
      });
      if (ran && delivered) pushToast("ok", "Đã gõ câu nhanh", phrase.name);
    } catch (error) {
      toastError("Gõ câu nhanh thất bại", error);
    }
  };

  const importFile = async () => {
    const path = await pickFile({
      title: "Chọn ảnh hoặc video",
      filters: [
        {
          name: "Ảnh / video",
          extensions: [
            "jpg",
            "jpeg",
            "png",
            "webp",
            "gif",
            "mp4",
            "mov",
            "3gp",
          ],
        },
      ],
    });
    if (!path) return;
    pushToast("info", "Đang đưa vào máy…", device.name);
    try {
      await runBusy(async () => {
        pushToast(
          "ok",
          "Đã vào thư viện",
          await importMedia(device.udid, path),
        );
      });
    } catch (error) {
      toastError("Đưa file vào máy thất bại", error);
    }
  };

  const exportFiles = async () => {
    const dir = await pickDirectory("Chọn thư mục lưu ảnh/video lấy từ máy");
    if (!dir) return;
    // A full camera roll over USB 2.0 takes minutes, so say so before it starts rather than
    // leaving the operator watching a disabled button — and say *how much*, because the
    // number is the part nobody expects. Measured on 23021RAAEG: `/sdcard/DCIM` held **761
    // files, 3.3 GB**, and the row pulls all of it with no way to stop. An operator who
    // wanted three photos should use "Tệp trên máy…" and pick them.
    pushToast(
      "info",
      "Đang lấy TOÀN BỘ ảnh/video…",
      `${device.name} — cả thư viện, có thể vài GB và vài phút. Muốn lấy vài tệp thì dùng "Tệp trên máy…".`,
    );
    try {
      await runBusy(async () => {
        const report = await exportMedia(device.udid, dir);
        if (report.found === 0) {
          // Not an error: an empty gallery is an answer, and reporting it as a failure
          // would send the operator looking for a bug that is not there.
          pushToast("info", "Máy không có ảnh/video nào", device.name);
        } else if (report.missed > 0) {
          // Said out loud, and as a warning. These files were on the phone and are not on
          // this machine; a plain "Đã lấy N file" reads as success and quietly loses the
          // rest, which is the whole complaint.
          pushToast(
            "warn",
            `Chỉ lấy được ${report.fetched}/${report.found} file`,
            `${report.missed} file trên máy không sao chép được — xem log để biết file nào. Đã lưu vào ${dir}`,
          );
        } else {
          pushToast("ok", `Đã lấy ${report.fetched} file`, dir);
        }
      });
    } catch (error) {
      toastError("Lấy ảnh/video thất bại", error);
    }
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
          pushToast(
            "ok",
            "Đã chụp màn hình",
            await saveViewSnapshot(device.udid, Array.from(bytes)),
          );
        }
      });
    } catch (e) {
      toastError("Chụp màn hình thất bại", e);
    }
  };

  const reboot = async () => {
    const proceed = await requestConfirm({
      title: `Khởi động lại ${device.name}?`,
      message:
        "Thiết bị sẽ ngắt kết nối vài phút và stream dừng cho tới khi khởi động xong.",
      confirmLabel: "Khởi động lại",
      danger: true,
    });
    if (!proceed) return;
    try {
      const ran = await runBusy(async () => {
        await rebootDevice(device.udid);
      });
      if (ran) pushToast("info", "Đang khởi động lại", device.name);
    } catch (e) {
      toastError("Khởi động lại thất bại", e);
    }
  };

  const backup = async () => {
    const dir = await pickDirectory("Chọn thư mục lưu backup");
    if (!dir) return;
    pushToast("info", "Đang backup…", `${device.name} — có thể mất vài phút.`);
    try {
      const ran = await runBusy(async () => {
        await backupDevice(device.udid, dir);
      });
      if (ran) pushToast("ok", "Backup xong", dir);
    } catch (e) {
      toastError("Backup thất bại", e);
    }
  };

  const restore = async () => {
    const dir = await pickDirectory("Chọn thư mục backup để phục hồi");
    if (!dir) return;
    const proceed = await requestConfirm({
      title: `Phục hồi ${device.name} từ backup?`,
      message:
        "Toàn bộ dữ liệu hiện tại trên thiết bị sẽ bị ghi đè và máy sẽ khởi động lại.",
      confirmLabel: "Ghi đè & phục hồi",
      danger: true,
    });
    if (!proceed) return;
    pushToast("info", "Đang phục hồi…", device.name);
    try {
      const ran = await runBusy(async () => {
        await restoreDevice(device.udid, dir);
      });
      if (ran) pushToast("ok", "Đã phục hồi", "Thiết bị sẽ khởi động lại.");
    } catch (e) {
      toastError("Phục hồi thất bại", e);
    }
  };

  return {
    pressKey,
    sendPhrase,
    importFile,
    exportFiles,
    copySerial,
    capture,
    reboot,
    backup,
    restore,
  };
}
