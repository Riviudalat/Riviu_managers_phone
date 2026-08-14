import { prepareDevice, viewEnsure } from "./api";
import type { DeviceInfo } from "./types";

/**
 * Start the *view* path for one phone. Android is scrcpy (`viewEnsure`);
 * iOS is session-then-MJPEG (`prepareDevice`).
 *
 * Toolbar Start used to call `prepareDevice` for every UDID. On Android that
 * is the nurture path: occupy ManualControl, foreground TikTok, wait up to
 * 40 s, then force `tileStreamState = Parked`. Tile Start already did the
 * right thing; this is the one function both buttons must share.
 */
export async function startDevicePreview(device: DeviceInfo): Promise<void> {
  if (device.platform === "android") {
    await viewEnsure(device.udid);
    return;
  }
  // `prepareDevice` answers with the refreshed DeviceInfo, which neither caller uses —
  // the registry event carries the same update. Discarded rather than widening the return
  // type, so the two platform paths keep one shape.
  await prepareDevice(device.udid);
}

/** Android phones start in parallel; iPhones stay sequential (USB / WDA). */
export async function startFleetPreview(devices: DeviceInfo[]): Promise<void> {
  const android = devices.filter((device) => device.platform === "android");
  const ios = devices.filter((device) => device.platform !== "android");
  await Promise.all(android.map((device) => startDevicePreview(device)));
  for (const device of ios) {
    await startDevicePreview(device);
  }
}
