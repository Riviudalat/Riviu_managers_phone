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

export function summarizeBulkRepair(statuses: AgentStatus[]): BulkRepairSummary {
  const readyCount = statuses.filter((status) => status.state === "ready").length;
  const errorCount = statuses.filter((status) => status.state === "error").length;
  const attentionCount = statuses.length - readyCount;
  return {
    readyCount,
    errorCount,
    attentionCount,
    heading: `Agent: ${readyCount} sẵn sàng, ${attentionCount} cần xử lý`,
    message: `${readyCount} máy sẵn sàng; ${attentionCount} máy cần xử lý.`,
  };
}
