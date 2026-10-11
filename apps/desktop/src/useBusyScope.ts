import { useCallback, useEffect, useMemo, useState } from "react";
import { listDeviceWorkStates } from "./api";
import { requestConfirm } from "./confirmStore";
import { deviceWorkOwnerLabel } from "./deviceWork";
import { newWorkBlock, type ExcludedDevice } from "./selectionEligibility";
import type { DeviceInfo, DeviceWorkOwner } from "./types";

/**
 * Busy phones in an automation scope (PLAN §3, §8): a phone that is running work is "Bị loại"
 * by default and gets no command. Starting the run never stops its old job silently; only a
 * per-phone "Dừng việc cũ và dùng máy này", confirmed with the job named, takes it back.
 */
export function useBusyScope(devices: readonly DeviceInfo[], targets: readonly string[]) {
  const [taken, setTaken] = useState<string[]>([]);
  const [owners, setOwners] = useState<ReadonlyMap<string, DeviceWorkOwner | null>>(new Map());
  // Work owners name the running job; an unread owner leaves only the roster's own `busy`.
  useEffect(() => {
    let active = true;
    const read = () => void Promise.resolve().then(() => listDeviceWorkStates())
      .then(states => { if (active) setOwners(new Map(states.map(state => [state.udid, state.currentOwner ?? null]))); })
      .catch(() => undefined);
    read();
    const timer = window.setInterval(read, 5_000);
    return () => { active = false; window.clearInterval(timer); };
  }, []);
  const excluded = useMemo<ExcludedDevice[]>(() => targets.flatMap(udid => {
    const device = devices.find(d => d.udid === udid);
    if (!device || device.status === "disconnected" || taken.includes(udid)) return [];
    const reason = newWorkBlock(device, owners.get(udid));
    return reason ? [{ udid, reason, at: 0 }] : [];
  }), [devices, targets, taken, owners]);
  const runTargets = useMemo(
    () => targets.filter(udid => !excluded.some(entry => entry.udid === udid)),
    [targets, excluded],
  );
  const take = useCallback(async (udid: string, label: string) => {
    // Name the job being stopped from a fresh read; an unread owner stays named as unknown.
    const owner = await listDeviceWorkStates().then(states => states.find(state => state.udid === udid)?.currentOwner ?? null).catch(() => null);
    const job = owner ? deviceWorkOwnerLabel(owner) : "tác vụ chưa đọc được tên";
    const accepted = await requestConfirm({
      title: `Dừng việc cũ trên ${label}?`,
      message: `${label} đang chạy: ${job}. Khi bạn bắt đầu lượt mới, việc này sẽ bị dừng và máy chuyển sang lượt mới. Hành động việc cũ đã gửi không được thu hồi.`,
      confirmLabel: "Dừng việc cũ và dùng máy này",
      cancelLabel: "Giữ máy bị loại",
      danger: true,
    });
    if (accepted) setTaken(current => [...new Set([...current, udid])]);
  }, []);
  return { excluded, runTargets, taken, take };
}
