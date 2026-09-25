import { expect, test } from "@playwright/test";

import { installTauriMock } from "./fixtures/tauriMock";
import { openOperatorPage } from "./fixtures/operatorNavigation";

test("header Inspector picks a connected Android device without sending an action", async ({ page }) => {
  await installTauriMock(page, { androidRoster: true, fleetSize: 2 });
  await page.addInitScript(() => {
    const runtime = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
      __INSPECTOR_CALLS__: { command: string; args?: Record<string, unknown> }[];
    };
    const invoke = runtime.__TAURI_INTERNALS__.invoke;
    runtime.__INSPECTOR_CALLS__ = [];
    runtime.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command.startsWith("inspector_")) runtime.__INSPECTOR_CALLS__.push({ command, args });
      if (command === "inspector_observe") return {
        id: "mock-snapshot", udid: args?.udid, package: "app.test", version: "1.0", locale: "en",
        width: 360, height: 720, pngBase64: "", treeSha256: "one", hierarchyXml: "<hierarchy/>", elements: [],
      };
      if (command === "inspector_recording") return null;
      return invoke(command, args);
    };
  });

  await page.goto("/");
  await openOperatorPage(page, "Cài đặt");
  const launch = page.getByRole("button", { name: "Mở Inspector" });
  await launch.click();
  const picker = page.getByRole("dialog", { name: "Chọn máy Android cho Inspector" });
  await expect(picker.locator(".inspector-picker-list button")).toHaveCount(2);
  expect(await page.evaluate(() => (window as unknown as { __INSPECTOR_CALLS__: unknown[] }).__INSPECTOR_CALLS__)).toEqual([]);

  await picker.locator(".inspector-picker-list button").first().click();
  const inspector = page.getByRole("dialog", { name: "Bắt thuộc tính và ghi Flow" });
  await expect(inspector.getByRole("img", { name: "Màn hình thiết bị" })).toBeVisible();
  const calls = await page.evaluate(() => (window as unknown as { __INSPECTOR_CALLS__: { command: string; args?: Record<string, unknown> }[] }).__INSPECTOR_CALLS__);
  expect(calls.some((call) => call.command === "inspector_observe")).toBe(true);
  expect(calls.every((call) => ["inspector_observe", "inspector_recording"].includes(call.command))).toBe(true);
  expect(calls.every((call) => call.args?.udid === "MOCK-FLEET-1")).toBe(true);
});
