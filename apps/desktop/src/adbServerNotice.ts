import { pushToast, type ToastKind } from "./toastStore";
import type { AppEvent } from "./types";

type AdbServerNoticeEvent = Extract<AppEvent, { type: "adbServerNotice" }>;

/**
 * What the activity history shows when the ADB server under the farm goes away or comes back.
 *
 * Measured 08/10/2026: another tool's adb replaced the shared server on port 5037 and all 30
 * scrcpy tiles went black at once with nothing on screen saying why. "lost"/"returned" are
 * another tool's restart seen by the roster reads; "restarted" is Riviu's own bounded restart
 * of a hung server (HX-ADB), which must never read as someone else's.
 */
export function adbServerToast(event: AdbServerNoticeEvent): { kind: ToastKind; title: string; detail: string } {
  if (event.change === "lost") {
    return {
      kind: "warn",
      title: `ADB bị phần mềm khác khởi động lại · ${event.transports} máy`,
      detail: event.message,
    };
  }
  if (event.change === "restarted") {
    return { kind: "warn", title: `Riviu đã khởi động lại ADB (cổng ${event.port})`, detail: event.message };
  }
  return { kind: "info", title: `ADB đã chạy lại (cổng ${event.port})`, detail: event.message };
}

export function announceAdbServerNotice(event: AdbServerNoticeEvent) {
  const toast = adbServerToast(event);
  pushToast(toast.kind, toast.title, toast.detail);
}
