import type { DeviceInfo } from "./types";

/**
 * Which devices a farm action applies to: the selection, or the whole fleet when empty.
 *
 * "Nothing selected" means "everything" everywhere in this app, and this is the one place
 * that says so. It used to be exported from `SelectionStrip.tsx`, which made four callers
 * import a selection rule from a component they only sometimes rendered.
 *
 * The one exception is a selection emptied because its phones went busy or offline
 * (`excluded` > 0, the "Bị loại" list): that is an empty scope, never the fleet. Otherwise
 * the last selected phone going busy would quietly widen the next command to every phone.
 */
export function targetsOf(selected: string[], devices: DeviceInfo[], excluded = 0): string[] {
  if (selected.length) return selected;
  return excluded > 0 ? [] : devices.map((d) => d.udid);
}
