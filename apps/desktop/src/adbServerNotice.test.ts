import { describe, expect, it } from "vitest";

import { adbServerToast } from "./adbServerNotice";

describe("adbServerNotice", () => {
  it("warns with the phone count when a populated server is lost", () => {
    const toast = adbServerToast({
      type: "adbServerNotice",
      port: 5037,
      change: "lost",
      transports: 30,
      message: "Phần mềm khác vừa khởi động lại hoặc tắt ADB (cổng 5037)",
    });
    expect(toast.kind).toBe("warn");
    expect(toast.title).toBe("ADB bị phần mềm khác khởi động lại · 30 máy");
    expect(toast.detail).toContain("cổng 5037");
  });

  it("records the return as information, not as a second warning", () => {
    const toast = adbServerToast({ type: "adbServerNotice", port: 5037, change: "returned", transports: 30, message: "ok" });
    expect(toast).toEqual({ kind: "info", title: "ADB đã chạy lại (cổng 5037)", detail: "ok" });
  });
});
