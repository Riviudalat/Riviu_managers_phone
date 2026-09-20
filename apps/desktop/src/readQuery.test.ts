import { invoke } from "@tauri-apps/api/core";
import { expect, it, vi } from "vitest";
import { operationDeviceLog, operationQueryRuns, operationStop } from "./api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

it("coalesces identical persisted reads while separating run/device/query scopes", async () => {
  const release: Array<(value: unknown) => void> = [];
  vi.mocked(invoke).mockImplementation(() => new Promise(resolve => release.push(resolve)));
  const reads = [
    operationQueryRuns({ kind: "publish", limit: 10 }),
    operationQueryRuns({ kind: "publish", limit: 10 }),
    operationQueryRuns({ kind: "interaction", limit: 10 }),
    operationDeviceLog("publish:a", "phone-a"),
    operationDeviceLog("publish:a", "phone-a"),
    operationDeviceLog("publish:a", "phone-b"),
  ];
  expect(invoke).toHaveBeenCalledTimes(4);
  release.forEach(resolve => resolve({ entries: [], runs: [], total: 0 }));
  await Promise.all(reads);
});

it("never coalesces or retries effect commands", async () => {
  vi.mocked(invoke).mockReset().mockRejectedValue(new Error("lost acknowledgement"));
  await Promise.allSettled([operationStop("publish:a"), operationStop("publish:a")]);
  expect(invoke).toHaveBeenCalledTimes(2);
});
