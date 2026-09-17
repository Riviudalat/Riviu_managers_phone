import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useAsyncList } from "./useAsyncList";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

describe("useAsyncList — vòng đời lần đọc", () => {
  it("lần đọc cũ không ghi đè dữ liệu hoặc kết thúc refresh đang chờ", async () => {
    const older = deferred<string[]>(), newer = deferred<string[]>();
    const read = vi.fn().mockResolvedValueOnce(["Đã tải"])
      .mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
    const { result } = renderHook(() => useAsyncList(read));
    await act(async () => {});
    expect(result.current.data).toEqual(["Đã tải"]);
    act(() => { void result.current.load(); });
    act(() => { void result.current.load(); });
    await act(async () => older.resolve(["Đã lỗi thời"]));
    expect(result.current.data).toEqual(["Đã tải"]);
    expect(result.current.refreshing).toBe(true);
    await act(async () => newer.resolve(["Mới nhất"]));
    expect(result.current.data).toEqual(["Mới nhất"]);
    expect(result.current.refreshing).toBe(false);
  });

  it("đổi nguồn không giữ hàng cũ hay cho callback cũ khởi động đọc lại", async () => {
    const first = vi.fn().mockResolvedValue(["Tài khoản"]);
    const pending = deferred<string[]>();
    const second = vi.fn().mockReturnValue(pending.promise);
    const { result, rerender } = renderHook(({ read }) => useAsyncList<string[]>(read), { initialProps: { read: first } });
    await act(async () => {});
    const oldLoad = result.current.load;
    rerender({ read: second });
    expect(result.current.data).toBeUndefined();
    expect(result.current.initialLoading).toBe(true);
    await act(async () => oldLoad());
    expect(first).toHaveBeenCalledTimes(1);
    expect(second).toHaveBeenCalledTimes(1);
    await act(async () => pending.resolve(["Kết nối"]));
    expect(result.current.data).toEqual(["Kết nối"]);
  });

  it.each(["resolve", "reject"] as const)("unmount vô hiệu hóa %s muộn và callback load đã giữ", async (settle) => {
    const pending = deferred<string[]>();
    const read = vi.fn().mockReturnValue(pending.promise);
    const { result, unmount } = renderHook(() => useAsyncList<string[]>(read));
    const snapshot = result.current;
    unmount();
    await act(async () => {
      if (settle === "resolve") pending.resolve(["Không còn view"]);
      else pending.reject(new Error("Lỗi sau unmount"));
      await snapshot.load();
    });
    expect(read).toHaveBeenCalledTimes(1);
    expect(result.current).toBe(snapshot);
  });
});
