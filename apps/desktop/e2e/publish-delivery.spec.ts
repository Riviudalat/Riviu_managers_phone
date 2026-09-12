import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";

for (const width of [1440, 820]) {
  test(`verified publication keeps its link while Sheet retries then confirms at ${width}px`, async ({ page }, testInfo) => {
    testInfo.annotations.push({ type: "fixture", description: "FIXTURE_ONLY" });
    await page.setViewportSize({ width, height: width === 820 ? 560 : 900 });
    await installTauriMock(page);
    await page.addInitScript(() => {
      const w = window as unknown as {
        __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> };
        deliveryFixture: { calls: string[] };
      };
      const original = w.__TAURI_INTERNALS__.invoke;
      const at = "2026-09-12T00:00:00Z";
      const campaign = { id: "delivery-fixture", requestId: "delivery-request", sourceRoot: "C:/fixture",
        state: "succeeded", visibility: "public", cleanupPolicy: "keepImportedAssets", assignments: [],
        createdAt: at, updatedAt: at };
      const sent = () => sessionStorage.getItem("delivery-fixture-sent") === "true";
      w.deliveryFixture = { calls: [] };
      w.__TAURI_INTERNALS__.invoke = async (command, args) => {
        w.deliveryFixture.calls.push(command);
        if (command === "publish_list") return [campaign];
        if (command === "publish_reconcile") return { campaignId: campaign.id, inputDigest: "a".repeat(64),
          status: sent() ? "complete" : "partial", retryScope: sent() ? "none" : "sheetOnly",
          reportJson: { sheetEnabled: true }, updatedAt: at };
        if (command === "publish_get") return { campaign, bundles: [], events: [], assignments: [{
          id: "delivery-assignment", campaignId: campaign.id, bundleId: "delivery-bundle", ordinal: 0,
          udid: "MOCK-IPHONE-01", state: "succeeded", revision: 3, createdAt: at, updatedAt: at,
          evidenceJson: JSON.stringify({ post: { publicationVerified: true,
            postUrl: "https://www.tiktok.com/@fixture/video/1234567890123" }, cleanup: { state: "kept" } }),
          sheetDelivery: { state: sent() ? "sent" : "failed", attempts: sent() ? 3 : 2,
            lastError: sent() ? null : "Google tạm thời gián đoạn",
            nextAttemptAtMs: sent() ? null : Date.parse("2026-09-12T00:02:00Z"), updatedAt: at },
        }] };
        if (["publish_execute", "publish_create_campaign", "publish_cancel", "publish_sheet_save_config", "publish_sheet_prepare"].includes(command)) {
          throw new Error(`Monitor must not dispatch: ${command}`);
        }
        return original(command, args);
      };
    });

    const openMonitor = async () => {
      await page.goto("/");
      await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
      await page.getByRole("tab", { name: "Theo dõi", exact: true }).click();
      await page.getByRole("button", { name: "Chi tiết máy", exact: true }).click();
      await page.evaluate(() => document.fonts.ready);
    };
    await openMonitor();
    const link = page.getByRole("link", { name: "Mở bài đã xác nhận" });
    await expect(link).toHaveAttribute("href", "https://www.tiktok.com/@fixture/video/1234567890123");
    await expect(page.getByRole("cell", { name: /^Đang chờ ghi Sheet/ })).toBeVisible();
    await expect(page.getByText("Google tạm thời gián đoạn", { exact: true })).toBeVisible();
    await expect(page.getByText(/Đã thử 2 lần/)).toBeVisible();
    await expect(page.getByText(/Thử tiếp:/)).toBeVisible();
    await expect(page.getByText("Sheet đã xác nhận", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Ghi lại Sheet", exact: true })).toBeVisible();
    await page.getByRole("cell", { name: /^Đang chờ ghi Sheet/ }).scrollIntoViewIfNeeded();
    await page.screenshot({ path: testInfo.outputPath(`pending-${width}.png`) });
    const mutations = ["publish_execute", "publish_create_campaign", "publish_cancel", "publish_sheet_save_config", "publish_sheet_prepare"];
    expect(await page.evaluate((commands) => (window as unknown as { deliveryFixture: { calls: string[] } })
      .deliveryFixture.calls.filter(command => commands.includes(command)), mutations)).toEqual([]);

    await page.evaluate(() => sessionStorage.setItem("delivery-fixture-sent", "true"));
    await openMonitor();
    await expect(page.getByRole("cell", { name: /^Sheet đã xác nhận/ })).toBeVisible();
    await expect(link).toHaveAttribute("href", "https://www.tiktok.com/@fixture/video/1234567890123");
    await expect(page.getByText("Google tạm thời gián đoạn", { exact: true })).toHaveCount(0);
    await expect(page.getByText(/Thử tiếp:/)).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Ghi lại Sheet", exact: true })).toHaveCount(0);
    await page.getByRole("cell", { name: /^Sheet đã xác nhận/ }).scrollIntoViewIfNeeded();
    await page.screenshot({ path: testInfo.outputPath(`sent-${width}.png`) });
    expect(await page.evaluate((commands) => (window as unknown as { deliveryFixture: { calls: string[] } })
      .deliveryFixture.calls.filter(command => commands.includes(command)), mutations)).toEqual([]);
  });
}
