import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { Sidebar } from "./Sidebar";

function renderSidebar() {
  const onPage = vi.fn();
  const onToggleCollapse = vi.fn();
  render(
    <Sidebar
      page="control"
      selectedCount={2}
      groupMode={false}
      onPage={onPage}
    />,
  );
  return { onPage, onToggleCollapse };
}

describe("Sidebar information architecture", () => {
  beforeEach(() => localStorage.clear());
  it("does not present grid selection as the scope of an automation workspace", () => {
    render(<Sidebar page="publish" selectedCount={21}
      groupMode={false} onPage={vi.fn()} />);
    expect(screen.queryByText("Đã chọn trong lưới")).toBeNull();
    expect(screen.getByLabelText("Trạng thái hệ thống")).toHaveTextContent("Đồng bộ");
  });
  it("groups every workspace in the requested operator order", () => {
    renderSidebar();

    const navigation = screen.getByRole("navigation", { name: "Điều hướng chính" });
    // Each page has one place: the six pinned shortcuts are not repeated in their groups.
    expect(within(navigation).getAllByTestId("nav-item")).toHaveLength(16);
    expect(within(navigation).queryByRole("button", {name:"Dữ liệu"})).toBeNull();
    expect(within(navigation).queryByRole("button", {name:"Mạng & Router"})).toBeNull();
    const pinned = within(navigation).getByRole("group", { name: "Lối tắt" });
    expect(within(pinned).getAllByRole("button").map((button) => button.getAttribute("aria-label")))
      .toEqual(["Trung tâm điều khiển", "Nuôi TikTok", "Tương tác", "Đăng bài", "Lượt chạy", "Trợ giúp"]);
    expect(within(pinned).getByRole("button", { name: "Trung tâm điều khiển" })).toHaveTextContent("Trung tâm");
    expect(within(navigation).getAllByRole("region").map((group) => group.getAttribute("aria-label")))
      .toEqual(["Dàn máy", "Công việc", "Nội dung", "Hệ thống"]);
    expect(within(navigation).getByRole("button", { name: "Ứng dụng" })).toBeVisible();
  });

  it("opens the app library and operation history", async () => {
    const { onPage } = renderSidebar();
    const user = userEvent.setup();

    await user.click(screen.getByRole("button", { name: "Ứng dụng" }));
    await user.click(screen.getByRole("button", { name: "Trung tâm điều khiển" }));

    expect(onPage).toHaveBeenNthCalledWith(1, "myApps");
    expect(onPage).toHaveBeenNthCalledWith(2, "control");
  });

  it("collapses to an icon rail without losing routes or fleet status", async () => {
    const { onPage } = renderSidebar();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Thu gọn thanh điều hướng" }));
    const sidebar = screen.getByLabelText("Riviu Manager");
    expect(sidebar).toHaveAttribute("data-rail-collapsed", "true");
    expect(localStorage.getItem("riviu.sidebar.rail")).toBe("true");
    expect(screen.getByRole("button", { name: "Ứng dụng" })).toBeVisible();
    // Never an icon alone: the rail keeps a short name under each icon.
    expect(screen.getByRole("button", { name: "Quản lý tài khoản" })).toHaveTextContent("Tài khoản");
    expect(screen.getByLabelText("Trạng thái hệ thống")).toHaveTextContent("Đồng bộ tắt");
    await user.click(screen.getByRole("button", { name: "Ứng dụng" }));
    expect(onPage).toHaveBeenCalledWith("myApps");
    await user.click(screen.getByRole("button", { name: "Mở rộng thanh điều hướng" }));
    expect(sidebar).toHaveAttribute("data-rail-collapsed", "false");
    expect(screen.getByText("Riviu Manager")).toBeVisible();
  });
  it("collapses Công việc while preserving the three original destinations",async()=>{
    localStorage.clear();renderSidebar();const user=userEvent.setup();
    await user.click(screen.getByRole("button",{name:"Công việc"}));
    expect(screen.queryByRole("button",{name:"Lịch chạy"})).toBeNull();
    expect(screen.getByRole("button",{name:"Nuôi TikTok"})).toBeVisible();
    await user.click(screen.getByRole("button",{name:"Công việc"}));
    expect(screen.getByRole("button",{name:"Lịch chạy"})).toBeVisible();
    expect(screen.getByRole("button",{name:"Nuôi TikTok"})).toBeVisible();
    expect(screen.getByRole("button",{name:"Tương tác"})).toBeVisible();
    expect(screen.getByRole("button",{name:"Đăng bài"})).toBeVisible();
  });

  it("keeps fleet status outside scrolling navigation", () => {
    renderSidebar();
    const navigation = screen.getByRole("navigation", { name: "Điều hướng chính" });
    const status = screen.getByLabelText("Trạng thái hệ thống");
    expect(navigation).not.toContainElement(status);
    expect(status).toHaveTextContent("Đã chọn trong lưới");
    // The fleet's readiness is counted once, by the header counters; no second "Kết nối" block.
    expect(status).not.toHaveTextContent("Kết nối");
    expect(status).not.toHaveTextContent("Sẵn sàng");
  });

  it("marks a collapsed group containing the current page", async () => {
    localStorage.clear();
    render(<Sidebar page="schedules" selectedCount={0}
      groupMode={false} onPage={vi.fn()} />);
    const automation = screen.getByRole("button", { name: "Công việc" });
    await userEvent.setup().click(automation);
    expect(automation).toHaveAttribute("aria-expanded", "false");
    expect(automation.closest(".menu-group")).toHaveAttribute("data-active", "true");
  });
});
