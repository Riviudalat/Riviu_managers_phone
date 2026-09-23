import { test, expect } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";
import { openOperatorPage } from "./fixtures/operatorNavigation";

for (const width of [1440, 820]) {
  test(`empty explicit scope can pick devices at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: width === 820 ? 560 : 900 });
    await installTauriMock(page, { androidRoster: true, fleetSize: 3 });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.goto("/");
    for (const name of ["Nuôi TikTok", "Tương tác", "Đăng bài"]) {
      await openOperatorPage(page, name);
      await page.getByRole("button", { name: "Chọn thiết bị", exact: true }).click();
      const dialog = page.getByRole("dialog", { name: "Chọn thiết bị thực hiện" });
      await expect(dialog).toBeVisible();
      await dialog.getByText("Máy cụ thể", { exact: true }).click();
      await expect(dialog.getByRole("radio", { name: "Máy cụ thể", exact: true })).toBeChecked();
      const choices = dialog.getByRole("checkbox");
      expect(await choices.count()).toBeGreaterThan(1);
      await choices.first().check();
      await expect(choices.first()).toBeChecked();
      await page.screenshot({ path: test.info().outputPath(`picker-${name}-${width}.png`) });
      await dialog.getByRole("button", { name: "Xong", exact: true }).click();
    }
    expect(errors).toEqual([]);
  });
}
