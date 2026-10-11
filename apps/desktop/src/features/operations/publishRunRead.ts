import { useCallback } from "react";
import { publishGet, publishRecoveryCapabilities } from "../../api";
import type { PublishAssignmentRecord, PublishRecoveryCapability } from "../../types";
import { useMonitorRead } from "./useMonitorRead";
import { postOutcome, type PostOutcome } from "./postOutcomeModel";

/** `sheetRequired` is sent by the backend but not in the shared IPC type yet. */
export type PublishRunCapability = PublishRecoveryCapability & { sheetRequired?: boolean };

export interface PublishRunRead {
  assignments: PublishAssignmentRecord[];
  capabilities: PublishRunCapability[];
  createdAt?: string;
}

export async function readPublishRun(campaignId: string): Promise<PublishRunRead> {
  const [detail, capabilities] = await Promise.all([publishGet(campaignId), publishRecoveryCapabilities(campaignId)]);
  return { assignments: detail?.assignments ?? [], capabilities, createdAt: detail?.campaign.createdAt };
}

/** The query key every reader of one campaign's assignments shares (monitor, totals, queue). */
export const publishRunScope = (campaignId: string) => ["monitor", "publishRecovery", campaignId] as const;

/**
 * One poll per campaign, shared by every monitor row, the run totals, the Cần xử lý queue and
 * Lượt chạy: the scope key makes TanStack keep a single in-flight read however many surfaces
 * show the campaign.
 */
export function usePublishRunRead(campaignId: string, enabled = true) {
  const read = useCallback(() => readPublishRun(campaignId), [campaignId]);
  return useMonitorRead(read, 2000, ["publishRecovery", campaignId], enabled);
}

/** Each assignment's outcome, with the Sheet requirement its capability reports. */
export function publishRunOutcomes(value: PublishRunRead | null): Map<string, PostOutcome> {
  const outcomes = new Map<string, PostOutcome>();
  for (const assignment of value?.assignments ?? []) {
    const capability = value?.capabilities.find(item => item.assignmentId === assignment.id);
    outcomes.set(assignment.id, postOutcome(assignment, { sheetRequired: capability?.sheetRequired }));
  }
  return outcomes;
}
