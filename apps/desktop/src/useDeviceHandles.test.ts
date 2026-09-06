import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { getDeviceMeta, saveDeviceHandle } from "./api";
import { useDeviceHandles } from "./useDeviceHandles";

vi.mock("./api", () => ({ getDeviceMeta: vi.fn(), saveDeviceHandle: vi.fn() }));
const meta = (handle: string) => ({ udid: "a", handle, alias: "", number: null, notes: "", tags: [], groupId: null });
const deferred = <T>() => { let resolve!: (value: T) => void; return { promise: new Promise<T>((r) => { resolve = r; }), resolve: (v: T) => resolve(v) }; };
beforeEach(() => { vi.resetAllMocks(); vi.mocked(getDeviceMeta).mockResolvedValue(meta("old.account")); });

describe("useDeviceHandles", () => {
  it("updates display and mention source together on scope return", async () => {
    const { result, rerender } = renderHook(({ ids }) => useDeviceHandles(ids), { initialProps: { ids: ["a"] } });
    await waitFor(() => expect(result.current.savedHandles.a).toBe("old.account"));
    rerender({ ids: [] });
    vi.mocked(getDeviceMeta).mockResolvedValue(meta("new.account"));
    rerender({ ids: ["a"] });
    await waitFor(() => expect(result.current.handles.a).toBe("new.account"));
    expect(result.current.savedHandles.a).toBe("new.account");
  });

  it("drops an older scope read after an explicit reload", async () => {
    const slow = deferred<ReturnType<typeof meta>>();
    vi.mocked(getDeviceMeta).mockReturnValueOnce(slow.promise);
    const { result } = renderHook(() => useDeviceHandles(["a"]));
    vi.mocked(getDeviceMeta).mockResolvedValue(meta("latest.account"));
    await act(() => result.current.reload("a"));
    await act(async () => slow.resolve(meta("stale.account")));
    expect(result.current.savedHandles.a).toBe("latest.account");
    expect(result.current.handles.a).toBe("latest.account");
  });

  it("preserves an edit across reload without using it as saved account", async () => {
    const { result, rerender } = renderHook(({ ids }) => useDeviceHandles(ids), { initialProps: { ids: ["a"] } });
    await waitFor(() => expect(result.current.savedHandles.a).toBe("old.account"));
    act(() => result.current.change("a", "draft.account"));
    rerender({ ids: [] });
    vi.mocked(getDeviceMeta).mockResolvedValue(meta("new.account"));
    rerender({ ids: ["a"] });
    await waitFor(() => expect(result.current.savedHandles.a).toBe("new.account"));
    expect(result.current.handles.a).toBe("draft.account");
  });

  it("keeps a completed save when an older fetch arrives", async () => {
    const slow = deferred<ReturnType<typeof meta>>();
    vi.mocked(getDeviceMeta).mockReturnValueOnce(slow.promise);
    vi.mocked(saveDeviceHandle).mockResolvedValue("saved.account");
    const { result } = renderHook(() => useDeviceHandles(["a"]));
    await act(() => result.current.persist("a", "saved.account"));
    await act(async () => slow.resolve(meta("old.account")));
    expect(result.current.savedHandles.a).toBe("saved.account");
    expect(result.current.handles.a).toBe("saved.account");
  });
});
