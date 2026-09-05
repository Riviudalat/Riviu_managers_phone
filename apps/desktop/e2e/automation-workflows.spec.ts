import { expect, test, type Page } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";

async function fixture(page: Page, scenario: "interaction" | "publish") {
  await installTauriMock(page, { androidRoster: true });
  await page.addInitScript((mode) => {
    const w = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> };
      __AUTOMATION_CALLS__: { command: string; args: Record<string, unknown> }[];
    };
    const invoke = w.__TAURI_INTERNALS__.invoke;
    w.__AUTOMATION_CALLS__ = [];
    w.__TAURI_INTERNALS__.invoke = async (command, args) => {
      const url = "https://www.tiktok.com/@fixture/video/111";
      const target = { originalUrl: url, normalizedUrl: url, targetKey: "content:111", contentId: "111", author: "fixture", kind: "video" };
      const definition = { id: "profile", name: "Lưu bài đã duyệt", kind: "interaction", latestRevision: 1, archived: false, createdAt: "2026-09-05T00:00:00Z", updatedAt: "2026-09-05T00:00:00Z" };
      if (mode === "interaction") {
        if (command === "automation_list") return [definition];
        if (command === "automation_get") return { definition, revision: {
          definitionId: "profile", revision: 1, targetRef: { type: "explicit", udids: ["MOCK-ANDROID-01"] },
          config: { schemaVersion: 1, request: { targets: [target], actions: { like: false, comment: false, save: true }, mode: "standalone", messageCount: 2, maxWords: 12, instruction: "" } },
        } };
        if (command === "automation_schedule_list") return [];
        if (command === "interaction_parse_links") {
          if (String(args.rawText).includes("222")) throw new Error("Không đọc được link mới");
          return [{ lineNo: 1, original: url, target, error: null }];
        }
        if (command === "interaction_start_thread" || command === "automation_revise" || command === "interaction_measure_post") {
          w.__AUTOMATION_CALLS__.push({ command, args });
          throw new Error("fixture dispatch must remain blocked");
        }
      }
      if (mode === "publish") {
        const campaign = { id: "partial-post", requestId: "request", sourceRoot: "C:/fixture", state: "succeeded", visibility: "public", cleanupPolicy: "deleteImportedAssetsAfterVerified", assignments: [{ bundleId: "bundle", udid: "MOCK-ANDROID-01", ordinal: 0 }], createdAt: "2026-09-05T00:00:00Z", updatedAt: "2026-09-05T00:00:00Z" };
        const summary = { id: "publish:partial-post", sourceId: "partial-post", kind: "publish", title: "Đăng bài", state: "partial", targetCount: 1, totalItems: 1, completedItems: 1, issueCount: 1, retryableCount: 1, retryScope: "sheetOnly", createdAt: campaign.createdAt, updatedAt: campaign.updatedAt };
        if (command === "publish_list") return [campaign];
        if (command === "operation_list_runs") return [summary];
        if (command === "operation_get_run") return { summary, items: [] };
        if (command === "publish_reconcile") return { campaignId: campaign.id, inputDigest: "digest", status: "partial", retryScope: "sheetOnly", reportJson: {}, updatedAt: campaign.updatedAt };
        if (command === "publish_get") return { campaign, bundles: [], events: [], assignments: [{ id: "assignment", campaignId: campaign.id, bundleId: "bundle", ordinal: 0, udid: "MOCK-ANDROID-01", state: "succeeded", evidenceJson: JSON.stringify({ post: { postUrl: url, soundSelection: { title: "Nhạc đã chọn trên tài khoản", artist: "Tác giả", section: "recommended", index: 2, candidatesDigest: "verified-sound-digest", confirmed: true } }, cleanup: { state: "cleaned" } }) }] };
        if (command === "publish_execute") { w.__AUTOMATION_CALLS__.push({ command, args }); throw new Error("fixture dispatch must remain blocked"); }
      }
      return invoke(command, args);
    };
  }, scenario);
  await page.goto("/");
  await expect(page.locator("[data-testid='device-tile']")).toHaveCount(2);
}

test("saved interaction profile hydrates its exact scope and stale replacement URL never dispatches", async ({ page }) => {
  await fixture(page, "interaction");
  await page.getByRole("button", { name: "Tương tác", exact: true }).click();
  await page.getByRole("combobox", { name: "Hồ sơ Tương tác" }).selectOption("profile");
  const start = page.getByRole("button", { name: "Bắt đầu tương tác" });
  await expect(start).toBeEnabled();
  await expect(page.getByRole("checkbox", { name: "Lưu", exact: true })).toBeChecked();
  await expect(page.getByRole("checkbox", { name: "Bình luận", exact: true })).not.toBeChecked();
  await expect(page.locator(".target-selector output")).toContainText("1 máy");
  await page.getByPlaceholder("Dán link TikTok, mỗi dòng một bài").fill("https://www.tiktok.com/@fixture/video/222");
  await expect(start).toBeDisabled();
  await expect(page.getByRole("button", { name: "Lưu bản mới" })).toBeDisabled();
  await expect(page.getByText("Không đọc được link mới", { exact: true })).toBeVisible();
  await expect(start).toBeDisabled();
  expect(await page.evaluate(() => (window as unknown as { __AUTOMATION_CALLS__: unknown[] }).__AUTOMATION_CALLS__)).toEqual([]);
});

test("publish monitor keeps partial delivery actionable and shows evidence at fleet viewports", async ({ page }) => {
  await fixture(page, "publish");
  await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
  await page.getByRole("tab", { name: "Theo dõi", exact: true }).click();
  await expect(page.getByText("Hoàn tất một phần", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Ghi lại Sheet" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Chạy lại từ đầu" })).toHaveCount(0);
  await page.getByRole("button", { name: "Chi tiết máy", exact: true }).click();
  await expect(page.getByRole("link", { name: "Mở bài đã xác nhận" })).toHaveAttribute("href", "https://www.tiktok.com/@fixture/video/111");
  await expect(page.getByText("Sheet chưa hoàn tất", { exact: true })).toBeVisible();
  for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
    await page.setViewportSize(viewport);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`publish-partial-${viewport.width}.png`), fullPage: true });
  }
  expect(await page.evaluate(() => (window as unknown as { __AUTOMATION_CALLS__: unknown[] }).__AUTOMATION_CALLS__)).toEqual([]);
});
