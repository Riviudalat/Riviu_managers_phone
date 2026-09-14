import { useCallback, useEffect, useState } from "react";
import type { DeviceInfo } from "../types";

export function useDeviceWindows(devices: DeviceInfo[]) {
  const [openUdids, setOpenUdids] = useState<string[]>([]);
  const [activeUdid, setActiveUdid] = useState<string | null>(null);
  const open = useCallback((udid: string | null) => {
    if (!udid) { setOpenUdids([]); setActiveUdid(null); return; }
    setOpenUdids(current => current.length === 1 && current[0] === udid ? current : [udid]);
    setActiveUdid(udid);
  }, []);
  const close = useCallback((udid: string) => {
    setOpenUdids(current => current.filter(id => id !== udid));
    setActiveUdid(current => current === udid ? null : current);
  }, []);
  const roster = devices.map(device => device.udid).join("\0");
  useEffect(() => {
    if (!roster) return;
    const present = new Set(roster.split("\0"));
    setOpenUdids(current => current.every(id => present.has(id)) ? current : current.filter(id => present.has(id)));
    setActiveUdid(current => current && present.has(current) ? current : null);
  }, [roster]);
  return { openUdids, activeUdid, open, close, activate: setActiveUdid };
}
