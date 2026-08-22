import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DeviceContextMenu } from "./DeviceContextMenu";
import type { DeviceMenuNode } from "../deviceMenu";
import type { DeviceInfo } from "../types";

afterEach(cleanup);

const REDMI: DeviceInfo = {
  udid: "10969614",
  name: "Redmi 12C",
  model: "23021RAAEG",
  platform: "android",
  osVersion: "15",
  connection: "usb",
  status: "ready",
  battery: 84,
  wdaReady: true,
  wdaExpiresAt: null,
  streamUrl: null,
  tileStreamState: "sampling",
  lastError: null,
};

// What search matches and what a platform is offered are decided and tested in
// deviceMenu.test.ts, which needs no DOM. What is left here is the wiring the pure model
// cannot express: that a submenu opens, that a lazy one is only asked when opened, that a
// row closes the menu before it runs, and that a refusal from the phone becomes a sentence.
describe("DeviceContextMenu", () => {
  function open(nodes: DeviceMenuNode[], onClose = vi.fn()) {
    render(
      <DeviceContextMenu
        device={REDMI}
        groups={[]}
        x={10}
        y={10}
        nodes={nodes}
        onAddToGroup={vi.fn()}
        onClose={onClose}
      />,
    );
    return onClose;
  }

  it("runs a row and closes first, so the menu is not left over a confirm dialog", async () => {
    const run = vi.fn();
    const onClose = open([{ id: "a", label: "Chụp màn hình", run }]);

    await userEvent.click(screen.getByRole("menuitem", { name: "Chụp màn hình" }));

    expect(onClose).toHaveBeenCalled();
    expect(run).toHaveBeenCalled();
  });

  /** Hovering is the whole interaction the reference product uses; a click must not be needed. */
  it("opens a submenu on hover, with no click", async () => {
    open([
      {
        id: "adb",
        label: "ADB",
        children: [{ id: "dpi", label: "Đặt lại mật độ điểm" }],
      },
    ]);
    expect(screen.queryByText("Đặt lại mật độ điểm")).toBeNull();

    await userEvent.hover(screen.getByRole("menuitem", { name: /ADB/ }));

    expect(screen.getByText("Đặt lại mật độ điểm")).toBeTruthy();
  });

  it("still opens on a click, for touch and for the keyboard", async () => {
    open([
      { id: "adb", label: "ADB", children: [{ id: "dpi", label: "Đặt lại mật độ điểm" }] },
    ]);

    await userEvent.click(screen.getByRole("menuitem", { name: /ADB/ }));

    expect(screen.getByText("Đặt lại mật độ điểm")).toBeTruthy();
  });

  /**
   * The classic hover-menu bug: there are a couple of pixels between the row and its flyout,
   * and closing the instant the pointer leaves the row makes the submenu unreachable. The
   * pointer landing on the flyout must cancel the close.
   */
  it("keeps the submenu open while the pointer travels onto it", async () => {
    open([
      { id: "adb", label: "ADB", children: [{ id: "dpi", label: "Đặt lại mật độ điểm" }] },
    ]);
    const row = screen.getByRole("menuitem", { name: /ADB/ });
    await userEvent.hover(row);

    const child = screen.getByText("Đặt lại mật độ điểm");
    await userEvent.unhover(row);
    await userEvent.hover(child);

    await new Promise((resolve) => setTimeout(resolve, 260));
    expect(screen.getByText("Đặt lại mật độ điểm")).toBeTruthy();
  });

  it("closes the submenu shortly after the pointer leaves both", async () => {
    open([
      { id: "adb", label: "ADB", children: [{ id: "dpi", label: "Đặt lại mật độ điểm" }] },
    ]);
    const row = screen.getByRole("menuitem", { name: /ADB/ });
    await userEvent.hover(row);
    expect(screen.getByText("Đặt lại mật độ điểm")).toBeTruthy();

    await userEvent.unhover(row);

    await waitFor(() => expect(screen.queryByText("Đặt lại mật độ điểm")).toBeNull());
  });

  /**
   * The property that keeps a menu from costing twenty adb calls: a lazy submenu asks the
   * phone when the operator opens it and not when the menu is drawn.
   */
  it("asks the phone for a lazy submenu only when that row is opened", async () => {
    const load = vi.fn().mockResolvedValue([{ id: "app-1", label: "com.zhiliaoapp.musically" }]);
    open([{ id: "apps", label: "Ứng dụng trên máy", loadChildren: load }]);

    expect(load).not.toHaveBeenCalled();

    await userEvent.hover(screen.getByRole("menuitem", { name: /Ứng dụng trên máy/ }));

    await waitFor(() => expect(screen.getByText("com.zhiliaoapp.musically")).toBeTruthy());
    expect(load).toHaveBeenCalledTimes(1);
  });

  it("says what the phone refused with instead of showing an empty submenu", async () => {
    const load = vi.fn().mockRejectedValue(new Error("device unauthorized"));
    open([{ id: "apps", label: "Ứng dụng trên máy", loadChildren: load }]);

    await userEvent.click(screen.getByRole("menuitem", { name: /Ứng dụng trên máy/ }));

    await waitFor(() => expect(screen.getByText(/device unauthorized/)).toBeTruthy());
  });

  it("says a submenu came back empty, which is itself news", async () => {
    open([{ id: "apps", label: "Ứng dụng", loadChildren: vi.fn().mockResolvedValue([]) }]);

    await userEvent.click(screen.getByRole("menuitem", { name: /Ứng dụng/ }));

    await waitFor(() => expect(screen.getByText("Máy không trả về mục nào.")).toBeTruthy());
  });

  it("filters to the rows that match what was typed, submenu rows included", async () => {
    open([
      { id: "open", label: "Mở điều khiển" },
      { id: "adb", label: "ADB", children: [{ id: "dpi", label: "Đặt lại DPI", keywords: "dpi" }] },
    ]);

    await userEvent.type(screen.getByLabelText("Tìm chức năng"), "dpi");

    expect(screen.getByText("ADB › Đặt lại DPI")).toBeTruthy();
    expect(screen.queryByText("Mở điều khiển")).toBeNull();
  });

  it("says nothing matched rather than showing an empty menu", async () => {
    open([{ id: "open", label: "Mở điều khiển" }]);

    await userEvent.type(screen.getByLabelText("Tìm chức năng"), "zzzz");

    expect(screen.getByText("Không có chức năng nào khớp.")).toBeTruthy();
  });

  it("hides Android-only rows on an iPhone", () => {
    render(
      <DeviceContextMenu
        device={{ ...REDMI, platform: "ios", udid: "0000-iphone" }}
        groups={[]}
        x={10}
        y={10}
        nodes={[
          { id: "open", label: "Mở điều khiển" },
          { id: "wifi", label: "Bật Wi-Fi trên máy", androidOnly: true },
        ]}
        onAddToGroup={vi.fn()}
        onClose={vi.fn()}
      />,
    );

    expect(screen.getByText("Mở điều khiển")).toBeTruthy();
    expect(screen.queryByText("Bật Wi-Fi trên máy")).toBeNull();
  });
});
