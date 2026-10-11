import { useEffect, useMemo, useState } from "react";
import { listDeviceWorkStates } from "./api";
import { newWorkBlock, type ExcludedDevice } from "./selectionEligibility";
import type { DeviceInfo, DeviceWorkOwner } from "./types";

/**
 * Busy phones in an automation scope (PLAN §3, §8): a phone that is running work is "Bị loại"
 * and gets no command. There is no takeover yet: the operator stops the old job from Lượt chạy.
 *
 * An exclusion is latched: a phone that frees up stays out of the receiving set until the
 * operator takes it out of the scope and chooses it again. Only positive evidence excludes.
 */
export function useBusyScope(devices: readonly DeviceInfo[], targets: readonly string[]) {
  const [owners, setOwners] = useState<ReadonlyMap<string, DeviceWorkOwner | null>>(new Map());
  const [latched, setLatched] = useState<ReadonlyMap<string, string>>(new Map());
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
  const targetKey = targets.join(",");
  useEffect(() => {
    setLatched(current => {
      const scope = new Set(targetKey ? targetKey.split(",") : []);
      const next = new Map([...current].filter(([udid]) => scope.has(udid)));
      for (const udid of scope) {
        const device = devices.find(d => d.udid === udid);
        if (!device || device.status === "disconnected" || next.has(udid)) continue;
        const reason = newWorkBlock(device, owners.get(udid));
        if (reason) next.set(udid, reason);
      }
      return next.size === current.size && [...next].every(([udid, reason]) => current.get(udid) === reason) ? current : next;
    });
  }, [devices, owners, targetKey]);
  const excluded = useMemo<ExcludedDevice[]>(
    () => targets.flatMap(udid => latched.has(udid) ? [{ udid, reason: latched.get(udid) ?? "Đang chạy tác vụ", at: 0 }] : []),
    [targets, latched],
  );
  const runTargets = useMemo(
    () => targets.filter(udid => !latched.has(udid)),
    [targets, latched],
  );
  return { excluded, runTargets };
}
