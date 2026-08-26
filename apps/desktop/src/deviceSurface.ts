/**
 * When a per-phone surface has to close because its phone left the fleet.
 *
 * **Three states in `App.tsx` hold "which phone is this surface open for"** — `adbFor`,
 * `filesFor`, `focusUdid` — and each one resolves through `devices.find(...) ?? null` into a
 * render gated on the result. None of them cleared the udid when the phone went away, and the
 * failure that produces is worse than it sounds:
 *
 * 1. the roster churns (a USB re-enumeration, an adb server bounce — `useFleet` replaces the
 *    whole roster on every `devicesUpdated`, so a single scan that reports zero phones is
 *    enough);
 * 2. the resolver returns `null` and the surface **unmounts with no message**;
 * 3. the udid stays in state, so clicking the same phone's row calls `setState` with the value
 *    it already holds, React bails out of the re-render, and **the row does nothing** — for
 *    that phone, permanently, until another phone is clicked or the app restarts.
 *
 * That is the operator's report: *"mở thư mục máy điện thoại còn mở không được"*.
 *
 * `controlCenter` already had this effect (with a doc comment making the argument), 470 lines
 * above the surface that needed it most. This is that effect, extracted so the next per-phone
 * surface cannot be written without it.
 */

/** Anything with a udid — the roster rows, and nothing else about them matters here. */
export interface HasUdid {
  udid: string;
}

/**
 * Whether a surface opened for `udid` should close because that phone is no longer in the fleet.
 *
 * **An empty roster is not a departure**, and that guard is the whole reason this is a function
 * rather than an inline `&&`. A roster is empty at boot before the first scan lands, and again
 * whenever a scan fails — `list_devices` reads until two consecutive `adb devices` agree, and a
 * restarting adb server can answer once with nothing. Closing every panel on that would make the
 * app shut its own windows during a blip it recovers from a second later.
 */
export function surfaceDeparted(devices: readonly HasUdid[], udid: string | null): boolean {
  if (!udid) return false;
  if (devices.length === 0) return false;
  return !devices.some((device) => device.udid === udid);
}
