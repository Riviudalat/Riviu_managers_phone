import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

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

  it("always shows text navigation with no collapse button", () => {
    renderSidebar();
    expect(screen.queryByRole("button", { name: /Thu g.n thanh|M. r.ng thanh/ })).toBeNull();
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
});
