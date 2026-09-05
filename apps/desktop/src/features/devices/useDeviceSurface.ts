import { useCallback, useEffect, useRef, useState } from "react";
import type { DeviceInfo } from "../../types";
import { surfaceDeparted } from "../../deviceSurface";
import { pushToast } from "../../toastStore";

/** Closes and announces a departed phone; transient empty scans preserve the open identity. */
export function useDeviceSurface(
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
