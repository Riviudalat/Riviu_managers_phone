import type { PublishDeviceGuard, PublishDeviceGuards } from "../../types";

export const UNKNOWN_PUBLISH_GUARD = "Chưa kiểm tra được bài đang chờ";
export function deviceGuardBlock(guards: PublishDeviceGuards | undefined, udid: string): string | undefined {
  if (guards === undefined) return undefined;
  const guard = guards[udid];
  if (!guard) return UNKNOWN_PUBLISH_GUARD;
  if (guard.blocking.length) return "Máy còn bài chưa lấy được link";
  return undefined;
}
export function deviceGuardPending(guard: PublishDeviceGuard | undefined) {
  return guard?.blocking[0] ?? guard?.linkReview[0];
}
