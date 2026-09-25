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
      total={5}
      readyCount={4}
      groupMode={false}
      onPage={onPage}
    />,
  );
  return { onPage, onToggleCollapse };
}

describe("Sidebar information architecture", () => {
  beforeEach(() => localStorage.clear());
  it("does not present grid selection as the scope of an automation workspace", () => {
    render(<Sidebar page="publish" selectedCount={21} total={21} readyCount={20}
      groupMode={false} onPage={vi.fn()} />);
    expect(screen.queryByText("Đã chọn trong lưới")).toBeNull();
    expect(screen.getByText("20/21")).toBeVisible();
  });
  it("groups every workspace in the requested operator order", () => {
    renderSidebar();

    const navigation = screen.getByRole("navigation", { name: "Điều hướng chính" });
    expect(within(navigation).getAllByTestId("nav-item")).toHaveLength(16);
    expect(within(navigation).queryByRole("button", {name:"Dữ liệu"})).toBeNull();
    expect(within(navigation).queryByRole("button", {name:"Mạng & Router"})).toBeNull();
    expect(within(navigation).getByRole("button", { name: "My Apps" })).toBeVisible();
    expect(within(navigation).getByRole("button", { name: "Control Center" })).toBeVisible();
  });

  it("opens the app library and operation history", async () => {
    const { onPage } = renderSidebar();
    const user = userEvent.setup();

    await user.click(screen.getByRole("button", { name: "My Apps" }));
    await user.click(screen.getByRole("button", { name: "Control Center" }));

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
    expect(screen.getByRole("button", { name: "My Apps" })).toBeVisible();
    expect(screen.getByLabelText("Trạng thái hệ thống")).toHaveTextContent("4/5");
    await user.click(screen.getByRole("button", { name: "My Apps" }));
    expect(onPage).toHaveBeenCalledWith("myApps");
    await user.click(screen.getByRole("button", { name: "Mở rộng thanh điều hướng" }));
    expect(sidebar).toHaveAttribute("data-rail-collapsed", "false");
    expect(screen.getByText("Riviu Manager")).toBeVisible();
  });
  it("collapses Automation while preserving the three original destinations",async()=>{
    localStorage.clear();renderSidebar();const user=userEvent.setup();
    await user.click(screen.getByRole("button",{name:"Automation"}));
    expect(screen.queryByRole("button",{name:"Nuôi TikTok"})).toBeNull();
    await user.click(screen.getByRole("button",{name:"Automation"}));
    expect(screen.getByRole("button",{name:"Nuôi TikTok"})).toBeVisible();
    expect(screen.getByRole("button",{name:"Tương tác"})).toBeVisible();
    expect(screen.getByRole("button",{name:"Đăng bài"})).toBeVisible();
  });

  it("keeps fleet status outside scrolling navigation", () => {
    renderSidebar();
    const navigation = screen.getByRole("navigation", { name: "Điều hướng chính" });
    const status = screen.getByLabelText("Trạng thái hệ thống");
    expect(navigation).not.toContainElement(status);
    expect(status).toHaveTextContent("4/5");
    expect(status).toHaveTextContent("Đã chọn trong lưới");
  });

  it("marks a collapsed group containing the current page", async () => {
    localStorage.clear();
    render(<Sidebar page="publish" selectedCount={0} total={5} readyCount={4}
      groupMode={false} onPage={vi.fn()} />);
    const automation = screen.getByRole("button", { name: "Automation" });
    await userEvent.setup().click(automation);
    expect(automation).toHaveAttribute("aria-expanded", "false");
    expect(automation.closest(".menu-group")).toHaveAttribute("data-active", "true");
  });
});
