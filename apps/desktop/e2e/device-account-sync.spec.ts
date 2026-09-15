import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";

for (const width of [1440, 820]) {
  test(`account menu and sync controls at ${width}`, async ({ page }, info) => {
    await page.setViewportSize({ width, height: width === 820 ? 560 : 900 });
    await page.emulateMedia({ reducedMotion: "reduce" });
    await installTauriMock(page, { androidRoster: true, fleetSize: 3 });
    await page.addInitScript(() => {
      const w = window as unknown as {
        __TAURI_INTERNALS__: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
        __MENU_CALLS__: { cmd: string; args: Record<string, unknown> }[];
        __READY_SYNC__: () => void;
      };
      const original = w.__TAURI_INTERNALS__.invoke;
      const metas = [1, 2, 3].map(n => ({ udid: `MOCK-FLEET-${n}`, number: [1, 7, 9][n - 1], alias: `Máy phụ ${n}`, handle: "", notes: "", tags: [], groupId: null }));
      const pendingControl: (() => void)[] = [];
      let holdControl = true;
      w.__READY_SYNC__ = () => {
        holdControl = false;
        pendingControl.splice(0).forEach(resolve => resolve());
      };
      w.__MENU_CALLS__ = [];
      w.__TAURI_INTERNALS__.invoke = async (cmd, args = {}) => {
        if (cmd === "startup_error") return null;
        if (cmd === "list_device_metas") return structuredClone(metas);
        if (cmd === "interaction_read_account") {
          w.__MENU_CALLS__.push({ cmd, args });
          const meta = metas.find(m => m.udid === args.udid)!;
          return { udid: meta.udid, expectedHandle: meta.handle, observedHandle: `nick_${meta.number}`, status: meta.handle ? "matched" : "unassigned", checkedAt: new Date().toISOString(), snapshotSha256: "proof" };
        }
        if (cmd === "save_device_handle") {
          w.__MENU_CALLS__.push({ cmd, args });
          const meta = metas.find(m => m.udid === args.udid)!;
          if (meta.handle !== args.expectedHandle) throw Error("stale account");
          meta.handle = String(args.handle);
          return meta.handle;
        }
        if (cmd === "device_control_begin") {
          if (!holdControl) return null;
          return new Promise<void>(resolve => pendingControl.push(resolve));
        }
        if (cmd === "device_control_end") return null;
        if (cmd === "group_input") {
          w.__MENU_CALLS__.push({ cmd, args });
          return { completedUdids: args.udids, skipped: [] };
        }
        if (cmd === "device_key") {
          w.__MENU_CALLS__.push({ cmd, args });
          return null;
        }
        return original(cmd, args);
      };
    });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.goto("/");
    const tiles = page.getByTestId("device-tile");
    await expect(tiles).toHaveCount(3);
    const calls = () => page.evaluate(() => (window as unknown as { __MENU_CALLS__: { cmd: string; args: Record<string, unknown> }[] }).__MENU_CALLS__);
    expect(await calls()).toEqual([]);
    await tiles.nth(0).click();
    await tiles.nth(1).click({ modifiers: ["Control"] });
    await tiles.nth(0).click({ button: "right" });
    const menu = page.getByRole("menu", { name: /Tác vụ cho/ });
    await expect(menu.getByRole("menuitem", { name: "Đọc và gán nick TikTok (2 máy đã chọn)" })).toBeVisible();
    await page.screenshot({ path: info.outputPath(`account-menu-${width}.png`) });
    await menu.getByRole("menuitem", { name: "Đọc và gán nick TikTok (2 máy đã chọn)" }).click();
    await expect(tiles.nth(0).locator(".dev-phone-handle")).toHaveText("@nick_1");
    await expect(tiles.nth(1).locator(".dev-phone-handle")).toHaveText("@nick_7");
    await expect(tiles.nth(2).locator(".dev-phone-handle")).toHaveCount(0);
    expect((await calls()).filter(c => c.cmd === "save_device_handle").map(c => c.args.udid)).toEqual(["MOCK-FLEET-1", "MOCK-FLEET-2"]);

    await tiles.nth(2).click({ modifiers: ["Control"] });
    const toolbar = page.getByRole("group", { name: "Thao tác thiết bị" });
    await toolbar.getByRole("button", { name: "Đồng bộ", exact: true }).click();
    const sync = page.getByRole("region", { name: "Điều khiển đồng bộ" });
    await expect(sync).toContainText("Đang tắt");
    await sync.getByLabel("Máy chính").selectOption("MOCK-FLEET-2");
    await expect(sync.getByRole("option", { name: "Máy 7 · Máy phụ 2 · @nick_7" })).toBeAttached();
    await sync.getByText("Độ trễ và độ lệch thao tác", { exact: true }).click();
    await sync.getByLabel("Độ trễ mỗi máy").selectOption("staggered");
    await sync.getByLabel("Bước (ms mỗi máy)").fill("250");
    await sync.getByRole("button", { name: "Áp dụng đồng bộ nhóm" }).click();
    await expect(sync.getByText("Đã áp dụng", { exact: true })).toBeVisible();
    await sync.getByText("Độ trễ và độ lệch thao tác", { exact: true }).click();
    const rect = await sync.boundingBox();
    expect(rect!.x + rect!.width).toBeLessThanOrEqual(width);
    expect(rect!.y + rect!.height).toBeLessThanOrEqual(width === 820 ? 560 : 900);
    await page.screenshot({ path: info.outputPath(`sync-panel-${width}.png`) });
    await sync.getByRole("button", { name: "Bật đồng bộ thao tác" }).click();

    const focus = page.getByRole("dialog", { name: "Điều khiển Máy thử 2", exact: true });
    await expect(page.locator(".focus-overlay")).toHaveCount(1);
    await expect(focus).toBeVisible();
    await page.evaluate(() => {
      if ("__READY_SYNC__" in window && typeof window.__READY_SYNC__ === "function") {
        window.__READY_SYNC__();
      }
    });
    await expect(focus.getByTestId("focus-control-status")).toContainText("Đang hoạt động 3/3");
    await expect(toolbar.getByRole("button", { name: "Đồng bộ · 3 máy", exact: true })).toBeVisible();

    await toolbar.getByRole("button", { name: "Đồng bộ · 3 máy", exact: true }).evaluate((button) => {
      if (button instanceof HTMLButtonElement) button.click();
    });
    await expect(sync.getByText("Máy chính", { exact: true })).toHaveCount(1);
    await expect(sync.getByText("Máy nhận", { exact: true })).toHaveCount(2);
    await expect(sync).toContainText("Đang hoạt động 3/3");
    await sync.getByRole("button", { name: "Đóng bảng" }).focus();
    await page.keyboard.press("Escape");
    await expect(toolbar.getByRole("button", { name: "Đồng bộ · 3 máy", exact: true })).toBeFocused();

    await focus.getByRole("button", { name: "Home", exact: true }).click();
    await expect.poll(async () => (await calls()).filter(c => c.cmd === "group_input").length).toBe(1);
    const group = (await calls()).find(c => c.cmd === "group_input")!;
    expect(group.args.udids).toEqual(["MOCK-FLEET-2", "MOCK-FLEET-1", "MOCK-FLEET-3"]);
    expect(group.args.masterUdid).toBe("MOCK-FLEET-2");
    expect(group.args.sync).toMatchObject({ delay: { mode: "staggered", stepMs: 250 } });

    await tiles.nth(2).evaluate((tile) => {
      tile.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true }));
    });
    await expect(toolbar.getByRole("button", { name: "Đồng bộ", exact: true })).toBeVisible();
    await expect(focus).toBeVisible();
    await expect(focus.getByTestId("focus-control-status")).toHaveCount(0);
    await expect(focus.getByRole("button", { name: "Home", exact: true })).toBeEnabled();
    await focus.getByRole("button", { name: "Home", exact: true }).click();
    await expect.poll(async () => (await calls()).filter(c => c.cmd === "device_key").length).toBe(1);
    expect((await calls()).filter(c => c.cmd === "group_input")).toHaveLength(1);
    expect(errors).toEqual([]);
  });
}
