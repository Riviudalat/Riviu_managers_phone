import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  orchestrationCancelRun,
  orchestrationGetRun,
  orchestrationListRuns,
  orchestrationReconcile,
  orchestrationRun,
} from "./api";
import type {
  OrchestrationAttemptState,
  OrchestrationBranch,
  OrchestrationNodeAction,
  OrchestrationRunState,
  TargetRef,
} from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const NODE_KINDS = {
  start: true,
  delay: true,
  runNurture: true,
  runInteraction: true,
  runPublish: true,
  end: true,
} satisfies Record<OrchestrationNodeAction["kind"], true>;

const BRANCHES = {
  done: true,
  partial: true,
  failed: true,
  uncertain: true,
} satisfies Record<OrchestrationBranch, true>;

const RUN_STATES = {
  queued: true,
  running: true,
  done: true,
  partial: true,
  failed: true,
  uncertain: true,
  cancelled: true,
} satisfies Record<OrchestrationRunState, true>;

const ATTEMPT_STATES = {
  queued: true,
  dispatching: true,
  waitingChild: true,
  done: true,
  partial: true,
  failed: true,
  uncertain: true,
  cancelled: true,
} satisfies Record<OrchestrationAttemptState, true>;

beforeEach(() => {
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
});

describe("orchestration Rust/TypeScript contract", () => {
  it("mirrors every node, outcome and durable state", () => {
    expect(Object.keys(NODE_KINDS)).toEqual([
      "start",
      "delay",
      "runNurture",
      "runInteraction",
      "runPublish",
      "end",
    ]);
    expect(Object.keys(BRANCHES)).toEqual(["done", "partial", "failed", "uncertain"]);
    expect(Object.keys(RUN_STATES)).toEqual([
      "queued",
      "running",
      "done",
      "partial",
      "failed",
      "uncertain",
      "cancelled",
    ]);
    expect(Object.keys(ATTEMPT_STATES)).toEqual([
      "queued",
      "dispatching",
      "waitingChild",
      "done",
      "partial",
      "failed",
      "uncertain",
      "cancelled",
    ]);
  });

  it("uses exact Tauri command names and camelCase run arguments", async () => {
    const target: TargetRef = { type: "group", groupId: "group-a" };
    await orchestrationRun("document-a", 4, target);
    await orchestrationListRuns(25);
    await orchestrationListRuns();
    await orchestrationGetRun("run-a");
    await orchestrationReconcile("run-a");
    await orchestrationCancelRun("run-a");

    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["orchestration_run", { documentId: "document-a", revision: 4, target }],
      ["orchestration_list_runs", { limit: 25 }],
      ["orchestration_list_runs", { limit: null }],
      ["orchestration_get_run", { runId: "run-a" }],
      ["orchestration_reconcile", { runId: "run-a" }],
      ["orchestration_cancel_run", { runId: "run-a" }],
    ]);
  });
});
