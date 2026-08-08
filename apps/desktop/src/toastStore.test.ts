import { renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  describeError,
  dismissToast,
  pushToast,
  resetToasts,
  toastError,
  useToasts,
} from "./toastStore";

/** The store is module-global, so every test starts from a clean queue. */
afterEach(() => {
  resetToasts();
  vi.useRealTimers();
});

/** Read the snapshot the ToastHost would render. */
function visibleToasts() {
  return renderHook(() => useToasts()).result.current;
}

describe("toastStore", () => {
  it("shows newest first and caps the stack so an error burst cannot fill the screen", () => {
    for (let index = 1; index <= 6; index += 1) {
      pushToast("error", `Lỗi ${index}`);
    }

    const visible = visibleToasts();
    expect(visible).toHaveLength(4);
    expect(visible.map((toast) => toast.title)).toEqual([
      "Lỗi 6",
      "Lỗi 5",
      "Lỗi 4",
      "Lỗi 3",
    ]);
  });

  it("auto-dismisses on the per-kind lifetime and can be closed early", () => {
    vi.useFakeTimers();
    pushToast("ok", "Đã lưu");
    const error = pushToast("error", "Hỏng");

    // `ok` lives 4s and `error` 9s, so only the error survives this tick.
    vi.advanceTimersByTime(5000);
    expect(visibleToasts().map((toast) => toast.id)).toEqual([error]);

    dismissToast(error);
    expect(visibleToasts()).toHaveLength(0);
  });

  it("normalises Error, Tauri CommandError and string throwables to one line", () => {
    expect(describeError(new Error("boom"))).toBe("boom");
    expect(describeError("plain")).toBe("plain");
    expect(describeError({ code: "DeviceBusy", message: "máy đang bận" })).toBe(
      "DeviceBusy: máy đang bận",
    );
    expect(describeError({ error: "không kết nối được" })).toBe("không kết nối được");
    expect(describeError(null)).toBe("Lỗi không rõ nguyên nhân");

    toastError("Backup thất bại", new Error("hết dung lượng"));
    expect(visibleToasts()[0]).toMatchObject({
      kind: "error",
      title: "Backup thất bại",
      detail: "hết dung lượng",
    });
  });
});
