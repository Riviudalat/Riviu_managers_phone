import { describe, expect, it } from "vitest";
import { agentStatusView, summarizeBulkRepair } from "./agentStatus";
import type { AgentState, AgentStatus } from "./types";

function status(state: AgentState, message: string | null = null): AgentStatus {
  return {
    udid: `fixture-${state}`,
    state,
    artifactId: "riviu-agent-fixture",
    artifactVersion: "1.2.3",
    bundleId: "com.riviu.fixture",
    protocolVersion: 1,
    features: ["text"],
    installedVersion: "1.2.3",
    installedBuild: "123",
    authReady: state === "ready",
    mjpegReady: state === "ready",
    sessionReady: state === "ready",
    message,
  };
}

describe("agentStatusView", () => {
  it("maps ready to Sẵn sàng and enables text comments", () => {
    const view = agentStatusView(status("ready"));

    expect(view.label).toBe("Sẵn sàng");
    expect(view.textCommentsEnabled).toBe(true);
  });

  it("maps repairRequired to Cần khôi phục Agent", () => {
    expect(agentStatusView(status("repairRequired")).label).toBe("Cần khôi phục Agent");
  });

  it("preserves a concise error message", () => {
    const message = "Agent authentication failed";

    expect(agentStatusView(status("error", message)).message).toBe(message);
  });
});

describe("summarizeBulkRepair", () => {
  it("counts ready and error devices without UDIDs in the toast heading", () => {
    const statuses = [
      status("ready"),
      status("ready"),
      status("error", "stream unavailable"),
    ];

    const summary = summarizeBulkRepair(statuses);

    expect(summary.readyCount).toBe(2);
    expect(summary.errorCount).toBe(1);
    expect(summary.attentionCount).toBe(1);
    expect(summary.heading).not.toContain("fixture-ready");
    expect(summary.heading).not.toContain("fixture-error");
  });
});
