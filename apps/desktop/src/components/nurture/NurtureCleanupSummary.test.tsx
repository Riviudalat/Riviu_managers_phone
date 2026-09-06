import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import type { NurtureSessionStatus } from "../../types";
import { NurtureRunProgress } from "./NurtureProgress";

afterEach(cleanup);
it("counts only proof-backed cleanup from this run, not merely finished machines", () => {
  const base = { runId: "current", runSize: 3, phase: "finished", running: false, outcome: "done", videoTarget: 1, videosDone: 1, startedAt: "2026-09-07T00:00:00Z", updatedAt: "2026-09-07T00:01:00Z" };
  render(<NurtureRunProgress now={Date.now()} statuses={[
    { ...base, udid: "a", cleanupState: "processAbsent", cleanupProof: { bundleId: "com.fixture", oldPid: 12 } },
    { ...base, udid: "b", cleanupState: "processAbsent", cleanupProof: null },
    { ...base, udid: "c", cleanupState: "failed", cleanupProof: null },
    { ...base, udid: "old", runId: "old", runSize: 1, updatedAt: "2026-09-06T00:00:00Z", startedAt: "2026-09-06T00:00:00Z", cleanupState: "processAbsent", cleanupProof: { bundleId: "com.fixture", oldPid: 13 } },
  ] as NurtureSessionStatus[]} />);
  expect(screen.getByText("TikTok đã tắt: 1/3 máy trong phiên")).toBeVisible();
});
