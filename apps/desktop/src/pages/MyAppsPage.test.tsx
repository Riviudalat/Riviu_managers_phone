import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { MyAppsPage } from "./MyAppsPage";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
beforeEach(() => invoke.mockReset());

it("chỉ báo quy trình không khớp khi ứng dụng có sẵn vẫn khớp tìm kiếm", async () => {
  invoke.mockResolvedValue([{ id: "custom", kind: "publish", name: "Quy trình đăng", latestRevision: 1, updatedAt: "2026-09-17T08:00:00Z" }]);
  render(<MyAppsPage devices={[]} onOpenApp={() => undefined} />);
  await screen.findByRole("button", { name: "Quy trình đăng" });
  await userEvent.type(screen.getByRole("searchbox", { name: "Tìm ứng dụng" }), "Nuôi");
  expect(screen.getByRole("button", { name: "Nuôi TikTok" })).toBeInTheDocument();
  expect(screen.getByText("Không có quy trình đã lưu khớp tìm kiếm")).toBeInTheDocument();
  expect(screen.queryByText("Không có ứng dụng khớp tìm kiếm")).not.toBeInTheDocument();
});

it("reject muộn từ màn hình đã rời không gắn lỗi vào lần mở trang mới", async () => {
  let rejectOld!: (cause: unknown) => void;
  invoke.mockReturnValueOnce(new Promise((_, reject) => { rejectOld = reject; })).mockResolvedValueOnce([]);
  const old = render(<MyAppsPage devices={[]} onOpenApp={() => undefined} />);
  old.unmount();
  render(<MyAppsPage devices={[]} onOpenApp={() => undefined} />);
  await screen.findByText("Chưa có quy trình đã lưu");
  await act(async () => rejectOld(new Error("Lỗi trang đã rời")));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByText("Chưa có quy trình đã lưu")).toBeInTheDocument();
});
