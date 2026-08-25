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

/** One phone that could not be started, and why. */
export interface PreviewFailure {
  udid: string;
  name: string;
  reason: unknown;
}

/**
 * Android phones start in parallel; iPhones stay sequential (USB / WDA).
 *
 * **No phone's failure ends another phone's turn.** This used to be `Promise.all` over the
 * Android half followed by the iOS loop, so the first Android phone to reject took the
 * whole call down with it — and the iPhones underneath were never reached at all. On a
 * mixed fleet, one unplugged Android meant pressing Start did nothing for every iPhone in
 * the room, with a single error naming only the Android.
 *
 * Every device now gets its turn and the failures come back as a list, so the caller can
 * say which phones did not start instead of naming whichever one failed first.
 */
export async function startFleetPreview(devices: DeviceInfo[]): Promise<PreviewFailure[]> {
  const android = devices.filter((device) => device.platform === "android");
  const ios = devices.filter((device) => device.platform !== "android");
  const failures: PreviewFailure[] = [];

  const settled = await Promise.allSettled(
    android.map((device) => startDevicePreview(device)),
  );
  settled.forEach((result, index) => {
    if (result.status === "rejected") {
      failures.push({
        udid: android[index].udid,
        name: android[index].name,
        reason: result.reason,
      });
    }
  });

  // Sequential, and each one guarded on its own: a single iPhone whose WDA is unhappy must
  // not stop the ones queued behind it either.
  for (const device of ios) {
    try {
      await startDevicePreview(device);
    } catch (reason) {
      failures.push({ udid: device.udid, name: device.name, reason });
    }
  }
  return failures;
}
