import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";
import { openOperatorPage } from "./fixtures/operatorNavigation";

const screenshotDirectory = resolve("../../target/publish-preflight-ux-20260924");
const sheetUrl = "https://docs.google.com/spreadsheets/d/FIXTURE/edit#gid=0";

for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
  test(`publish preflight keeps Sheet and machine blockers actionable at ${viewport.width}px`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await installTauriMock(page, { androidRoster: true, fleetSize: 10 });
    await page.addInitScript(({ sheetUrl }) => {
      const host = window as unknown as {
        __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> };
        preflightVisualCalls: string[];
      };
      const original = host.__TAURI_INTERNALS__.invoke;
      host.preflightVisualCalls = [];
      host.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
        if (command === "plugin:dialog|open") return "C:/fixture-preflight";
        if (command === "publish_sheet_get_config") return {
          provider: "googleDirect", webhookUrl: "", hasToken: true, internalReporting: true, sheetUrl,
        };
        if (command === "google_sheets_status") return {
          configured: true, connected: true, active: true, clientId: "fixture.apps.googleusercontent.com",
          pickerConfigured: true, phase: "idle", error: null, email: "operator@example.test", accountId: "account-a",
          selectedFileId: "FIXTURE", selectedFileName: "Kết quả", sheetUrl, writerId: "fixture-writer", hasSheetsScope: true,
        };
        if (command === "publish_scan_folder") return {
          sourceRoot: args.sourceRoot, scannedAt: "2026-09-24T00:00:00Z", notices: [], ignoredPartnerFiles: 0, ignoredHiddenFiles: 0,
          bundles: Array.from({ length: 10 }, (_, index) => ({
            id: `bundle-${index + 1}`, name: `Bài Đà Lạt ${index + 1}`, sourcePath: `C:/fixture-preflight/${index + 1}`,
            mediaKind: "image", images: [{ path: `C:/fixture-preflight/${index + 1}/1.png`, fileName: "1.png",
              order: 0, sha256: "a".repeat(64), byteLen: 100, width: 100, height: 100 }],
            captionPath: "caption.txt", caption: `Nội dung bài ${index + 1}`, captionSha256: "b".repeat(64), totalBytes: 100,
          })),
        };
        if (command === "publish_image_preview") return "data:image/svg+xml," + encodeURIComponent(
          '<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="100" height="100" fill="#bfd8c7"/></svg>',
        );
        if (command === "publish_preflight") {
          host.preflightVisualCalls.push(command);
          const request = args.request as { bundleIds: string[]; udids: string[]; targetRef: unknown };
          const soundIssue = { code: "sound_picker_unmeasured", udid: request.udids[9], message: "fixture: picker chưa được đo" };
          const sheetIssue = { code: "sheet_connection_unverified", message: "fixture: mất quyền ghi Sheet sau khi liên kết" };
          return {
            inputDigest: "fixture-digest", sheetEnabled: true, sheetConfigured: true, canExecute: false,
            targetSnapshot: { targetRef: request.targetRef, included: request.udids.map((udid, index) => ({ udid, number: index + 1, alias: "" })),
              excluded: [], rosterSha256: "c".repeat(64) },
            assignments: request.bundleIds.map((id, index) => ({ ordinal: index, bundleId: id, udid: request.udids[index],
              packageName: "com.zhiliaoapp.musically", version: "45.7.3", locale: "en-US", media: "pass", composer: "pass",
              soundPicker: index === 9 ? "fail" : "pass", storage: "pass", requiredBytes: 100, availableBytes: 99999,
              issues: index === 9 ? [soundIssue] : [] })),
            issues: [sheetIssue, soundIssue],
          };
        }
        if (command === "publish_create_campaign" || command === "publish_execute" || command === "publish_sheet_check") {
          host.preflightVisualCalls.push(command);
          throw Error(`Unexpected public or remote Sheet action in visual fixture: ${command}`);
        }
        return original(command, args);
      };
    }, { sheetUrl });

    await page.goto("/");
    await openOperatorPage(page, "Đăng bài");
    await page.getByRole("combobox", { name: "Phạm vi thiết bị" }).selectOption("all");
    await page.getByRole("button", { name: "Chọn thư mục", exact: true }).click();
    await page.getByRole("button", { name: "Quét", exact: true }).click();
    await page.getByRole("button", { name: "Chọn nhanh", exact: true }).click();
    await expect(page.locator(".pq-footer")).toContainText("10/10 bài có máy");
    await expect(page.locator(".publish-sheet-result.is-linked")).toContainText("Đã liên kết Google Sheet");
    await expect(page.locator(".pq-run-options")).toHaveText("Sheet bật · Dọn bản chuyển bật");
    await expect(page.getByRole("checkbox", { name: "Ghi kết quả lên Sheet" })).toHaveCount(0);
    await page.getByRole("button", { name: "Kiểm tra & đăng", exact: true }).click();

    const dialog = page.getByRole("dialog", { name: "Kiểm tra đợt đăng" });
    await expect(dialog.getByRole("region", { name: "Điều kiện chung chưa đạt" })).toContainText("Kết nối Sheet chưa được xác minh");
    await expect(dialog.getByRole("article", { name: /^Máy 10 ·/ })).toBeVisible();
    await expect(dialog.getByRole("article", { name: /^Máy 1 ·/ })).toHaveCount(0);
    await expect(dialog.getByRole("button", { name: "Xác nhận đăng 10 bài" })).toBeDisabled();

    const geometry = await dialog.evaluate((node) => {
      const box = (element: Element) => { const r = element.getBoundingClientRect(); return { top: r.top, right: r.right, bottom: r.bottom, left: r.left }; };
      const header = node.querySelector(":scope > header")!, body = node.querySelector(":scope > .publish-dialog-body")!;
      const footer = node.querySelector(":scope > footer")!, global = node.querySelector(".pw-preflight-global")!;
      const machineList = node.querySelector(".pw-check-results")!;
      return { dialog: box(node), header: box(header), body: box(body), footer: box(footer),
        global: box(global), machineList: box(machineList), bodyScrollWidth: body.scrollWidth, bodyClientWidth: body.clientWidth,
        viewportWidth: innerWidth, viewportHeight: innerHeight, pageScrollWidth: document.documentElement.scrollWidth };
    });
    expect(geometry.dialog.left, JSON.stringify(geometry)).toBeGreaterThanOrEqual(0);
    expect(geometry.dialog.right, JSON.stringify(geometry)).toBeLessThanOrEqual(geometry.viewportWidth);
    expect(geometry.dialog.bottom, JSON.stringify(geometry)).toBeLessThanOrEqual(geometry.viewportHeight);
    expect(geometry.header.bottom, JSON.stringify(geometry)).toBeLessThanOrEqual(geometry.body.top + 1);
    expect(geometry.body.bottom, JSON.stringify(geometry)).toBeLessThanOrEqual(geometry.footer.top + 1);
    expect(geometry.global.bottom, JSON.stringify(geometry)).toBeLessThanOrEqual(geometry.machineList.top + 1);
    expect(geometry.bodyScrollWidth, JSON.stringify(geometry)).toBeLessThanOrEqual(geometry.bodyClientWidth + 1);
    expect(geometry.pageScrollWidth, JSON.stringify(geometry)).toBeLessThanOrEqual(geometry.viewportWidth);
    await expect(dialog.getByRole("region", { name: "Điều kiện chung chưa đạt" })).toBeInViewport();
    await expect(dialog.getByRole("button", { name: "Kiểm tra lại" })).toBeInViewport();
    await mkdir(screenshotDirectory, { recursive: true });
    await page.screenshot({ path: resolve(screenshotDirectory, `preflight-${viewport.width}x${viewport.height}.png`) });

    await dialog.getByRole("button", { name: "Đạt (9)" }).click();
    await expect(dialog.getByRole("article", { name: /^Máy 1 ·/ })).toBeVisible();
    await dialog.getByRole("button", { name: "Cần xử lý (1)" }).click();
    await expect(dialog.getByRole("article", { name: /^Máy 10 ·/ })).toBeVisible();
    expect(await page.evaluate(() => (window as unknown as { preflightVisualCalls: string[] }).preflightVisualCalls)).toEqual(["publish_preflight"]);
  });
}
