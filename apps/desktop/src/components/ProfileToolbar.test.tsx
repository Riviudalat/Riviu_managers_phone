import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { DeviceInfo } from "../types";
import { ProfileToolbar } from "./ProfileToolbar";

afterEach(cleanup);

const devices = [
  { udid: "a", name: "Same model", status: "ready" },
  { udid: "b", name: "Same model", status: "ready" },
] as DeviceInfo[];

function baseProps() {
  return {
    selected: devices,
    deviceCount: 3,
    onStart: vi.fn(),
    onInstall: vi.fn(),
    onRefresh: vi.fn(),
    onGroupTools: vi.fn(),
    onGroups: vi.fn(),
    groupsOpen: false,
    groupToolsOpen: false,
  };
}

describe("device toolbar", () => {
  it("keeps repair behind the maintenance disclosure and dispatches only the explicit action", async () => {
    const props = baseProps();
    render(<ProfileToolbar {...props} selected={[]} />);
    expect(screen.getByRole("button", { name: "Sửa Riviu Agent" })).not.toBeVisible();
    await userEvent.click(screen.getByText("Bảo trì").closest("summary")!);
    expect(props.onInstall).not.toHaveBeenCalled();
    expect(screen.getByText("Các máy đang kết nối")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Sửa Riviu Agent" }));
    expect(props.onInstall).toHaveBeenCalledOnce();
  });

  it("closes maintenance with Escape and keeps refresh separate", async () => {
    const props = baseProps();
    render(<ProfileToolbar {...props} selected={[]} />);
    const menu = screen.getByText("Bảo trì").closest("summary")!;
    await userEvent.click(menu);
    screen.getByRole("button", { name: "Sửa Riviu Agent" }).focus();
    await userEvent.keyboard("{Escape}");
    expect(menu).toHaveFocus();
    expect(props.onInstall).not.toHaveBeenCalled();
    await userEvent.click(screen.getByTitle("Quét lại thiết bị"));
    expect(props.onRefresh).toHaveBeenCalledOnce();
  });
});
