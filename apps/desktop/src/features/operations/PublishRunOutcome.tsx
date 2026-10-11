import type { ReactNode } from "react";
import type { OperationRunItem } from "../../types";
import { PostOutcomeEvidence, PostOutcomeLayers, PostOutcomeTotals } from "./PostOutcome";
import { postOutcomeTotals, runItemsPostOutcome } from "./postOutcomeModel";
import { publishRunOutcomes, usePublishRunRead } from "./publishRunRead";

/** The campaign's separate totals for the operation monitor and Lượt chạy, from the shared poll. */
export function PublishRunOutcomeTotals({ campaignId }: { campaignId: string }) {
  const state = usePublishRunRead(campaignId);
  if (state.error) return <small role="status">Chưa đọc được kết quả bài đăng: {state.error}</small>;
  if (!state.value) return <small role="status">Đang đọc kết quả bài đăng…</small>;
  return <PostOutcomeTotals totals={postOutcomeTotals([...publishRunOutcomes(state.value).values()])} />;
}

/** One Lượt chạy item's post result: the full assignment when the shared poll has it. */
export function PublishItemOutcome({ campaignId, item, fallback }: {
  campaignId: string;
  item: Pick<OperationRunItem, "id" | "state" | "evidence" | "errorCode">;
  /** Shown while the post has not been submitted (preparation, transfer, stop before Post). */
  fallback: ReactNode;
}) {
  const state = usePublishRunRead(campaignId);
  const outcome = publishRunOutcomes(state.value).get(item.id) ?? runItemsPostOutcome([item]);
  if (!outcome || outcome.post === "notSent") return <>{fallback}</>;
  return <><PostOutcomeLayers outcome={outcome} /><PostOutcomeEvidence outcome={outcome} /></>;
}
