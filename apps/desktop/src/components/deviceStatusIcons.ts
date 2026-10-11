import { CircleCheck, CircleDashed, CircleHelp, CirclePlay, TriangleAlert, Unplug, type LucideIcon } from "lucide-react";
import type { DeviceOperationalStatus } from "../deviceWork";

/** One icon per condensed state, so a status reads without colour (and in black and white). */
export const DEVICE_STATUS_ICONS: Record<DeviceOperationalStatus, LucideIcon> = {
  ready: CircleCheck,
  unchecked: CircleDashed,
  busy: CirclePlay,
  warning: TriangleAlert,
  offline: Unplug,
  unknown: CircleHelp,
};
