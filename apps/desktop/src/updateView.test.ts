import { describe, expect, it } from "vitest";
import { updateView } from "./updateView";
import type { UpdateStatus } from "./types";

function status(overrides: Partial<UpdateStatus> = {}): UpdateStatus {
  return {
    available: true,
    version: "0.1.1",
    current: "0.1.0",
    busyReason: null,
    ...overrides,
  };
}

describe("updateView", () => {
  it("offers the install only when a version exists and nothing is running", () => {
    const view = updateView(status(), null, false);

    expect(view.canInstall).toBe(true);
    expect(view.headline).toContain("0.1.1");
  });

  it("refuses the install while work is running, and says what that work is", () => {
    const view = updateView(
      status({ busyReason: "2 phiên Nuôi TT đang chạy — dừng hết trước khi cập nhật" }),
      null,
      false,
    );

    expect(view.canInstall).toBe(false);
    // The sentence itself is the point: a bare "busy" sends the operator hunting.
    expect(view.detail).toContain("Nuôi TT");
    expect(view.tone).toBe("warn");
  });

  it("never offers the install when the fleet state is unreadable", () => {
    // busy_reason fails closed on an unreadable job queue, and that string arrives here
    // as an ordinary reason. It must disable the button exactly like a real session does.
    const view = updateView(
      status({ busyReason: "không đọc được hàng đợi việc (database is locked)" }),
      null,
      false,
    );

    expect(view.canInstall).toBe(false);
  });

  it("treats not having checked as the resting state, not as a problem", () => {
    const view = updateView(null, null, false);

    expect(view.tone).toBe("info");
    expect(view.canInstall).toBe(false);
    expect(view.detail).toBe('Bấm "Kiểm bản mới" để kiểm tra theo yêu cầu.');
  });

  it("reports being current with the running version named", () => {
    const view = updateView(status({ available: false, version: null }), null, false);

    expect(view.tone).toBe("ok");
    expect(view.headline).toContain("0.1.0");
    expect(view.canInstall).toBe(false);
  });

  it("shows a failed check as a warning rather than silence", () => {
    const view = updateView(null, "không kiểm được bản mới: dns error", false);

    expect(view.tone).toBe("warn");
    expect(view.detail).toContain("dns error");
    expect(view.canInstall).toBe(false);
  });

  it("closes the button while installing, so a second press cannot start a second install", () => {
    const view = updateView(status(), null, true);

    expect(view.canInstall).toBe(false);
    expect(view.headline).toContain("Đang tải");
  });

  it("survives an available update whose version the backend did not name", () => {
    const view = updateView(status({ version: null }), null, false);

    expect(view.canInstall).toBe(true);
    expect(view.headline).toContain("?");
  });
});
