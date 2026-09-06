export type DeviceAutomation = "nurture" | "interaction" | "publish";

export function isDeviceAutomation(value: string): value is DeviceAutomation {
  return value === "nurture" || value === "interaction" || value === "publish";
}
