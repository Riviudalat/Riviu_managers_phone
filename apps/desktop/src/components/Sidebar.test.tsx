import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Sidebar } from "./Sidebar";

function renderSidebar(collapsed = false) {
  const onPage = vi.fn();
  const onToggleCollapse = vi.fn();
  render(
    <Sidebar
      page="control"
      collapsed={collapsed}
      selectedCount={2}
      total={5}
      readyCount={4}
      groupMode={false}
      onPage={onPage}
      onToggleCollapse={onToggleCollapse}
    />,
  );
  return { onPage, onToggleCollapse };
}

describe("Sidebar information architecture", () => {
  it("does not present grid selection as the scope of an automation workspace", () => {
    render(<Sidebar page="publish" collapsed={false} selectedCount={21} total={21} readyCount={20}
      groupMode={false} onPage={vi.fn()} onToggleCollapse={vi.fn()} />);
    expect(screen.queryByText("Đã chọn trong lưới")).toBeNull();
    expect(screen.getByText("20/21")).toBeVisible();
  });
  it("groups every workspace in the requested operator order", () => {
    renderSidebar();

    const navigation = screen.getByRole("navigation", { name: "Điều hướng chính" });
    expect(within(navigation).getAllByRole("heading", { level: 2 }).map((item) => item.textContent))
      .toEqual(["Thiết bị", "Tự động hóa", "Tài nguyên", "Hệ thống"]);
    expect(within(navigation).getAllByTestId("nav-item").map((item) => item.textContent?.trim()))
      .toEqual([
        "Thiết bị",
        "Chẩn đoán",
        "Nuôi TikTok",
        "Tương tác",
        "Đăng bài",
        "Flow",
        "Tác vụ",
        "Kho nội dung",
        "Trung tâm ứng dụng",
        "Dữ liệu",
        "API",
        "Cài đặt",
      ]);
  });

  it("opens the dedicated nurture and interaction workspaces", async () => {
    const { onPage } = renderSidebar();
    const user = userEvent.setup();

    await user.click(screen.getByRole("button", { name: "Nuôi TikTok" }));
    await user.click(screen.getByRole("button", { name: "Tương tác" }));

    expect(onPage).toHaveBeenNthCalledWith(1, "nurture");
    expect(onPage).toHaveBeenNthCalledWith(2, "interaction");
  });

  it("marks only the current page and exposes collapse state", () => {
    renderSidebar();

    expect(screen.getByRole("button", { name: "Thiết bị" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.getByRole("button", { name: "Đăng bài" })).not.toHaveAttribute("aria-current");
    expect(screen.getByRole("button", { name: "Thu gọn thanh điều hướng" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(screen.getByRole("button", { name: "Thu gọn thanh điều hướng" })).toHaveAttribute(
      "aria-controls",
      "primary-navigation",
    );
  });

  it("keeps icon-only navigation and collapse controls named", async () => {
    const { onToggleCollapse } = renderSidebar(true);
    const user = userEvent.setup();

    expect(screen.getByRole("button", { name: "Đăng bài" })).toBeVisible();
    const expand = screen.getByRole("button", { name: "Mở rộng thanh điều hướng" });
    await user.click(expand);
    expect(onToggleCollapse).toHaveBeenCalledTimes(1);
  });
});
