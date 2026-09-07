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
    const deviceHandles: Record<string, string> = {};
    w.__AUTOMATION_CALLS__ = [];
    w.__TAURI_INTERNALS__.invoke = async (command, args) => {
      const url = "https://www.tiktok.com/@fixture/video/111";
      const target = { originalUrl: url, normalizedUrl: url, targetKey: "content:111", contentId: "111", author: "fixture", kind: "video" };
      const definition = { id: "profile", name: "Lưu bài đã duyệt", kind: "interaction", latestRevision: 1, archived: false, createdAt: "2026-09-05T00:00:00Z", updatedAt: "2026-09-05T00:00:00Z" };
      if (mode === "interaction") {
        if (command === "interaction_preview_thread") {
          const request = args.request as { requestId: string; actorUdids: string[]; targets: typeof target[] };
          return { lines: request.targets.map((target, index) => ({ lineNo: index + 1, original: target.normalizedUrl, target, error: null })), validTargetCount: request.targets.length, cohortCount: 1, streamCapacity: 8, plan: { requestId: request.requestId, assignments: request.targets.flatMap((target) => request.actorUdids.map((actorUdid, ordinal) => ({ actorUdid, ordinal, targetKey: target.targetKey, parentOrdinal: null, cohort: 0 }))) } };
        }
        if (command === "interaction_read_account") return { udid: args.udid, expectedHandle: deviceHandles[String(args.udid)] ?? "", observedHandle: "test.account", status: "matched", checkedAt: "2026-09-06T00:00:00Z", snapshotSha256: "account-proof" };
        if (command === "interaction_import_sheet") return { sourceUrl: args.sheetUrl, column: "D", digest: "sheet-proof", rows: [{ row: 2, line: { lineNo: 2, original: url, target, error: null }, duplicateOf: null }, { row: 3, line: { lineNo: 3, original: url, target, error: null }, duplicateOf: 2 }] };
        if (command === "get_device_meta") return { udid: args.udid, handle: deviceHandles[String(args.udid)] ?? "", notes: "", tags: [], groupId: null, alias: "", number: null };
        if (command === "save_device_handle") {
          const udid = String(args.udid);
          if ((deviceHandles[udid] ?? "") !== args.expectedHandle) throw new Error("mapping changed");
          deviceHandles[udid] = String(args.handle);
          w.__AUTOMATION_CALLS__.push({ command, args });
          return deviceHandles[udid];
        }
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

test("interaction action-only setup fits desktop and narrow layouts without hidden comment fields", async ({ page }) => {
  await fixture(page,"interaction");
  await page.getByRole("button",{name:"Tương tác",exact:true}).click();
  await page.getByRole("combobox",{name:"Hồ sơ Tương tác"}).selectOption("profile");
  await expect(page.getByRole("checkbox",{name:"Lưu",exact:true})).toBeChecked();
  await expect(page.getByRole("radiogroup",{name:"Kiểu tương tác"})).toHaveCount(0);
  await expect(page.getByRole("button",{name:"Bắt đầu tương tác"})).toBeEnabled();
  for(const viewport of [{width:1440,height:900},{width:820,height:560}]) {
    await page.setViewportSize(viewport);
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({path:test.info().outputPath(`interaction-save-${viewport.width}.png`),fullPage:true});
  }
  expect(await page.evaluate(()=>(window as unknown as {__AUTOMATION_CALLS__:unknown[]}).__AUTOMATION_CALLS__)).toEqual([]);
});

test("interaction maps nick by device and keeps duplicate errors inline across viewports", async ({ page }) => {
  await fixture(page, "interaction");
  await page.getByRole("button", { name: "Tương tác", exact: true }).click();
  await page.getByRole("radiogroup", { name: "Cách chọn thiết bị" }).getByText("Toàn bộ", { exact: true }).click();
  const inputs = page.getByPlaceholder("@handle");
  await expect(inputs).toHaveCount(2);
  await inputs.nth(0).fill("@test.account");
  await inputs.nth(0).press("Tab");
  await expect(inputs.nth(0)).toHaveValue("test.account");
  await expect.poll(async () => await page.evaluate(() => (window as unknown as { __AUTOMATION_CALLS__: { command: string }[] }).__AUTOMATION_CALLS__.filter((c) => c.command === "save_device_handle").length)).toBe(1);
  await inputs.nth(1).fill("TEST.account");
  await inputs.nth(1).press("Tab");
  await expect(inputs.nth(1)).toHaveAttribute("aria-invalid", "true");
  await expect(page.getByRole("alert").filter({ hasText: "Nick này đang gán cho máy khác" })).toBeVisible();
  for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
    await page.setViewportSize(viewport);
    await page.getByRole("button", { name: "Tải lại nick đã lưu" }).scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`interaction-handles-${viewport.width}.png`), fullPage: true });
  }
  await page.getByRole("button", { name: "Tải lại nick đã lưu" }).click();
  await expect(inputs.nth(1)).toHaveValue("");
  await expect(inputs.nth(1)).toHaveAttribute("aria-invalid", "false");
});

