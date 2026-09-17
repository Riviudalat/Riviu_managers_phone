import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { HelpPage } from "./HelpPage";

it("hướng dẫn theo nhiệm vụ chỉ điều hướng, không thực thi thao tác", async () => {
  const onOpenPage = vi.fn();
  render(<HelpPage onOpenPage={onOpenPage} />);
  expect(screen.queryByRole("heading", { level: 1 })).toBeNull();
  expect(screen.getByRole("heading", { name: "Bạn cần làm gì?" })).toBeVisible();
  for (const [label, page] of [
    ["Kiểm tra thiết bị", "diagnostics"],
    ["Mở My Apps", "myApps"],
    ["Thiết lập Đăng bài", "publish"],
    ["Xem lượt chạy", "jobs"],
    ["Tham chiếu API", "api"],
  ]) {
    await userEvent.click(screen.getByRole("button", { name: label }));
    expect(onOpenPage).toHaveBeenLastCalledWith(page);
  }
  expect(onOpenPage).toHaveBeenCalledTimes(5);
});
