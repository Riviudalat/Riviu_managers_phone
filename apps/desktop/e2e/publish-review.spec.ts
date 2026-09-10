import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installTauriMock } from "./fixtures/tauriMock";

for (const width of [1440, 820]) {
  test(`publication review stays actionable without restarting Post at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: width === 820 ? 560 : 900 });
    await installTauriMock(page);
    await page.addInitScript(() => {
      const w = window as unknown as {
        __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> };
        reviewCalls: string[];
      };
      const original = w.__TAURI_INTERNALS__.invoke;
      const now = new Date().toISOString();
      const campaign = { id: "review", requestId: "request", sourceRoot: "C:/fixture", state: "uncertain",
        errorCode: "post_verification_needs_review", assignments: [{ bundleId: "bundle", udid: "snapshot-phone-2", ordinal: 0 }],
        createdAt: now, updatedAt: now, visibility: "public", cleanupPolicy: "keepImportedAssets" };
      const summary = { id: "publish:review", sourceId: "review", kind: "publish", title: "Đăng bài", state: "uncertain",
        targetCount: 1, totalItems: 1, completedItems: 0, issueCount: 1, retryableCount: 1, retryScope: "linkAndSheet", createdAt: now, updatedAt: now };
      const detail = { campaign, bundles: [], events: [], assignments: [{ id: "assignment", campaignId: "review", bundleId: "bundle",
        udid: "snapshot-phone-2", ordinal: 0, state: "uncertain", errorCode: "post_verification_needs_review",
        evidenceJson: JSON.stringify({ verificationStatus: { state: "needsReview", reason: "Hồ sơ có bản nháp; chưa tìm thấy bài đã gửi." } }) }] };
      const snapshot = { campaignId: "review", inputDigest: "digest", status: "uncertain", retryScope: "linkAndSheet", reportJson: { sheetEnabled: true }, updatedAt: now };
      w.reviewCalls = [];
      w.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (command === "publish_list") return [campaign];
        if (command === "publish_get") return detail;
        if (command === "publish_reconcile") return snapshot;
        if (command === "operation_list_runs") return [summary];
        if (command === "operation_get_run") return { summary, items: [], batch: null };
        if (command === "publish_execute") { w.reviewCalls.push(command); return { ...snapshot, issues: [], detail }; }
        if (command === "publish_create_campaign") { w.reviewCalls.push(command); throw Error("Unexpected fresh Post campaign"); }
        return original(command, args);
      };
    });
    await page.goto("/");
    await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
    await page.getByRole("tab", { name: "Theo dõi", exact: true }).click();
    await expect(page.getByText("Cần kiểm tra bài đăng", { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Chạy lại từ đầu", exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: "Chi tiết máy", exact: true }).click();
    await expect(page.getByText("Hồ sơ có bản nháp; chưa tìm thấy bài đã gửi.")).toBeVisible();
    await expect(page.getByText("Đang chờ xác minh bài đăng", { exact: true })).toHaveCount(0);
    await page.getByRole("button",{name:/^Hoàn tất/}).click();
    await expect(page.getByRole("button",{name:"Kiểm tra liên kết",exact:true})).toHaveCount(0);
    await page.getByRole("button",{name:/^Cần xử lý/}).click();
    await expect(page.getByRole("button",{name:"Kiểm tra liên kết",exact:true})).toBeVisible();
    const accessibility = await new AxeBuilder({page}).include(".publish-page").analyze();
    expect(accessibility.violations).toEqual([]);
    await page.getByRole("region",{name:"Chi tiết chiến dịch đang chọn"}).scrollIntoViewIfNeeded();
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({ path: `../../target/publish-ux-20260909/monitor-${width}.png` });
    await page.getByRole("button", { name: "Kiểm tra liên kết", exact: true }).click();
    const confirm = page.getByRole("alertdialog");
    await expect(confirm).toContainText("Chỉ tiếp tục lấy liên kết và ghi Sheet");
    await confirm.getByRole("button", { name: "Tiếp tục", exact: true }).click();
    await expect.poll(() => page.evaluate(() => (window as unknown as { reviewCalls: string[] }).reviewCalls)).toEqual(["publish_execute"]);
  });
}
