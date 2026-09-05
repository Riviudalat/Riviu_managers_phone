import { expect, test } from "@playwright/test";
import { installTauriMock, mockCommandCalls } from "./fixtures/tauriMock";

for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
  test(`library view controls preserve scope and keyboard access at ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await installTauriMock(page, { androidRoster: true });
    await page.goto("/");
    await page.getByRole("button", { name: "Trung tâm ứng dụng", exact: true }).click();
    const columns = page.getByLabel("Chọn cột: Thư viện ứng dụng");
    await columns.focus();
    await page.keyboard.press("Enter");
    await page.getByRole("checkbox", { name: "Nền tảng", exact: true }).uncheck();
    await page.keyboard.press("Escape");
    await expect(columns).toBeFocused();
    await expect(page.getByRole("columnheader", { name: "Nền tảng" })).toHaveCount(0);
    await page.reload();
    await page.getByRole("button", { name: "Trung tâm ứng dụng", exact: true }).click();
    await expect(page.getByRole("columnheader", { name: "Nền tảng" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Cài → 0 Android" })).toBeDisabled();
    const add = page.getByRole("button", { name: "Thêm gói", exact: true });
    await add.click();
    const drawer = page.getByRole("dialog", { name: "Thêm gói cài đặt" });
    await expect(drawer).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(drawer).toHaveCount(0);
    await expect(add).toBeFocused();
    expect((await mockCommandCalls(page)).filter(({ command }) => command === "install_library_app_batch")).toHaveLength(0);
  });
}
