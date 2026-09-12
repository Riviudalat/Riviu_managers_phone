import { expect, test } from "@playwright/test";
import { installTauriMock, mockCommandCalls } from "./fixtures/tauriMock";

test.beforeEach(async ({ page }) => {
  await installTauriMock(page);
  await page.goto("/");
  await expect(page.getByTestId("device-tile")).toHaveCount(2);
});

test("device maintenance names its scope and requires the existing confirmation", async ({ page }) => {
  await expect(page.getByTitle("Quét lại thiết bị")).toHaveCount(1);
  await expect(page.getByTitle("Làm mới danh sách máy")).toHaveCount(0);
  await page.getByText("Bảo trì", { selector: "summary", exact: true }).click();
  await expect(page.getByText("Các máy đang kết nối", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Sửa Riviu Agent", exact: true }).click();
  const confirmation = page.getByRole("alertdialog");
  await expect(confirmation).toContainText("Sửa Riviu Agent trên 2 máy?");
  expect((await mockCommandCalls(page)).filter(call => call.command === "agent_bulk_repair")).toHaveLength(0);
  await page.keyboard.press("Escape");
  await expect(confirmation).toHaveCount(0);
});

test("group tools use keyboard tabs and return focus to their launcher", async ({ page }) => {
  const launch = page.getByRole("button", { name: "Công cụ", exact: true });
  await launch.click();
  const dialog = page.getByRole("dialog", { name: /Công cụ nhóm/ });
  await expect(dialog).toBeVisible();
  const tabs = dialog.getByRole("tablist");
  const first = tabs.getByRole("tab").first();
  await first.focus();
  await page.keyboard.press("ArrowRight");
  await expect(tabs.getByRole("tab").nth(1)).toBeFocused();
  await expect(tabs.getByRole("tab").nth(1)).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(launch).toBeFocused();
});

test("API connection link opens and focuses the integration settings", async ({ page }) => {
  await page.getByRole("navigation", { name: "Điều hướng chính" }).getByRole("button", { name: "API", exact: true }).click();
  await page.getByRole("button", { name: "Cấu hình kết nối", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Cài đặt" })).toBeVisible();
  await expect(page.locator("#settings-integration")).toBeFocused();
  await expect(page.getByRole("navigation", { name: "Nhóm cài đặt" }).getByRole("link", { name: "Kết nối và API" })).toHaveAttribute("aria-current", "location");
});
