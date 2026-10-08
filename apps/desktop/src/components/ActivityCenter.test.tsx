import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { pushToast, resetToasts } from "../toastStore";
import { ActivityCenter } from "./ActivityCenter";

afterEach(() => resetToasts());

describe("ActivityCenter", () => {
  it("announces one compact outcome with dismiss and persistent details", () => {
    render(<ActivityCenter />);

    act(() => {
      pushToast("error", "Không khởi động được", "Máy 3 đang bận");
    });

    expect(screen.getByRole("status")).toHaveTextContent("Không khởi động được");
    expect(screen.getByRole("status")).not.toHaveTextContent("Máy 3 đang bận");
    fireEvent.click(screen.getByRole("button", { name: "Chi tiết" }));
    expect(within(screen.getByRole("dialog")).getByText("Máy 3 đang bận")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Đóng trung tâm hoạt động" }));
    fireEvent.click(screen.getByRole("button", { name: "Đóng thông báo" }));
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getByRole("button", { name: "Hoạt động" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Hoạt động" }));
    expect(within(screen.getByRole("dialog")).getByText("Máy 3 đang bận")).toBeTruthy();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(document.querySelector(".activity-center-panel")).toBeNull();
  });

  it("opens persistent history, filters attention and clears it", () => {
    render(<ActivityCenter />);
    act(() => {
      pushToast("ok", "Đã lưu");
      pushToast("warn", "Máy chưa phản hồi", "Máy 2 cần kiểm tra");
    });

    fireEvent.click(screen.getByRole("button", { name: "Hoạt động, 2 mục chưa xem" }));
    const panel = screen.getByRole("dialog", { name: "Hoạt động" });
    expect(within(panel).getByText("Đã lưu")).toBeTruthy();
    expect(within(panel).getByText("Máy 2 cần kiểm tra")).toBeTruthy();

    fireEvent.click(within(panel).getByRole("button", { name: /Cần xử lý/ }));
    expect(within(panel).queryByText("Đã lưu")).toBeNull();
    expect(within(panel).getByText("Máy chưa phản hồi")).toBeTruthy();

    fireEvent.click(within(panel).getByRole("button", { name: "Xóa toàn bộ lịch sử hoạt động" }));
    expect(within(panel).getByText("Chưa có hoạt động")).toBeTruthy();
  });

  it("closes with Escape and returns focus to the trigger", () => {
    render(<ActivityCenter />);
    const trigger = screen.getByRole("button", { name: "Hoạt động" });
    fireEvent.click(trigger);
    expect(screen.getByRole("dialog", { name: "Hoạt động" })).toBeTruthy();

    fireEvent.keyDown(document, { key: "Escape" });

    expect(screen.queryByRole("dialog", { name: "Hoạt động" })).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });
});
