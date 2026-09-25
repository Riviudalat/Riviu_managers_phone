import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installTauriMock } from "./fixtures/tauriMock";
import { openOperatorPage } from "./fixtures/operatorNavigation";
for (const width of [1440, 820]) {
  test(`publish warns on the exact pending machine before preflight at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: width === 820 ? 560 : 900 });
    await installTauriMock(page, { androidRoster: true, fleetSize: 3 });
    await page.addInitScript(() => {
      const w = window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> }; pendingResolved: boolean; publishCalls: string[] };
      const invoke = w.__TAURI_INTERNALS__.invoke; w.pendingResolved = true; w.publishCalls = [];
      w.__TAURI_INTERNALS__.invoke = async (cmd, args = {}) => {
        if (cmd === "plugin:dialog|open") return "C:/fixture";
        if (cmd === "publish_scan_folder") return { sourceRoot: "C:/fixture", scannedAt: new Date().toISOString(), notices: [], ignoredPartnerFiles: 0, ignoredHiddenFiles: 0,
          bundles: [1, 2].map(n => ({ id: `b${n}`, name: `Bài ${n}`, sourcePath: `C:/fixture/${n}`, mediaKind: "image", images: [], captionPath: "caption", caption: "Nội dung đã chuẩn bị", captionSha256: "a".repeat(64), totalBytes: 1 })) };
        if (cmd === "publish_device_guards") return Object.fromEntries((args.udids as string[]).map((id, index) => [id, { blocking: !w.pendingResolved && index === 0 ? [{ assignmentId: "pending-a", campaignId: "old-pending", updatedAt: new Date().toISOString(), reason: "TikTok báo bài đang được xử lý" }] : [], linkReview: [] }]));
        if (cmd === "publish_preflight" || cmd === "publish_execute" || cmd === "publish_create_campaign") { w.publishCalls.push(cmd); throw Error("No posting in pending guard test"); }
        return invoke(cmd, args);
      };
    });
    await page.goto("/"); await openOperatorPage(page, "Đăng bài");
    await page.getByRole("button", { name: "Chọn thư mục", exact: true }).click(); await page.getByRole("button", { name: "Quét", exact: true }).click();
    await page.getByRole("checkbox", { name: "Chọn Bài 1", exact: true }).check();
    const machine = page.getByRole("combobox", { name: "Máy nhận bài Bài 1", exact: true });
    const first = await machine.locator("option").nth(1).getAttribute("value"); await machine.selectOption(first!);
    await page.evaluate(() => { (window as unknown as { pendingResolved: boolean }).pendingResolved = false; window.dispatchEvent(new Event("focus")); });
    await expect(page.getByRole("button", { name: "Kiểm tra & đăng", exact: true })).toBeEnabled();
    await expect(machine).toHaveValue(first!);
    const warning = page.locator(".machine-choice").filter({ hasText: "Máy còn bài chưa lấy được link" });
    await expect(warning).toHaveCount(1); await expect(warning).toContainText("TikTok báo bài đang được xử lý");
    await warning.scrollIntoViewIfNeeded();
    await page.screenshot({ path: test.info().outputPath(`pending-warning-${width}.png`) });
    await page.getByRole("button", { name: "Chọn nhanh", exact: true }).click();
    await expect(machine).toHaveValue(first!);
    await expect(machine.locator(`option[value="${first}"]`)).toHaveJSProperty("disabled", false);
    await page.evaluate(() => { (window as unknown as { pendingResolved: boolean }).pendingResolved = true; window.dispatchEvent(new Event("focus")); });
    await expect(machine.locator(`option[value="${first}"]`)).toHaveJSProperty("disabled", false);
    await machine.selectOption(first!);
    await expect(page.getByRole("button", { name: "Kiểm tra & đăng", exact: true })).toBeEnabled();
    await expect(page.getByText("Máy còn bài chưa lấy được link", { exact: true })).toHaveCount(0);
    expect(await page.evaluate(() => (window as unknown as { publishCalls: string[] }).publishCalls)).toEqual([]);
    const sheetStatus = page.locator(".publish-sheet-result");
    await expect(sheetStatus).toContainText("Bản app chưa có cấu hình Google");
    if (width === 820) {
      await page.getByRole("button", { name: "Thiết lập Google Sheet", exact: true }).focus();
      await page.keyboard.press("Tab");
      await expect(sheetStatus).toBeFocused();
      await page.getByRole("button", { name: "Thiết lập Google Sheet", exact: true }).click();
      await expect(page.locator(".pq-settings.is-open")).toBeVisible();
      await expect(sheetStatus).toContainText("Mở Thiết lập Google để bổ sung.");
      expect(await sheetStatus.evaluate(node => node.scrollWidth <= node.clientWidth)).toBe(true);
    }
    expect((await new AxeBuilder({ page }).include(".publish-page").analyze()).violations).toEqual([]);
    await page.screenshot({ path: test.info().outputPath(`pending-cleared-${width}.png`) });
  });
}
