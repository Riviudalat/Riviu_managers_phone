import { deviceRosterView, deviceStatusText } from "../deviceWork";
import type { DeviceOperationalView } from "../deviceWork";
import type { DevicePlatform, DeviceStatus } from "../types";

/**
 * A picker only has the roster row: no work owner and no Android helper proof. An iPhone reads its
 * own WDA proof from that row; without a platform the phone is read as Android, the stricter rule,
 * so a picker never claims Sẵn sàng it cannot prove.
 */
export function machineStatusView(status: string, reason?: string | null, platform: DevicePlatform = "android", wdaReady = false): DeviceOperationalView {
  return deviceRosterView({ platform, status: status as DeviceStatus, wdaReady, lastError: reason ?? null });
}

export function machineStatusLabel(status: string, platform?: DevicePlatform, wdaReady?: boolean) {
  return deviceStatusText(machineStatusView(status, null, platform, wdaReady));
}
