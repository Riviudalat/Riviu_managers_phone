import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { pushMaterialBatch } from "./api";
import type {
  MaterialPushDeviceResult,
  MaterialPushStatus,
} from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

beforeEach(() => {
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
});

describe("material batch Rust/TypeScript contract", () => {
  it("keeps terminal states exhaustive and evidence/error typed", () => {
    const states = {
      succeeded: true,
      failed: true,
      uncertain: true,
      cancelledBeforeDispatch: true,
    } satisfies Record<MaterialPushStatus, true>;
    const result: MaterialPushDeviceResult = {
      udid: "phone-1",
      status: "succeeded",
      evidence: "sha256=ok",
    };

    expect(Object.keys(states)).toEqual(["succeeded", "failed", "uncertain", "cancelledBeforeDispatch"]);
    expect(result.error).toBeUndefined();
  });

  it("uses the exact Tauri command and preserves the semantic target", async () => {
    const request = {
      materialId: "material-a",
      target: { type: "group" as const, groupId: "group-a" },
    };

    await pushMaterialBatch(request);

    expect(invoke).toHaveBeenCalledWith("push_material_batch", { request });
  });
});
