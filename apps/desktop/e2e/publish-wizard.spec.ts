import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installTauriMock } from "./fixtures/tauriMock";

for (const viewport of [
  { width: 1440, height: 900 },
  { width: 900, height: 900 },
  { width: 820, height: 560 },
]) {
  test(`production publish quick workspace ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await installTauriMock(page, { androidRoster: true, fleetSize: 20 });
    await page.addInitScript(() => {
      const w = window as unknown as {
        __TAURI_INTERNALS__: {
          invoke: (
            command: string,
            args: Record<string, unknown>,
          ) => Promise<unknown>;
        };
        __PUBLISH_CALLS__: { command: string; args: Record<string, unknown> }[];
      };
      const invoke = w.__TAURI_INTERNALS__.invoke;
      w.__PUBLISH_CALLS__ = [];
      w.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (command === "startup_error") return null;
        if (command === "android_tool_problems") return [];
        if (command === "operation_query_runs") return {
          total: 1, offset: 0, limit: 200,
          counts: { active: 0, succeeded: 1, attention: 0 }, hasMore: false,
          runs: [{ id: "publish:finished", sourceId: "finished", kind: "publish", title: "Đăng bài", state: "succeeded", targetCount: 1, totalItems: 1, completedItems: 1, issueCount: 0, retryableCount: 0, retryScope: "none", createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() }],
        };
        if (command === "plugin:dialog|open") return "C:/Bài đăng";
        if (command === "publish_scan_folder")
          return {
            sourceRoot: args.sourceRoot,
            scannedAt: "2026-09-07T00:00:00Z",
            notices: [],
            ignoredPartnerFiles: 0,
            ignoredHiddenFiles: 0,
            bundles: Array.from({ length: 10 }, (_, i) => ({
              id: `bundle-${i + 1}`,
              name: `Bài Đà Lạt ${i + 1}`,
              sourcePath: `C:/Bài đăng/Bài ${i + 1}`,
              mediaKind: "image",
              images: Array.from({ length: 5 }, (_, j) => ({
                path: `C:/Bài đăng/Bài ${i + 1}/${j + 1}.png`,
                fileName: `${j + 1}.png`,
                order: j,
                sha256: "a".repeat(64),
                byteLen: 100,
                width: 100,
                height: 100,
              })),
              captionPath: "caption.txt",
              caption: `Nội dung bài ${i + 1}`,
              captionSha256: "b".repeat(64),
              totalBytes: 500,
            })),
          };
        if (command === "publish_image_preview")
          return (
            "data:image/svg+xml," +
            encodeURIComponent(
              '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="160"><rect width="120" height="160" fill="#b9d8c1"/><path d="M0 130 40 45 65 90 87 55 120 130" fill="#608771"/><text x="14" y="22" font-size="14">Đà Lạt</text></svg>',
            )
          );
        if (command === "publish_preflight") {
          w.__PUBLISH_CALLS__.push({ command, args });
          const r = args.request as {
            bundleIds: string[];
            udids: string[];
            targetRef: unknown;
          };
          return {
            inputDigest: "digest",
            sheetEnabled: false,
            sheetConfigured: false,
            canExecute: true,
            targetSnapshot: {
              targetRef: r.targetRef,
              included: r.udids.map((udid) => ({ udid, alias: "" })),
              excluded: [],
              rosterSha256: "c".repeat(64),
            },
            assignments: r.bundleIds.map((id, i) => ({
              ordinal: i,
              bundleId: id,
              udid: r.udids[i],
              media: "pass",
              composer: "pass",
              soundPicker: "pass",
              storage: "pass",
              requiredBytes: 100,
              availableBytes: 99999,
              issues: [],
            })),
            issues: [],
          };
        }
        if (
          command === "publish_create_campaign" ||
          command === "publish_execute"
        ) {
          w.__PUBLISH_CALLS__.push({ command, args });
          throw new Error("No public action in layout gate");
        }
        return invoke(command, args);
      };
    });
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto("/");
    await expect(page.getByTestId("device-tile")).toHaveCount(20);
    await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
    await page.getByRole("combobox", { name: "Phạm vi thiết bị" }).selectOption("all");
    await page
      .getByRole("button", { name: "Chọn thư mục", exact: true })
      .click();
    await page.getByRole("button", { name: "Quét", exact: true }).click();
    await expect(
      page.getByRole("checkbox", { name: "Chọn Bài Đà Lạt 1", exact: true }),
    ).not.toBeChecked();

    await page.getByRole("button", { name: "Chọn tất cả bài", exact: true }).click();
    await expect(page.getByRole("textbox", { name: "Nội dung bài đăng", exact: true })).toHaveValue("Nội dung bài 1");
    const checkLayout = async (label: string) => {
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      const root = page.locator(".publish-quick");
      const before = await root.boundingBox();
      const surface = page.locator(".run-monitor");
      await surface.evaluate(element => element.classList.remove("is-minimized"));
      expect(await root.boundingBox()).toEqual(before);
      await surface.evaluate(element => element.classList.add("is-minimized"));
      const footer = root.locator(".pq-footer");
      const box = await footer.boundingBox();
      expect(box!.x).toBeGreaterThanOrEqual(0);
      expect(box!.x + box!.width).toBeLessThanOrEqual(viewport.width);
      expect(box!.y + box!.height).toBeLessThanOrEqual(viewport.height);
      await page.screenshot({ path: test.info().outputPath(`${label}-${viewport.width}.png`) });
    };
    await checkLayout("source");
    await page.getByRole("button", { name: "Phóng to ảnh", exact: true }).click();
    const imagePreview = page.getByRole("dialog", { name: "Xem trước · Bài Đà Lạt 1" });
    await expect(imagePreview).toBeVisible();
    await expect(imagePreview.getByRole("img")).toHaveAttribute("alt", "1.png");
    await imagePreview.getByRole("button", { name: "Ảnh tiếp", exact: true }).click();
    await expect(imagePreview.getByRole("img")).toHaveAttribute("alt", "2.png");
    await expect(imagePreview.getByText("2 / 5", { exact: true })).toBeVisible();
    const previewDimensions = await imagePreview.boundingBox();
    expect(previewDimensions!.x).toBeGreaterThanOrEqual(0);
    expect(previewDimensions!.y).toBeGreaterThanOrEqual(0);
    expect(previewDimensions!.x + previewDimensions!.width).toBeLessThanOrEqual(viewport.width);
    expect(previewDimensions!.y + previewDimensions!.height).toBeLessThanOrEqual(viewport.height);
    await page.keyboard.press("Escape");
    await expect(imagePreview).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Phóng to ảnh", exact: true })).toBeFocused();
    const machines = page.getByRole("region", { name: "Máy thực hiện", exact: true });
    await expect(machines.getByRole("checkbox")).toHaveCount(20);
    expect(await machines.locator(".pq-machine-grid").evaluate(element => getComputedStyle(element).gridTemplateColumns.split(" ").length)).toBe(2);
    await machines.getByRole("button", { name: "Chọn tất cả", exact: true }).click();
    await page.getByRole("button", { name: "Chọn tất cả bài", exact: true }).click();
    await page.getByRole("button", { name: "Chọn nhanh", exact: true }).click();
    await expect(page.locator(".pq-footer")).toContainText("10/10 bài có máy");
    await checkLayout("board");
    // Swapping the active post keeps the other post assigned to the displaced phone.
    const assignmentSelect = page.getByRole("combobox", { name: "Máy nhận bài đang chỉnh" });
    const firstPhone = await assignmentSelect.inputValue();
    const secondPhone = await assignmentSelect.locator("option").nth(2).getAttribute("value");
    await assignmentSelect.selectOption(secondPhone!);
    await expect(assignmentSelect).toHaveValue(secondPhone!);
    await assignmentSelect.selectOption(firstPhone);
    await machines.getByRole("button", { name: "Bỏ chọn", exact: true }).click();
    await expect(machines.getByRole("checkbox", { checked: true })).toHaveCount(0);
    await machines.getByRole("checkbox", { name: /Chọn Máy 20 ·/ }).check();
    await expect(machines.getByRole("checkbox", { checked: true })).toHaveCount(1);
    await page.getByRole("button", { name: "Chọn nhanh", exact: true }).click();
    await expect(machines.getByRole("status")).toContainText("Đã gán 1 bài cho 1 máy");
    await expect(page.locator(".pq-footer")).toContainText("1/1 bài có máy");
    await page.getByRole("button", { name: "Chọn tất cả bài", exact: true }).click();
    await machines.getByRole("button", { name: "Chọn tất cả", exact: true }).click();
    await page.getByRole("button", { name: "Chọn nhanh", exact: true }).click();
    await expect(page.locator(".pq-footer")).toContainText("10/10 bài có máy");
    await page.getByRole("tab", { name: "Hẹn giờ", exact: true }).click();
    await expect(page.getByRole("tabpanel", { name: "Hẹn giờ", exact: true })).toBeVisible();
    await page.getByRole("tab", { name: "Thiết lập", exact: true }).click();
    await expect(page.locator(".pq-footer")).toContainText("10/10 bài có máy");
    await page.getByRole("checkbox", { name: "Xóa bản chuyển sau khi đăng thành công" }).check();
    for (const label of ["Ghi kết quả lên Sheet", "Xóa bản chuyển sau khi đăng thành công"]) {
      const row = await page.getByRole("checkbox", { name: label }).evaluate(input => {
        const parent = input.parentElement!;
        return { direction: getComputedStyle(parent).flexDirection, width: input.getBoundingClientRect().width, height: input.getBoundingClientRect().height };
      });
      expect(row).toEqual({ direction: "row", width: 15, height: 15 });
    }
    await page.getByRole("button", { name: "Kiểm tra & đăng", exact: true }).click();
    await expect(page.getByRole("button", { name: "Xác nhận đăng 10 bài", exact: true })).toBeEnabled();
    await expect(page.getByText("Nhạc được chọn sau khi mở TikTok.", { exact: false })).toBeVisible();
    await page.screenshot({ path: test.info().outputPath(`preflight-${viewport.width}.png`) });
    const axe = await new AxeBuilder({ page }).include(".publish-dialog[open]").withTags(["wcag2a", "wcag2aa"]).analyze();
    expect(axe.violations).toEqual([]);
    await page.getByRole("button", { name: "Xác nhận đăng 10 bài", exact: true }).click();
    const publicConfirm = page.getByRole("alertdialog", { name: "Xác nhận đăng công khai?" });
    await expect(publicConfirm).toBeVisible();
    await expect(publicConfirm.getByRole("button", { name: "Đăng bài", exact: true })).toBeFocused();
    await publicConfirm.getByRole("button", { name: "Huỷ", exact: true }).click();
    const checkDialog = page.getByRole("dialog", { name: "Kiểm tra đợt đăng" });
    await expect(checkDialog).toBeVisible();
    const calls = await page.evaluate(() => (window as unknown as { __PUBLISH_CALLS__: { command: string; args: { request?: { udids: string[]; bundleIds: string[]; deleteAfterPublish: boolean } } }[] }).__PUBLISH_CALLS__);
    expect(calls).toHaveLength(1);
    expect(calls[0].args.request?.udids).toHaveLength(10);
    expect(calls[0].args.request?.bundleIds).toHaveLength(10);
    expect(calls[0].args.request?.deleteAfterPublish).toBe(true);
    expect(errors).toEqual([]);
    await checkDialog.getByRole("button", { name: "Đóng" }).click();
    await page.getByRole("button", { name: "Dữ liệu", exact: true }).click();
    await expect(page.getByRole("alertdialog")).toHaveCount(0);
    await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
    await expect(page.locator(".pq-footer")).toContainText("10/10 bài có máy");
    await page.reload();
    await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
    await expect(page.locator(".pq-footer")).toContainText("10/10 bài có máy");
  });
}
