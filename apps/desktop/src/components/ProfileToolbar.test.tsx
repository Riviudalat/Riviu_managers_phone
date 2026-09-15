import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getGroupSync, defaultGroupSync, setGroupSync } from "../groupSync";
import type { DeviceInfo } from "../types";
import { ProfileToolbar } from "./ProfileToolbar";

afterEach(()=>{cleanup();setGroupSync(defaultGroupSync());});

function setup() {
  const onInstall = vi.fn();
  const onRefresh = vi.fn();
  render(<ProfileToolbar selected={[]} deviceCount={3} onStart={vi.fn()} onStop={vi.fn()}
    onInstall={onInstall} onSync={vi.fn()} onRefresh={onRefresh} onGroupTools={vi.fn()}
    onGroups={vi.fn()} groupsOpen={false} groupToolsOpen={false} syncOn={false} />);
  return { onInstall, onRefresh };
}

describe("device toolbar", () => {
  it("opens settings without enabling sync, shows numbered targets and applies the shared policy",async()=>{
    const onSync=vi.fn(),onControlCenter=vi.fn();
    const devices=[{udid:"a",name:"Same model"},{udid:"b",name:"Same model"}] as DeviceInfo[];
    render(<ProfileToolbar selected={devices} deviceNumbers={new Map([["a",1],["b",7]])} deviceCount={2} controlCenter={null} onControlCenter={onControlCenter} onStart={vi.fn()} onStop={vi.fn()} onInstall={vi.fn()} onRefresh={vi.fn()} onGroups={vi.fn()} onGroupTools={vi.fn()} groupsOpen={false} groupToolsOpen={false} syncOn={false} onSync={onSync}/>);
    await userEvent.click(screen.getByRole("button",{name:"Đồng bộ"}));
    expect(onSync).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("Máy chính"),{target:{value:"b"}});
    expect(onControlCenter).toHaveBeenCalledWith("b");
    expect(screen.getByRole("option",{name:"Máy 7 · Same model"})).toBeVisible();
    await userEvent.click(screen.getByText("Độ trễ và độ lệch thao tác"));
    fireEvent.change(screen.getByLabelText("Độ trễ mỗi máy"),{target:{value:"staggered"}});
    fireEvent.change(screen.getByLabelText("Bước (ms mỗi máy)"),{target:{value:"350"}});
    await userEvent.click(screen.getByRole("button",{name:"Áp dụng đồng bộ nhóm"}));
    expect(getGroupSync().delay).toEqual({mode:"staggered",stepMs:350});
    await userEvent.click(screen.getByRole("button",{name:"Bật đồng bộ thao tác"}));
    expect(onSync).toHaveBeenCalledOnce();
  });

  it("refuses enabling a single phone but lets the operator turn off an active group",async()=>{
    const props={selected:[{udid:"a",name:"Phone"}] as DeviceInfo[],deviceCount:1,onSync:vi.fn(),onStart:vi.fn(),onStop:vi.fn(),onInstall:vi.fn(),onRefresh:vi.fn(),onGroups:vi.fn(),onGroupTools:vi.fn(),groupsOpen:false,groupToolsOpen:false};
    const view=render(<ProfileToolbar {...props} syncOn={false}/>);
    await userEvent.click(screen.getByRole("button",{name:"Đồng bộ"}));
    expect(screen.getByRole("button",{name:"Bật đồng bộ thao tác"})).toBeDisabled();
    view.rerender(<ProfileToolbar {...props} syncOn/>);
    await userEvent.click(screen.getByRole("button",{name:"Tắt đồng bộ thao tác"}));
    expect(props.onSync).toHaveBeenCalledOnce();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("region",{name:"Điều khiển đồng bộ"})).toBeNull();
  });
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
