import type { DeviceGroup, DeviceInfo } from "./types";

/** The pseudo-tab that is not a group and must never be saved as one. */
export const ALL_DEVICES_TAB = "__all__";

export interface GroupTab {
  id: string;
  label: string;
  /** How many *currently connected* devices this tab would show. */
  count: number;
  /** Group colour, or null for the all-devices tab. */
  color: string | null;
}

/**
 * The tab strip above the device grid.
 *
 * Counts are of devices actually present, not of the udids stored in the group. A group
 * remembers phones that may be unplugged; a tab labelled with the stored count would
 * promise rows the grid cannot show, so the number here is what the operator will see
 * when they click it.
 *
 * Empty groups are kept rather than hidden. A group whose phones are all unplugged is
 * information — it says "these are elsewhere", and silently dropping the tab makes the
 * group look deleted.
 */
export function groupTabs(devices: DeviceInfo[], groups: DeviceGroup[]): GroupTab[] {
  const present = new Set(devices.map((device) => device.udid));
  return [
    {
      id: ALL_DEVICES_TAB,
      label: "Tất cả",
      count: devices.length,
      color: null,
    },
    ...groups.map((group) => ({
      id: group.id,
      label: group.name,
      count: group.udids.filter((udid) => present.has(udid)).length,
      color: group.color,
    })),
  ];
}

/**
 * The devices one tab shows, in the order the fleet listing gave them.
 *
 * A tab whose group has disappeared falls back to every device rather than to none.
 * Showing an empty grid because a group was deleted in another window looks exactly
 * like a fleet that vanished, and the operator cannot tell which happened.
 */
export function devicesInTab(
  devices: DeviceInfo[],
  groups: DeviceGroup[],
  tabId: string,
): DeviceInfo[] {
  if (tabId === ALL_DEVICES_TAB) return devices;
  const group = groups.find((candidate) => candidate.id === tabId);
  if (!group) return devices;
  const members = new Set(group.udids);
  return devices.filter((device) => members.has(device.udid));
}

/**
 * The group list with one device added to one group, ready to save.
 *
 * Returns null when there is nothing to do — the group is unknown, or the device is
 * already in it. A caller that saves regardless would rewrite `updatedAt` and make an
 * idempotent click look like a change.
 */
export function withDeviceAdded(
  groups: DeviceGroup[],
  groupId: string,
  udid: string,
): DeviceGroup | null {
  const group = groups.find((candidate) => candidate.id === groupId);
  if (!group || group.udids.includes(udid)) return null;
  return { ...group, udids: [...group.udids, udid] };
}
