import type { ReactNode } from "react";
import type { OperationRunItem } from "../../types";
import { PostOutcomeEvidence, PostOutcomeLayers, PostOutcomeTotals } from "./PostOutcome";
import { postOutcomeTotals } from "./postOutcomeModel";
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
  const outcome = publishRunOutcomes(state.value).get(item.id);
  // The run item lacks the frozen intent: until its assignment is read the result is unknown.
  if (!outcome) {
    if (state.error) return <small>Chưa rõ: chưa đọc được kết quả bài đăng</small>;
    if (!state.value) return <small>Đang đọc kết quả bài đăng…</small>;
    if (item.state === "succeeded" || item.state === "uncertain") return <small>Chưa rõ: chưa đọc được bản ghi của bài này</small>;
    return <>{fallback}</>;
  }
  if (outcome.post === "notSent") return <>{fallback}</>;
  return <><PostOutcomeLayers outcome={outcome} /><PostOutcomeEvidence outcome={outcome} /></>;
}
