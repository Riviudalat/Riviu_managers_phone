import { test, expect } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";

test("preview scales, display rail hides, and group creation stays reachable", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await installTauriMock(page, { androidRoster: true, fleetSize: 4 });
  await page.goto("/");
  const tiles = page.getByTestId("device-tile");
  await expect(tiles).toHaveCount(4);
  await page.getByRole("button", { name: "Hiện bảng Hiển thị", exact: true }).hover();
  // Match the real stream's small encoded frame. The CSS must enlarge it with its tile.
  await page.evaluate(() => {
    for (const host of document.querySelectorAll(".dev-phone-screen .phone-canvas-host")) {
      const canvas = document.createElement("canvas");
      canvas.width = 232; canvas.height = 480; canvas.className = "phone-canvas";
      canvas.getContext("2d")!.fillRect(0, 0, 232, 480);
      host.replaceChildren(canvas);
    }
  });
  await page.getByRole("slider", { name: "Kích thước ô xem trước", exact: true }).fill("350");
  const screen = tiles.first().locator(".dev-phone-screen");
  const canvas = screen.locator("canvas");
  const bounds = await screen.boundingBox();
  const image = await canvas.boundingBox();
  expect(image!.width).toBeGreaterThan(300);
  expect(Math.abs(image!.width - bounds!.width)).toBeLessThan(2);
  expect(Math.abs(image!.height - bounds!.height)).toBeLessThan(2);
  await page.keyboard.press("Escape");
  await page.mouse.move(1400, 10);
  await expect(page.getByRole("slider", { name: "Kích thước ô xem trước", exact: true })).toBeHidden();
  await page.getByRole("button", { name: "Hiện bảng Hiển thị", exact: true }).click();
  await expect(page.getByRole("slider", { name: "Kích thước ô xem trước", exact: true })).toHaveValue("350");
  await page.getByRole("button", { name: "Tạo nhóm thiết bị", exact: true }).click();
  await expect(page.getByRole("button", { name: /Tạo Nhóm/ })).toBeVisible();
});

test("Automation collapses while the three original workspaces remain direct destinations", async ({ page }) => {
  await installTauriMock(page);
  await page.goto("/");
  const nav = page.getByRole("navigation", { name: "Điều hướng chính" });
  await nav.getByRole("button", { name: "Automation", exact: true }).click();
  for (const name of ["Nuôi TikTok", "Tương tác", "Đăng bài"]) {
    await expect(nav.getByRole("button", { name, exact: true })).toBeHidden();
  }
  await page.reload();
  await expect(nav.getByRole("button", { name: "Automation", exact: true })).toHaveAttribute("aria-expanded", "false");
  await nav.getByRole("button", { name: "Automation", exact: true }).click();
  for (const name of ["Nuôi TikTok", "Tương tác", "Đăng bài"]) {
    await nav.getByRole("button", { name, exact: true }).click();
    await expect(page.getByRole("heading", { name, exact: true }).first()).toBeVisible();
    await expect(page.locator(".react-flow")).toHaveCount(0);
  }
  await nav.getByRole("button", { name: "My Apps", exact: true }).click();
  await expect(page.getByRole("button", { name: "Thêm Flow", exact: true })).toHaveCount(3);
  await page.getByRole("button", { name: "Mở chức năng", exact: true }).first().click();
  await expect(page.getByRole("heading", { name: "Nuôi TikTok", exact: true }).first()).toBeVisible();
});

test("switching A to B reuses one window and waits for A to release; file actions browse B", async ({ page }) => {
  await installTauriMock(page, { androidRoster: true, fleetSize: 3 });
  await page.addInitScript(() => {
    const w = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
      __HANDOFF__: string[];
    };
    const invoke = w.__TAURI_INTERNALS__.invoke;
    w.__HANDOFF__ = [];
    w.__TAURI_INTERNALS__.invoke = async (cmd, args = {}) => {
      if (cmd === "device_control_end") {
        await new Promise(resolve => setTimeout(resolve, 250));
        w.__HANDOFF__.push(`end:${args.udid}`); return null;
      }
      if (cmd === "device_control_begin") { w.__HANDOFF__.push(`begin:${args.udid}`); return null; }
      if (cmd === "device_list_dir") {
        w.__HANDOFF__.push(`list:${args.udid}:${args.path}`);
        return { entries: [{ name: "Download", kind: "directory", size: 0, modified: null, linkTarget: null }], incomplete: null };
      }
      return invoke(cmd, args);
    };
  });
  await page.goto("/");
  const tiles = page.getByTestId("device-tile");
  await tiles.first().dblclick();
  const focus = page.locator(".device-floating-window");
  await expect(focus.getByRole("button", { name: "Home", exact: true })).toBeEnabled();
  await focus.evaluate(el => el.setAttribute("data-reused", "yes"));
  const firstId = await focus.locator("canvas").getAttribute("data-udid");
  // The second tile can be partly behind the floating screen; a plain selection must switch it.
  await tiles.nth(1).dispatchEvent("click", { button: 0 });
  await expect(focus).toHaveCount(1);
  await expect(focus).toHaveAttribute("data-reused", "yes");
  await expect(focus).toHaveAttribute("aria-label", "Điều khiển Máy thử 2");
  await expect(focus.getByRole("button", { name: "Home", exact: true })).toBeEnabled();
  const secondId = await focus.locator("canvas").getAttribute("data-udid");
  const calls = await page.evaluate(() => (window as unknown as { __HANDOFF__: string[] }).__HANDOFF__);
  expect(calls.indexOf(`end:${firstId}`)).toBeLessThan(calls.indexOf(`begin:${secondId}`));
  await expect(focus.getByTestId("focus-control-status")).toHaveCount(0);
  for (const name of ["Điện thoại → PC"]) {
    await focus.getByRole("button", { name, exact: true }).click();
    const files = page.getByRole("dialog", { name: "Tệp trên Máy thử 2", exact: true });
    await expect(files.getByRole("checkbox", { name: "Chọn Download" })).toBeVisible();
    expect(await files.evaluate(el => !el.closest(".device-floating-window"))).toBe(true);
    await files.getByRole("button", { name: "Đóng", exact: true }).click();
  }
  const listings = await page.evaluate(() => (window as unknown as { __HANDOFF__: string[] }).__HANDOFF__.filter(v => v.startsWith("list:")));
  expect(listings.length).toBeGreaterThanOrEqual(1);
  expect(listings.every(v => v === `list:${secondId}:/sdcard`)).toBe(true);
  await page.screenshot({ path: test.info().outputPath("single-window.png") });
});
