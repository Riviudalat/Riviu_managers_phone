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
  const outcome = (status: AgentStatus, agentRepaired: boolean, helperRecoveryRequired = false) =>
    ({ status, agentRepaired, helperRecoveryRequired });

  it("counts repaired and failed devices without UDIDs in the toast heading", () => {
    const summary = summarizeBulkRepair([
      outcome(status("ready"), true),
      outcome(status("ready"), true),
      outcome(status("error", "stream unavailable"), false),
    ]);

    expect(summary.repairedCount).toBe(2);
    expect(summary.failedCount).toBe(1);
    expect(summary.helperPendingCount).toBe(0);
    expect(summary.attentionCount).toBe(1);
    expect(summary.heading).toBe("Sửa agent: 2 máy đã sửa, 1 máy lỗi");
    expect(summary.heading).not.toContain("fixture-ready");
    expect(summary.heading).not.toContain("fixture-error");
  });

  it("reports a repaired agent separately from a helper that still needs recovery", () => {
    // The latched helper failure overrides the cached status; the repair still succeeded.
    const latched = status("repairRequired", "Riviu Helper: runtime qualification failed; recovery required (no action replay)");
    const summary = summarizeBulkRepair([outcome(latched, true, true)]);

    expect(summary.repairedCount).toBe(1);
    expect(summary.failedCount).toBe(0);
    expect(summary.helperPendingCount).toBe(1);
    expect(summary.attentionCount).toBe(1);
    expect(summary.heading).toBe("Sửa agent: 1 máy đã sửa, 0 máy lỗi");
    expect(summary.message).toContain("Khôi phục helper");
  });
});
