import { expect, test } from "@playwright/test";
import { emitRiviuEvent, installTauriMock, mockCommandCalls } from "./fixtures/tauriMock";

test("iOS warning follows connected iPhones and stays inspectable in Settings", async ({ page }, info) => {
  await installTauriMock(page, { androidRoster: true });
  await page.addInitScript(() => {
    const app = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
      __IOS_ISSUE__: string | null;
    };
    const invoke = app.__TAURI_INTERNALS__.invoke;
    app.__IOS_ISSUE__ = "iOS agent runtime unavailable: fixture credential missing";
    app.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "startup_error") return null;
      if (command === "driver_degraded_reason") return app.__IOS_ISSUE__;
      return invoke(command, args);
    };
  });
  await page.goto("/");
  await expect(page.getByTestId("device-tile")).toHaveCount(2);
  const warning = page.getByText(/Nhánh iOS không sẵn sàng/);
  await expect(warning).toHaveCount(0);
  await page.screenshot({ path: info.outputPath("android-without-ios-warning.png") });
  const nav = page.getByRole("navigation", { name: "Điều hướng chính" });
  await nav.getByRole("button", { name: "Cài đặt", exact: true }).click();
  await page.getByRole("link", { name: "Bảo trì", exact: true }).click();
  const detail = page.locator('details[aria-label="Cấu hình Agent iOS"]');
  await expect(detail).not.toHaveAttribute("open");
  await expect(page.getByText(/fixture credential missing/)).toBeHidden();
  await detail.locator("summary").click();
  await expect(detail).toContainText("fixture credential missing");
  await expect(detail).toContainText("không quyết định trạng thái Agent Android");
  await page.screenshot({ path: info.outputPath("ios-diagnostic-disclosure.png") });
  await nav.getByRole("button", { name: "Thiết bị", exact: true }).click();
  const android = await page.evaluate(() => (window as unknown as {
    __TAURI_INTERNALS__: { invoke: (command: string) => Promise<Record<string, unknown>[]> };
  }).__TAURI_INTERNALS__.invoke("list_devices"));
  const iphone = { ...android[0], udid: "IOS-NOTICE-FIXTURE", platform: "ios", name: "iPhone fixture", status: "connected" };
  await emitRiviuEvent(page, { type: "devicesUpdated", devices: [...android, iphone] });
  await expect(warning).toBeVisible();
  await expect(page.getByTestId("device-tile")).toHaveCount(3);
  await emitRiviuEvent(page, { type: "devicesUpdated", devices: [...android, { ...iphone, status: "disconnected" }] });
  await expect(warning).toHaveCount(0);
  await emitRiviuEvent(page, { type: "devicesUpdated", devices: [] });
  await expect(warning).toHaveCount(0);
  await page.evaluate(() => { (window as unknown as { __IOS_ISSUE__: string | null }).__IOS_ISSUE__ = null; });
  await page.getByTitle("Quét lại thiết bị").click();
  await expect(page.getByTestId("device-tile")).toHaveCount(2);
  await emitRiviuEvent(page, { type: "devicesUpdated", devices: [...android, iphone] });
  await expect(warning).toHaveCount(0);
  expect((await mockCommandCalls(page)).filter(call => /^(agent_repair|agent_preflight|agent_bulk_repair|install_ipa)$/.test(call.command))).toEqual([]);
});
