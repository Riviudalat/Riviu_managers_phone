import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { defaultGroupSync, getGroupSync, setGroupSync } from "../groupSync";
import type { DeviceInfo } from "../types";
import { ProfileToolbar } from "./ProfileToolbar";

afterEach(() => {
  cleanup();
  setGroupSync(defaultGroupSync());
});

const devices = [
  { udid: "a", name: "Same model", status: "ready" },
  { udid: "b", name: "Same model", status: "ready" },
] as DeviceInfo[];

function baseProps() {
  return {
    selected: devices,
    deviceCount: 3,
    activeSync: null,
    readiness: null,
    resolvedMasterUdid: "a",
    onMasterChange: vi.fn(),
    onEnableSync: vi.fn(),
    onDisableSync: vi.fn(),
    onStart: vi.fn(),
    onStop: vi.fn(),
    onInstall: vi.fn(),
    onRefresh: vi.fn(),
    onGroupTools: vi.fn(),
    onGroups: vi.fn(),
    groupsOpen: false,
    groupToolsOpen: false,
  };
}

describe("device toolbar", () => {
  it("configures a named master without enabling until the explicit action", async () => {
    const props = baseProps();
    render(
      <ProfileToolbar
        {...props}
        deviceNumbers={new Map([["a", 1], ["b", 7]])}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Đồng bộ" }));
    expect(props.onEnableSync).not.toHaveBeenCalled();
    expect(screen.queryByRole("option", { name: "Máy đang mở" })).toBeNull();
    fireEvent.change(screen.getByLabelText("Máy chính"), { target: { value: "b" } });
    expect(props.onMasterChange).toHaveBeenCalledWith("b");
    expect(screen.getByRole("option", { name: "Máy 7 · Same model" })).toBeVisible();
    await userEvent.click(screen.getByText("Độ trễ và độ lệch thao tác"));
    fireEvent.change(screen.getByLabelText("Độ trễ mỗi máy"), { target: { value: "staggered" } });
    fireEvent.change(screen.getByLabelText("Bước (ms mỗi máy)"), { target: { value: "350" } });
    await userEvent.click(screen.getByRole("button", { name: "Áp dụng đồng bộ nhóm" }));
    expect(getGroupSync().delay).toEqual({ mode: "staggered", stepMs: 350 });
    await userEvent.click(screen.getByRole("button", { name: "Bật đồng bộ thao tác" }));
    expect(props.onEnableSync).toHaveBeenCalledWith("a");
    expect(screen.queryByRole("region", { name: "Điều khiển đồng bộ" })).toBeNull();
  });

  it("disables activation for an offline target but always permits an explicit stop", async () => {
    const props = baseProps();
    const view = render(
      <ProfileToolbar
        {...props}
        selected={[devices[0], { ...devices[1], status: "disconnected" }]}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Đồng bộ" }));
    expect(screen.getByRole("button", { name: "Bật đồng bộ thao tác" })).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent("ngoại tuyến");

    view.rerender(
      <ProfileToolbar
        {...props}
        activeSync={{ masterUdid: "a", targetUdids: ["a", "b"] }}
        readiness={{ state: "degraded", readyUdids: ["a"], failures: { b: "device offline" } }}
      />,
    );
    expect(screen.getByText("Cần xử lý 1 máy")).toBeVisible();
    expect(screen.getByText("device offline")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Tắt đồng bộ" }));
    expect(props.onDisableSync).toHaveBeenCalledOnce();
  });

  it("shows preparing and active readiness with master and recipient rows", async () => {
    const props = baseProps();
    const view = render(
      <ProfileToolbar
        {...props}
        activeSync={{ masterUdid: "a", targetUdids: ["a", "b"] }}
        readiness={{ state: "preparing", readyUdids: [], failures: {} }}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /Đồng bộ · 2 máy/ }));
    expect(screen.getByText("Đang chuẩn bị 0/2")).toBeVisible();
    expect(screen.getAllByText("Máy chính")).toHaveLength(2);
    expect(screen.getByText("Máy nhận")).toBeVisible();
    view.rerender(
      <ProfileToolbar
        {...props}
        activeSync={{ masterUdid: "a", targetUdids: ["a", "b"] }}
        readiness={{ state: "active", readyUdids: ["a", "b"], failures: {} }}
      />,
    );
    expect(screen.getByText("Đang hoạt động 2/2")).toBeVisible();
    await userEvent.keyboard("{Escape}");
    expect(screen.getByRole("button", { name: /Đồng bộ · 2 máy/ })).toHaveFocus();
  });

  it("keeps repair behind the maintenance disclosure and dispatches only the explicit action", async () => {
    const props = baseProps();
    render(<ProfileToolbar {...props} selected={[]} />);
    expect(screen.getByRole("button", { name: "Sửa Riviu Agent" })).not.toBeVisible();
    await userEvent.click(screen.getByText("Bảo trì", { selector: "summary" }));
    expect(props.onInstall).not.toHaveBeenCalled();
    expect(screen.getByText("Các máy đang kết nối")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Sửa Riviu Agent" }));
    expect(props.onInstall).toHaveBeenCalledOnce();
  });

  it("closes maintenance with Escape and keeps refresh separate", async () => {
    const props = baseProps();
    render(<ProfileToolbar {...props} selected={[]} />);
    const menu = screen.getByText("Bảo trì", { selector: "summary" });
    await userEvent.click(menu);
    screen.getByRole("button", { name: "Sửa Riviu Agent" }).focus();
    await userEvent.keyboard("{Escape}");
    expect(menu).toHaveFocus();
    expect(props.onInstall).not.toHaveBeenCalled();
    await userEvent.click(screen.getByTitle("Quét lại thiết bị"));
    expect(props.onRefresh).toHaveBeenCalledOnce();
  });
});
