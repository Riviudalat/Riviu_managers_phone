import type { AgentRepairOutcome, AgentState, AgentStatus } from "./types";

type StatusTone = "ok" | "warn" | "danger" | "info";

export interface AgentStatusView {
  label: string;
  tone: StatusTone;
  textCommentsEnabled: boolean;
  message: string | null;
}

export interface BulkRepairSummary {
  repairedCount: number;
  failedCount: number;
  helperPendingCount: number;
  attentionCount: number;
  heading: string;
  message: string;
}

const STATE_VIEW: Record<AgentState, Pick<AgentStatusView, "label" | "tone">> = {
  unknown: { label: "Chưa kiểm tra", tone: "info" },
  missing: { label: "Chưa cài Agent", tone: "warn" },
  repairRequired: { label: "Cần khôi phục Agent", tone: "warn" },
  starting: { label: "Đang khởi động", tone: "info" },
  ready: { label: "Sẵn sàng", tone: "ok" },
  error: { label: "Lỗi Agent", tone: "danger" },
};

function concise(message: string | null): string | null {
  if (!message) return null;
  const normalized = message.replace(/\s+/g, " ").trim();
  return normalized.length > 140 ? `${normalized.slice(0, 137)}...` : normalized;
}

export function agentStatusView(status: AgentStatus): AgentStatusView {
  return {
    ...STATE_VIEW[status.state],
    textCommentsEnabled:
      status.state === "ready" &&
      status.features.includes("text") &&
      status.authReady &&
      status.mjpegReady &&
      status.sessionReady,
    message: concise(status.message),
  };
}

/**
 * "Sửa agent" repairs the control agent only. A helper latched as failed overrides the
 * cached status, so the outcome's own flags, not the status state, say whether it worked.
 */
export function summarizeBulkRepair(outcomes: AgentRepairOutcome[]): BulkRepairSummary {
  const repairedCount = outcomes.filter((outcome) => outcome.agentRepaired).length;
  const failedCount = outcomes.length - repairedCount;
  const helperPendingCount = outcomes.filter((outcome) => outcome.agentRepaired && outcome.helperRecoveryRequired).length;
  const helper = helperPendingCount > 0
    ? ` ${helperPendingCount} máy đã sửa agent nhưng helper vẫn cần Khôi phục helper; Sửa agent không khôi phục helper.`
    : "";
  return {
    repairedCount,
    failedCount,
    helperPendingCount,
    attentionCount: failedCount + helperPendingCount,
    heading: `Sửa agent: ${repairedCount} máy đã sửa, ${failedCount} máy lỗi`,
    message: `${repairedCount} máy đã sửa agent; ${failedCount} máy sửa lỗi.${helper}`,
  };
}
