import { deviceRosterView, deviceStatusText } from "../deviceWork";
import type { DeviceOperationalView } from "../deviceWork";
import type { DevicePlatform, DeviceStatus } from "../types";

/**
 * A picker only has the roster row: no work owner and no helper proof. Without a platform the
 * phone is read as Android, the stricter rule, so a picker never claims Sẵn sàng it cannot prove.
 */
export function machineStatusView(status: string, reason?: string | null, platform: DevicePlatform = "android"): DeviceOperationalView {
  return deviceRosterView({ platform, status: status as DeviceStatus, wdaReady: false, lastError: reason ?? null });
}

export function machineStatusLabel(status: string) {
  return deviceStatusText(machineStatusView(status));
}
