import type { DeviceGroup, DeviceInfo, TargetRef } from "./types";

/** Resolves a UI target reference against the roster visible at execution time. */
export function resolveAutomationTarget(
  target: TargetRef,
  devices: DeviceInfo[],
  groups: DeviceGroup[],
): string[] {
  const roster = new Set(devices.map((device) => device.udid));
  if (target.type === "all") return devices.map((device) => device.udid);
  if (target.type === "group") {
    const members = new Set(groups.find((group) => group.id === target.groupId)?.udids ?? []);
    return devices.filter((device) => members.has(device.udid)).map((device) => device.udid);
  }
  const seen = new Set<string>();
  return target.udids.filter((udid) => {
    if (!roster.has(udid) || seen.has(udid)) return false;
    seen.add(udid);
    return true;
  });
}
