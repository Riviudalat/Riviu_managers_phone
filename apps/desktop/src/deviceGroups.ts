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

/** One box section of the wall: a saved group, or the phones no group lists. */
export interface WallBoxSection {
  id: string;
  label: string;
  /** The phones drawn after search, status and connection filters. */
  devices: DeviceInfo[];
  /** The box's whole present roster: filters never change these counts (PLAN §2). */
  total: number;
  ready: number;
  busy: number;
  warning: number;
}

/** The pseudo-section for phones no box lists. Never saved; never derived from Máy N. */
export const UNASSIGNED_BOX_ID = "__unassigned__";

/**
 * The wall's box sections (PLAN §2). A box is the list of udids the operator assigned, so a
 * phone's section comes from membership alone — never from its Máy N. Sections keep the
 * groups' own order; inside one, phones keep the order they arrive in (number order), so a
 * status change never moves a tile. A phone listed by two boxes shows once, under the first;
 * phones no box lists close the wall under "Chưa thuộc box". With a box chip active only that
 * box's section is shown. Totals and status counts come from `roster` (every present phone),
 * so a search narrows the tiles without shrinking the box; `shown` is what the filters leave.
 */
export function wallBoxSections(
  shown: DeviceInfo[],
  roster: DeviceInfo[],
  groups: DeviceGroup[],
  activeTab: string,
  views: ReadonlyMap<string, { kind: string }>,
): WallBoxSection[] {
  const visible = new Set(shown.map((device) => device.udid));
  const placed = new Set<string>();
  const section = (id: string, label: string, members: DeviceInfo[]): WallBoxSection => ({
    id,
    label,
    devices: shown.filter((device) => members.some((member) => member.udid === device.udid)),
    total: members.length,
    ready: members.filter((device) => views.get(device.udid)?.kind === "ready").length,
    busy: members.filter((device) => views.get(device.udid)?.kind === "busy").length,
    warning: members.filter((device) => views.get(device.udid)?.kind === "warning").length,
  });
  const sections: WallBoxSection[] = [];
  for (const group of groups) {
    const listed = new Set(group.udids);
    const members = roster.filter((device) => listed.has(device.udid) && !placed.has(device.udid));
    members.forEach((device) => placed.add(device.udid));
    if (activeTab !== ALL_DEVICES_TAB && activeTab !== group.id) continue;
    if (members.some((device) => visible.has(device.udid))) sections.push(section(group.id, group.name, members));
  }
  const rest = roster.filter((device) => !placed.has(device.udid));
  if (rest.some((device) => visible.has(device.udid)) && (activeTab === ALL_DEVICES_TAB || !groups.some((group) => group.id === activeTab))) {
    sections.push(section(UNASSIGNED_BOX_ID, "Chưa thuộc box", rest));
  }
  return sections;
}
