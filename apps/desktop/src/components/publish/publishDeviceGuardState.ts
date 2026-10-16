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
/** A scheduled phone with an earlier post owing its link is a note, not a block: its job
 * releases a post TikTok holds or waits at its own turn; preflight says which. */
export function deviceGuardScheduleNote(guards: PublishDeviceGuards | undefined, udid: string): { title: string; detail: string } | undefined {
  if (!guards?.[udid]?.blocking.length) return undefined;
  return { title: "Máy còn bài chờ link", detail: "Vẫn hẹn giờ được; tới giờ app tự xử lý, xem Kết quả kiểm tra" };
}
