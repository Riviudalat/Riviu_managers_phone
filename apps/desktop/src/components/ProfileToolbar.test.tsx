import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ProfileToolbar } from "./ProfileToolbar";

function setup() {
  const onInstall = vi.fn();
  const onRefresh = vi.fn();
  render(<ProfileToolbar selected={[]} deviceCount={3} onStart={vi.fn()} onStop={vi.fn()}
    onInstall={onInstall} onSync={vi.fn()} onRefresh={onRefresh} onGroupTools={vi.fn()}
    onGroups={vi.fn()} groupsOpen={false} groupToolsOpen={false} syncOn={false} />);
  return { onInstall, onRefresh };
}

describe("device toolbar", () => {
  it("keeps repair behind the maintenance disclosure and dispatches only the explicit action", async () => {
    const { onInstall } = setup();
    expect(screen.getByRole("button", { name: "Sửa Riviu Agent" })).not.toBeVisible();
    await userEvent.click(screen.getByText("Bảo trì", { selector: "summary" }));
    expect(onInstall).not.toHaveBeenCalled();
    expect(screen.getByText("Các máy đang kết nối")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Sửa Riviu Agent" }));
    expect(onInstall).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "Sửa Riviu Agent" })).not.toBeVisible();
  });

  it("closes maintenance with Escape and returns focus without repairing", async () => {
    const { onInstall, onRefresh } = setup();
    const menu = screen.getByText("Bảo trì", { selector: "summary" });
    await userEvent.click(menu);
    screen.getByRole("button", { name: "Sửa Riviu Agent" }).focus();
    await userEvent.keyboard("{Escape}");
    expect(menu).toHaveFocus();
    expect(onInstall).not.toHaveBeenCalled();
    await userEvent.click(screen.getByTitle("Quét lại thiết bị"));
    expect(onRefresh).toHaveBeenCalledOnce();
  });
});
