import type { DeviceInfo } from "./types";

/**
 * Which devices a farm action applies to: the selection, or the whole fleet when empty.
 *
 * "Nothing selected" means "everything" everywhere in this app, and this is the one place
 * that says so. It used to be exported from `SelectionStrip.tsx`, which made four callers
 * import a selection rule from a component they only sometimes rendered.
 */
export function targetsOf(selected: string[], devices: DeviceInfo[]): string[] {
  return selected.length ? selected : devices.map((d) => d.udid);
}
