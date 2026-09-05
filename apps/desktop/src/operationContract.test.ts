import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { operationGetRun, operationListRuns } from "./api";
import type { OperationRunKind, OperationRunState, PublishRetryScope } from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

beforeEach(() => {
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
});

describe("operations Rust/TypeScript contract", () => {
  it("pins every normalized source and state", () => {
    const kinds = {
      script: true,
      flow: true,
      orchestration: true,
      nurture: true,
      interaction: true,
      publish: true,
      appInstall: true,
      materialTransfer: true,
    } satisfies Record<OperationRunKind, true>;
    const states = {
      queued: true,
      running: true,
      succeeded: true,
      partial: true,
      failed: true,
      uncertain: true,
      cancelled: true,
      skipped: true,
    } satisfies Record<OperationRunState, true>;

    expect(Object.keys(kinds)).toHaveLength(8);
    expect(Object.keys(states)).toHaveLength(8);

    const retryScopes = {
      fullPipeline: true,
      linkAndSheet: true,
      sheetOnly: true,
      none: true,
    } satisfies Record<PublishRetryScope, true>;
    expect(Object.keys(retryScopes)).toHaveLength(4);
  });

  it("invokes the exact list and detail commands", async () => {
    await operationListRuns(25);
    await operationGetRun("flow:run-a");

    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["operation_list_runs", { limit: 25 }],
      ["operation_get_run", { operationId: "flow:run-a" }],
    ]);
  });
});