test("interaction reads Sheet selections and account proof without dispatching", async ({ page }) => {
  await fixture(page, "interaction");
  await page.getByRole("button", { name: "Tương tác", exact: true }).click();
  await page.getByRole("radiogroup", { name: "Cách chọn thiết bị" }).getByText("Toàn bộ", { exact: true }).click();
  await page.getByText("Nhập từ Google Sheet", { exact: true }).click();
  await page.getByLabel("Link Sheet", { exact: true }).fill("https://docs.google.com/spreadsheets/d/fixture/edit#gid=42");
  await page.getByRole("button", { name: "Đọc Sheet", exact: true }).click();
  await expect(page.getByLabel("Chọn dòng 3")).toBeDisabled();
  await page.getByLabel("Chọn dòng 2").check();
  for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
    await page.setViewportSize(viewport);
    await page.getByLabel("Chọn dòng 2").scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`interaction-sheet-${viewport.width}.png`) });
  }
  await page.getByRole("button", { name: "Thêm 1 bài đã chọn" }).click();
  await expect(page.getByPlaceholder("Dán link TikTok, mỗi dòng một bài")).toHaveValue("https://www.tiktok.com/@fixture/video/111");
  const input = page.getByPlaceholder("@handle").first();
  await input.fill("test.account"); await input.press("Tab");
  await page.getByRole("button", { name: "Đọc tài khoản từ máy", exact: true }).first().click();
  await expect(page.getByText("Khớp tài khoản · @test.account")).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __AUTOMATION_CALLS__: { command: string }[] }).__AUTOMATION_CALLS__.filter((call) => call.command !== "save_device_handle"))).toEqual([]);
});

test("publish monitor keeps partial delivery actionable and shows evidence at fleet viewports", async ({ page }) => {
  await fixture(page, "publish");
  await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
  await page.getByRole("button", { name: "Theo dõi", exact: true }).click();
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

test("publish Sheet toggle is keyboard accessible and fits both workspace sizes", async ({ page }) => {
  await installTauriMock(page, { androidRoster: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
  await page.getByRole("button", { name: "Hồ sơ & cài đặt" }).click();
  const toggle = page.getByRole("checkbox", { name: "Ghi kết quả lên Sheet" });
  await expect(toggle).not.toBeChecked();
  await toggle.focus();
  await page.keyboard.press("Space");
  await expect(toggle).toBeChecked();
  await toggle.focus();
  await page.keyboard.press("Space");
  await expect(toggle).not.toBeChecked();
  await expect(page.getByText("Sheet chờ cấu hình", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Không ghi Sheet", { exact: true })).toBeVisible();
  for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
    await page.setViewportSize(viewport);
    await toggle.scrollIntoViewIfNeeded();
    const geometry = await toggle.evaluate((input) => {
      const box = input.getBoundingClientRect();
      const label = input.parentElement!.querySelector("span")!.getBoundingClientRect();
      return { width: box.width, x: box.right, labelX: label.left, deltaY: Math.abs(box.top + box.height / 2 - label.top - label.height / 2) };
    });
    expect(geometry.width).toBe(18);
    expect(geometry.labelX).toBeGreaterThan(geometry.x);
    expect(geometry.deltaY).toBeLessThan(2);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`publish-sheet-off-${viewport.width}.png`), fullPage: true });
  }
});
