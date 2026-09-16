import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { openOperatorPage } from "./fixtures/operatorNavigation";
import { installTauriMock } from "./fixtures/tauriMock";

for (const width of [1440, 820]) {
  test(`stopped publication resumes only observation at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await installTauriMock(page);
    await page.addInitScript(() => {
      const w = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> }; recoveryCalls: { command: string; args: Record<string, unknown> }[] };
      const original = w.__TAURI_INTERNALS__.invoke;
      const now = new Date().toISOString();
      const campaign = { id: "stopped", requestId: "r", sourceRoot: "C:/fixture", state: "cancelled", assignments: [{ bundleId: "bundle", udid: "snapshot-phone-2", ordinal: 0 }],
        createdAt: now, updatedAt: now, visibility: "public", cleanupPolicy: "keepImportedAssets" };
      const summary = { id: "publish:stopped", sourceId: "stopped", kind: "publish", title: "Đăng bài", state: "cancelled", targetCount: 1, totalItems: 1, completedItems: 0,
        issueCount: 1, retryableCount: 0, retryScope: "none", createdAt: now, updatedAt: now };
      let resumed = false;
      let revision = 7;
      w.recoveryCalls = [];
      w.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (command === "publish_list") return [campaign];
        if (command === "operation_list_runs") return [summary];
        if (command === "operation_get_run") return { summary, items: [], batch: null };
        if (command === "publish_reconcile") return { campaignId: campaign.id, inputDigest: "d", status: "uncertain", retryScope: "none", reportJson: { sheetEnabled: true }, updatedAt: now };
        if (command === "publish_get") return { campaign, bundles: [], events: [], assignments: [{ id: "old-post", campaignId: campaign.id, bundleId: "bundle", udid: "snapshot-phone-2", ordinal: 0,
          state: resumed ? "verifying" : "uncertain", effectIntent: "post-proof", evidenceJson: JSON.stringify({ verificationStatus: resumed
            ? { state: "pending", reason: "Đã tiếp tục xác minh bài đã gửi", checkIntervalSeconds: 300, checkedAt: now, nextCheckAt: new Date(Date.parse(now) + 300000).toISOString() }
            : { state: "needsReview", reason: "Người dùng đã dừng tác vụ", cause: "operatorStopped" } }) }] };
        if (command === "publish_recovery_capabilities") return [{ assignmentId: "old-post", revision, verificationResumed: resumed,
          resumeVerification: { allowed: !resumed, reason: null }, checkLink: { allowed: resumed, reason: null }, retryBeforePost: { allowed: false, reason: "Bài đã gửi" } }];
        if (command === "publish_resume_verification") {
          w.recoveryCalls.push({ command, args }); resumed = true; revision++;
          return { assignmentId: "old-post", state: "accepted", reason: null };
        }
        if (command === "operation_stop") {
          w.recoveryCalls.push({ command, args }); resumed = false; revision++;
          return { operationId: "publish:stopped", state: "closed", devices: [] };
        }
        if (["publish_execute", "publish_create_campaign", "publish_retry_assignment"].includes(command)) {
          w.recoveryCalls.push({ command, args }); throw Error("Không được gọi Post trong recovery");
        }
        return original(command, args);
      };
    });
    await page.goto("/");
    await openOperatorPage(page, "Đăng bài");
    await page.getByRole("tab", { name: "Theo dõi", exact: true }).click();
    await page.getByRole("button", { name: "Chi tiết máy", exact: true }).click();
    const resume = page.getByRole("button", { name: /^Tiếp tục xác minh bài đã gửi/ });
    await expect(resume).toBeEnabled();
    await resume.click();
    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toContainText("Không đăng lại");
    await dialog.getByRole("button", { name: "Huỷ", exact: true }).click();
    expect(await page.evaluate(() => (window as unknown as { recoveryCalls: unknown[] }).recoveryCalls)).toEqual([]);
    await resume.click();
    await dialog.getByRole("button", { name: "Tiếp tục xác minh", exact: true }).click();
    await expect(page.getByRole("button", { name: "Dừng kiểm tra lại", exact: true })).toBeVisible();
    await expect.poll(() => page.evaluate(() => (window as unknown as { recoveryCalls: unknown[] }).recoveryCalls)).toEqual([
      { command: "publish_resume_verification", args: { assignmentId: "old-post", confirmed: true, expectedRevision: 7 } },
    ]);
    const accessibility = await new AxeBuilder({ page }).include(".publish-page").analyze();
    expect(accessibility.violations).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: `../../target/publish-recovery-20260917/monitor-${width}.png` });
    await page.getByRole("button", { name: "Dừng kiểm tra lại", exact: true }).click();
    await dialog.getByRole("button", { name: "Dừng kiểm tra", exact: true }).click();
    await expect(resume).toBeEnabled();
    await expect.poll(() => page.evaluate(() => (window as unknown as { recoveryCalls: unknown[] }).recoveryCalls)).toEqual([
      { command: "publish_resume_verification", args: { assignmentId: "old-post", confirmed: true, expectedRevision: 7 } },
      { command: "operation_stop", args: { operationId: "publish:stopped" } },
    ]);
  });
}
