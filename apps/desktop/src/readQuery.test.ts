import { invoke } from "@tauri-apps/api/core";
import { expect, it, vi } from "vitest";
import { listDeviceMetas, operationDeviceLog, operationPrepareDevices, operationQueryRuns, operationStop, patchDeviceMeta, saveDeviceHandle, saveDeviceMeta } from "./api";
import { readQueryClient } from "./readQuery";

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

it("caches device metadata briefly and invalidates it after its owning mutation", async () => {
  readQueryClient.clear();
  vi.mocked(invoke).mockReset().mockImplementation(async (command) => {
    if (command === "list_device_metas") return [];
    return undefined;
  });
  await listDeviceMetas();
  await listDeviceMetas();
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "list_device_metas")).toHaveLength(1);

  await saveDeviceMeta({ udid: "phone", alias: "", number: null, notes: "", tags: [], groupId: null, handle: "" });
  await listDeviceMetas();
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "list_device_metas")).toHaveLength(2);

  await saveDeviceHandle("phone", "", "new.handle");
  await listDeviceMetas();
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "list_device_metas")).toHaveLength(3);

  await patchDeviceMeta("phone", { field: "alias", value: "Máy mới" });
  await listDeviceMetas();
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "list_device_metas")).toHaveLength(4);
});

it("refuses to start a new operation with a partially released device set", async () => {
  vi.mocked(invoke).mockReset().mockResolvedValue({
    operationId: "handoff:fixture",
    state: "needsAttention",
    devices: [
      { udid: "ready", closed: true, message: "Đã nhả" },
      { udid: "held", closed: false, message: "Tác vụ cũ chưa nhả thiết bị" },
    ],
    stopMarker: null,
  });
  await expect(operationPrepareDevices(["ready", "held"])).rejects.toThrow(
    "held: Tác vụ cũ chưa nhả thiết bị",
  );
  expect(invoke).toHaveBeenCalledTimes(1);
});
