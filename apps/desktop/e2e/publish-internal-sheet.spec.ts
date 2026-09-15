import { openOperatorPage } from "./fixtures/operatorNavigation";
import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installTauriMock } from "./fixtures/tauriMock";

for (const width of [1440, 820]) {
  test(`compact Sheet check preserves source controls and readiness at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: width === 820 ? 560 : 900 });
    await installTauriMock(page);
    await page.addInitScript(() => {
      const w = window as unknown as {
        __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> };
        sheetCalls: { command: string; args: Record<string, unknown> }[];
        setSheetReady: () => void;
      };
      const original = w.__TAURI_INTERNALS__.invoke;
      const url = "https://docs.google.com/spreadsheets/d/SAVED/edit#gid=0";
      let reportingReady = false;
      w.sheetCalls = [];
      w.setSheetReady = () => { reportingReady = true; };
      w.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
        if (command === "publish_sheet_get_config") return {
          provider: "googleDirect", webhookUrl: "https://script.google.com/fixture/exec", hasToken: true,
          internalReporting: true, sheetUrl: url,
        };
        if (command === "google_sheets_status") return {
          configured: true, connected: true, active: true, clientId: "fixture.apps.googleusercontent.com",
          pickerConfigured: true, phase: "idle", error: null, email: "operator@example.test", accountId: "account-a",
          selectedFileId: "SAVED", selectedFileName: "Kết quả", sheetUrl: url, writerId: "fixture-writer",
        };
        if (command === "publish_sheet_check") {
          w.sheetCalls.push({ command, args });
          if (args.sheetUrl !== url) throw Error("Check must match the active workbook and gid");
          return {
            sheetUrl: url, spreadsheetId: "SAVED", sheetGid: 0, reportingEpoch: "fixture-epoch",
            readable: true, connectionVerified: true, reportingReady, layout: "internal",
            columns: ["Máy", "Tài khoản TikTok", "Trạng thái", "Lỗi hoặc ghi chú", "Đối tác 1"],
            message: reportingReady ? "Đã xác minh kết nối đúng Sheet." : "Đợt báo cáo chưa sẵn sàng nhận dữ liệu.",
          };
        }
        if (command === "publish_sheet_save_config" || command === "publish_sheet_prepare"
          || command === "publish_execute" || command.startsWith("publish_create")
          || command === "automation_profile_create" || command === "google_sheets_connect"
          || command === "google_sheets_pick_file") {
          w.sheetCalls.push({ command, args });
          throw new Error("Unexpected mutation during active connection check");
        }
        return original(command, args);
      };
    });
    await page.goto("/");
    await openOperatorPage(page, "Đăng bài");
    await expect(page.getByRole("tablist", { name: "Chế độ Đăng bài" }).getByRole("tab")).toHaveText(["Thiết lập", "Hẹn giờ", "Theo dõi"]);
    await expect(page.getByRole("button", { name: /Hồ sơ|Nhập nội dung/ })).toHaveCount(0);
    const controls = page.locator(".pq-source-controls");
    await expect(controls.getByRole("button", { name: "Chọn thư mục", exact: true })).toBeVisible();
    await expect(controls.getByRole("button", { name: "Quét", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Chọn thư mục", exact: true })).toHaveCount(1);
    const panel = page.locator(".google-sheet-connection");
    const link = panel.getByRole("textbox", { name: "Link Google Sheet" });
    const check = panel.getByRole("button", { name: "Kiểm tra kết nối", exact: true });
    await expect(link).toHaveValue("https://docs.google.com/spreadsheets/d/SAVED/edit#gid=0");
    await expect(link).toBeEditable();
    await expect(panel.getByRole("button")).toHaveCount(2);
    await expect(panel.locator("input")).toHaveCount(1);
    await expect(page.getByText(/Apps Script/)).toHaveCount(0);
    await link.fill("");
    await expect(check).toBeDisabled();
    await link.fill("https://docs.google.com/spreadsheets/d/SAVED/edit#gid=0");
    await check.click();
    await expect(panel.locator(".publish-sheet-result[role=status]")).toContainText("chưa sẵn sàng");
    await expect(panel.locator(".publish-sheet-result.is-verified")).toHaveCount(0);
    await page.evaluate(() => (window as unknown as { setSheetReady: () => void }).setSheetReady());
    await check.click();
    await expect(panel.locator(".publish-sheet-result.is-verified")).toContainText("Kết nối đã xác minh.");
    await link.fill("https://docs.google.com/spreadsheets/d/SAVED/edit#gid=7");
    await expect(panel.locator(".publish-sheet-result.is-verified")).toHaveCount(0);
    const calls = await page.evaluate(() => (window as unknown as { sheetCalls: { command: string }[] }).sheetCalls);
    expect(calls.filter(call => call.command !== "publish_sheet_check")).toEqual([]);
    expect(calls.filter(call => call.command === "publish_sheet_check").length).toBeGreaterThanOrEqual(2);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const box = await page.locator(".pq-footer").boundingBox();
    expect(box!.y + box!.height).toBeLessThanOrEqual(width === 820 ? 560 : 900);
    expect((await new AxeBuilder({ page }).include(".publish-page").analyze()).violations).toEqual([]);
    await page.screenshot({ path: test.info().outputPath(`setup-${width}.png`) });
  });
}
