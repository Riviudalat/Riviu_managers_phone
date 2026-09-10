import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installTauriMock } from "./fixtures/tauriMock";

for (const width of [1440, 820]) {
  test(`inline Sheet check and source controls at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: width === 820 ? 560 : 900 });
    await installTauriMock(page);
    await page.addInitScript(() => {
      const w = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> }; sheetCalls: {command:string; args:Record<string, unknown>}[] };
      const original = w.__TAURI_INTERNALS__.invoke;
      w.sheetCalls = [];
      w.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (command === "publish_sheet_get_config") return { webhookUrl: "https://script.google.com/fixture/exec", hasToken: true, internalReporting: true, sheetUrl: "https://docs.google.com/spreadsheets/d/SAVED/edit" };
        if (command === "publish_sheet_prepare") {
          w.sheetCalls.push({command,args});
          const verified = String(args.sheetUrl).includes("VERIFIED");
          return {sheetUrl:args.sheetUrl, spreadsheetId:verified ? "VERIFIED" : "READABLE", sheetGid:0, readable:true, connectionVerified:verified, layout:"internal", columns:["Máy","Tài khoản TikTok","Trạng thái","Lỗi hoặc ghi chú","Đối tác 1"], message:verified ? "Đã xác minh kết nối đúng Sheet." : "Đọc được Sheet; chưa xác minh kết nối ghi kết quả."};
        }
        if (command === "publish_sheet_save_config" || command === "publish_execute" || command === "automation_profile_create") {
          w.sheetCalls.push({command,args});
          throw new Error("Unexpected mutation during connection check");
        }
        return original(command, args);
      };
    });
    await page.goto("/");
    await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
    await expect(page.getByRole("tablist", {name:"Chế độ Đăng bài"}).getByRole("tab")).toHaveText(["Thiết lập","Hẹn giờ","Theo dõi"]);
    await expect(page.getByRole("button",{name:/Hồ sơ|Nhập nội dung/})).toHaveCount(0);
    const controls = page.locator(".pq-source-controls");
    await expect(controls.getByRole("button",{name:"Chọn thư mục",exact:true})).toBeVisible();
    await expect(controls.getByRole("button",{name:"Quét",exact:true})).toBeVisible();
    await expect(page.getByRole("button",{name:"Chọn thư mục",exact:true})).toHaveCount(1);
    const link = page.getByRole("textbox",{name:"Link Google Sheet"});
    const check = page.getByRole("button",{name:"Kết nối Sheet",exact:true});
    await expect(link).toHaveValue("https://docs.google.com/spreadsheets/d/SAVED/edit");
    await expect(link).toBeEnabled();
    await link.fill("");
    await expect(check).toBeDisabled();
    await link.fill("https://docs.google.com/spreadsheets/d/READABLE/edit");
    await check.click();
    await expect(page.locator(".publish-sheet-result")).toHaveText("Đọc được Sheet; chưa xác minh kết nối ghi kết quả.");
    await expect(page.locator(".publish-sheet-result")).not.toHaveClass(/is-verified/);
    await link.fill("https://docs.google.com/spreadsheets/d/VERIFIED/edit");
    await expect(page.locator(".publish-sheet-result")).toHaveCount(0);
    await link.press("Enter");
    await expect(page.locator(".publish-sheet-result")).toHaveClass(/is-verified/);
    await expect(page.locator(".publish-sheet-result")).toHaveText("Đã xác minh kết nối đúng Sheet.");
    expect(await page.evaluate(() => (window as unknown as {sheetCalls:{command:string}[]}).sheetCalls.map(c=>c.command))).toEqual(["publish_sheet_prepare","publish_sheet_prepare","publish_sheet_prepare"]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const box = await page.locator(".pq-footer").boundingBox();
    expect(box!.y+box!.height).toBeLessThanOrEqual(width === 820 ? 560 : 900);
    const accessibility = await new AxeBuilder({page}).include(".publish-page").analyze();
    expect(accessibility.violations).toEqual([]);
    await page.screenshot({path:test.info().outputPath(`setup-${width}.png`)});
  });
}
