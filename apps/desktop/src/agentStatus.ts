import type { AgentState, AgentStatus } from "./types";

type StatusTone = "ok" | "warn" | "danger" | "info";

export interface AgentStatusView {
  label: string;
  tone: StatusTone;
  textCommentsEnabled: boolean;
  message: string | null;
}

export interface BulkRepairSummary {
  readyCount: number;
  errorCount: number;
  attentionCount: number;
  heading: string;
  message: string;
}

const STATE_VIEW: Record<AgentState, Pick<AgentStatusView, "label" | "tone">> = {
  unknown: { label: "Chua kiem tra", tone: "info" },
  missing: { label: "Chua cai Agent", tone: "warn" },
  repairRequired: { label: "Can sua Agent", tone: "warn" },
  starting: { label: "Dang khoi dong", tone: "info" },
  ready: { label: "San sang", tone: "ok" },
  error: { label: "Loi Agent", tone: "danger" },
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

export function summarizeBulkRepair(statuses: AgentStatus[]): BulkRepairSummary {
  const readyCount = statuses.filter((status) => status.state === "ready").length;
  const errorCount = statuses.filter((status) => status.state === "error").length;
  const attentionCount = statuses.length - readyCount;
  return {
    readyCount,
    errorCount,
    attentionCount,
    heading: `Agent: ${readyCount} san sang, ${attentionCount} can xu ly`,
    message: `${readyCount} may san sang; ${attentionCount} may can xu ly.`,
  };
}
