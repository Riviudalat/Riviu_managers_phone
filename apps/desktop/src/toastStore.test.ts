import { renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  clearToasts,
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

/** Read the snapshot the activity center would render. */
function activityHistory() {
  return renderHook(() => useToasts()).result.current;
}

describe("activity history store", () => {
  it("keeps the newest 100 outcomes instead of dropping a device-error burst", () => {
    for (let index = 1; index <= 105; index += 1) {
      pushToast("error", `Lỗi ${index}`);
    }

    const history = activityHistory();
    expect(history).toHaveLength(100);
    expect(history[0].title).toBe("Lỗi 105");
    expect(history[99].title).toBe("Lỗi 6");
  });

  it("does not disappear on a timer and supports dismissing or clearing explicitly", () => {
    vi.useFakeTimers();
    pushToast("ok", "Đã lưu");
    const error = pushToast("error", "Hỏng");

    vi.advanceTimersByTime(60_000);
    expect(activityHistory()).toHaveLength(2);

    dismissToast(error);
    expect(activityHistory().map((activity) => activity.title)).toEqual(["Đã lưu"]);

    clearToasts();
    expect(activityHistory()).toHaveLength(0);
  });

  it("records when the outcome happened", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-09-05T03:04:05.000Z"));
    pushToast("info", "Đang chuẩn bị");

    expect(activityHistory()[0].createdAt).toBe(1_788_577_445_000);
  });

  it("puts a normalised error in the toast's detail line", () => {
    // `describeError` itself is proved in describeError.test.ts; what matters here is that a
    // toast passes the throwable through it rather than interpolating it raw.
    toastError("Backup thất bại", new Error("hết dung lượng"));
    expect(activityHistory()[0]).toMatchObject({
      kind: "error",
      title: "Backup thất bại",
      detail: "hết dung lượng",
    });

    toastError("Không mở được thư mục", { code: "Io", message: "Permission denied" });
    expect(activityHistory()[0].detail).toBe("Io: Permission denied");
  });
});
