import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { publishDeviceGuards } from "../../api";
import { usePublishDeviceGuards } from "./usePublishDeviceGuards";
import type { PublishDeviceGuards } from "../../types";
vi.mock("../../api", () => ({ publishDeviceGuards: vi.fn() }));
const empty = (ids: string[]) => Object.fromEntries(ids.map(id => [id, { blocking: [], linkReview: [] }]));
beforeEach(() => { vi.mocked(publishDeviceGuards).mockReset().mockImplementation(async ids => empty(ids)); });
afterEach(() => { cleanup(); vi.useRealTimers(); });
it("starts unknown, coalesces repeated refreshes and ignores a stale roster response", async () => {
  let resolve!: (value: PublishDeviceGuards) => void;
  vi.mocked(publishDeviceGuards).mockImplementationOnce(() => new Promise(yes => { resolve = yes; }));
  const view = renderHook(({ ids }) => usePublishDeviceGuards(ids), { initialProps: { ids: ["a"] } });
  expect(view.result.current.guards).toEqual({});
  view.rerender({ ids: ["b"] });
  await act(async () => { await view.result.current.refresh(); await view.result.current.refresh(); });
  expect(publishDeviceGuards).toHaveBeenCalledTimes(1);
  await act(async () => resolve({ a: { blocking: [{ assignmentId: "old", campaignId: "old", updatedAt: "then", reason: "old" }], linkReview: [] } }));
  await waitFor(() => expect(view.result.current.guards).toEqual(empty(["b"])));
  expect(publishDeviceGuards).toHaveBeenCalledTimes(2);
});
it("fails closed on incomplete or failed reads and recovers on the next poll", async () => {
  vi.useFakeTimers();
  vi.mocked(publishDeviceGuards).mockResolvedValueOnce({}).mockRejectedValueOnce(new Error("offline"));
  const view = renderHook(() => usePublishDeviceGuards(["a"]));
  await act(async () => {}); expect(view.result.current.failed).toBe(true);
  await act(async () => vi.advanceTimersByTimeAsync(5000)); expect(view.result.current.guards).toEqual({});
  await act(async () => vi.advanceTimersByTimeAsync(5000)); expect(view.result.current.guards).toEqual(empty(["a"])); expect(view.result.current.failed).toBe(false);
  view.unmount(); const calls = vi.mocked(publishDeviceGuards).mock.calls.length;
  await act(async () => vi.advanceTimersByTimeAsync(20000)); expect(publishDeviceGuards).toHaveBeenCalledTimes(calls);
});
it("splits more than 500 IDs into sequential bounded requests", async () => {
  const ids = Array.from({ length: 501 }, (_, i) => String(i));
  const view = renderHook(() => usePublishDeviceGuards(ids));
  await waitFor(() => expect(Object.keys(view.result.current.guards)).toHaveLength(501));
  expect(vi.mocked(publishDeviceGuards).mock.calls.map(([batch]) => batch.length)).toEqual([500, 1]);
});
