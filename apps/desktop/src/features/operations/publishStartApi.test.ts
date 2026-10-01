import { invoke } from "@tauri-apps/api/core";
import { afterEach, expect, it, vi } from "vitest";
import { publishStartStatus } from "../../api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => { vi.useRealTimers(); vi.resetAllMocks(); });

it("ends a stalled status read without dispatching or replacing the pending start", async () => {
  vi.useFakeTimers();
  let finish!: (value: null) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  let outcome: unknown = "pending";
  const read = publishStartStatus("same-request").then(
    value => { outcome = value; }, error => { outcome = error; },
  );
  await vi.advanceTimersByTimeAsync(15_000);
  expect(outcome).toBeInstanceOf(Error);
  expect((outcome as Error).message).toContain("Chưa đọc được trạng thái");
  finish(null);
  await read;
  expect(outcome).toBeInstanceOf(Error);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("publish_start_status", { requestId: "same-request" });
});
