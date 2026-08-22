/**
 * What a tile is called and where it sits (xiaowei "Change Name" / "Change Number").
 *
 * Pure, because all three rules here are the kind that look obvious and are wrong in one
 * corner: falling back to the phone's own name, falling back to the grid position, and
 * ordering a fleet where only *some* phones have been numbered. That last one is the reason
 * this is a module and not three inline expressions — a sort that drops the unnumbered
 * phones, or shuffles them, loses tiles from an operator's grid.
 */

import type { DeviceInfo, DeviceMeta } from "./types";

/** Records keyed by udid, for a grid that has to label twenty tiles per frame. */
export function metaByUdid(metas: DeviceMeta[]): Map<string, DeviceMeta> {
  return new Map(metas.map((meta) => [meta.udid, meta]));
}

/** The operator's own name for this phone, or the one the phone reports. */
export function tileName(device: DeviceInfo, meta: DeviceMeta | undefined): string {
  const alias = meta?.alias?.trim();
  return alias ? alias : device.name;
}

/**
 * The big number on the tile: the operator's, or the tile's 1-based position.
 *
 * `position` is what every tile showed before numbering existed, and it stays the fallback
 * rather than a blank — a grid of twenty tiles with no numbers is harder to talk about over
 * a shoulder than one numbered by accident of order.
 */
export function tileNumber(position: number, meta: DeviceMeta | undefined): number {
  return meta?.number ?? position;
}

/**
 * Numbered phones first in number order, then everything else in the order it arrived.
 *
 * Not a plain sort by `number ?? Infinity`: `Array.prototype.sort` is stable in every engine
 * this ships on, so that would work — but it also silently reorders the *unnumbered* tail
 * whenever two phones share a number, which is a state the UI allows (nothing stops an
 * operator typing 3 twice) and which must not scramble the grid. Partitioning says what
 * happens instead: duplicates keep their arrival order relative to each other.
 */
export function orderDevicesByNumber(
  devices: DeviceInfo[],
  metas: Map<string, DeviceMeta>,
): DeviceInfo[] {
  const numbered: { device: DeviceInfo; number: number; at: number }[] = [];
  const rest: DeviceInfo[] = [];
  devices.forEach((device, at) => {
    const number = metas.get(device.udid)?.number;
    if (typeof number === "number") numbered.push({ device, number, at });
    else rest.push(device);
  });
  numbered.sort((a, b) => (a.number === b.number ? a.at - b.at : a.number - b.number));
  return [...numbered.map((row) => row.device), ...rest];
}

/**
 * Read a number an operator typed, or say why not.
 *
 * `null` means "clear the number", which is a real thing to want and is what an emptied
 * field means. Everything else has to be a positive whole number: a phone numbered `0` sorts
 * before phone 1 and cannot be written on a sticker as anything meaningful, and `-3` or `2.5`
 * are typing accidents rather than intentions.
 */
export function parseDeviceNumber(raw: string): { number: number | null } | { error: string } {
  const text = raw.trim();
  if (!text) return { number: null };
  if (!/^\d+$/.test(text)) return { error: "Số máy phải là số nguyên dương." };
  const value = Number(text);
  if (value < 1) return { error: "Số máy phải từ 1 trở lên." };
  if (value > 9999) return { error: "Số máy tối đa là 9999." };
  return { number: value };
}
