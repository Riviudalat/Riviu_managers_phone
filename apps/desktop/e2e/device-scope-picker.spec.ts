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
    // Nuôi edits its scope in its own "Chọn máy" drawer; the others share the scope drawer.
    for (const [name, opener, title] of [
      ["Nuôi TikTok", "Chọn máy", "Chọn máy Nuôi TikTok"],
      ["Tương tác", "Chọn thiết bị", "Chọn thiết bị thực hiện"],
      ["Đăng bài", "Chọn thiết bị", "Chọn thiết bị thực hiện"],
    ]) {
      await openOperatorPage(page, name);
      await page.getByRole("button", { name: opener, exact: true }).click();
      const dialog = page.getByRole("dialog", { name: title });
      await expect(dialog).toBeVisible();
      if (name !== "Nuôi TikTok") {
        await dialog.getByText("Máy cụ thể", { exact: true }).click();
        await expect(dialog.getByRole("radio", { name: "Máy cụ thể", exact: true })).toBeChecked();
      }
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
