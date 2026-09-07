import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installTauriMock } from "./fixtures/tauriMock";

for (const viewport of [
  { width: 1440, height: 900 },
  { width: 900, height: 900 },
  { width: 820, height: 560 },
]) {
  test(`production publish wizard ${viewport.width}`, async ({ page }) => {
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
    await page.locator(".automation-scope summary").click();
    await page
      .locator(".target-selector-modes label")
      .filter({ hasText: "Toàn bộ" })
      .click();
    await page.locator(".automation-scope summary").click();
    await page
      .getByRole("button", { name: "Chọn thư mục", exact: true })
      .click();
    await expect(
      page.getByRole("checkbox", { name: "Chọn Bài Đà Lạt 1", exact: true }),
    ).not.toBeChecked();
    await page.getByRole("button", { name: "Chọn nhanh", exact: true }).click();
    await page.getByRole("spinbutton", { name: "Số bài" }).fill("10");
    await page
      .getByRole("button", { name: "Chọn 10 bài", exact: true })
      .click();
    const checkLayout = async (label: string) => {
      const dimensions = await page
        .locator(".publish-wizard")
        .evaluate((root) => {
          const visible = (e: Element) => !!e.getClientRects().length;
          return {
            overflow: document.documentElement.scrollWidth > innerWidth,
            rows: [...root.querySelectorAll(".pw-page-rows,.pw-machine-grid")]
              .filter(visible)
              .filter((e) => e.scrollHeight > e.clientHeight + 1)
              .map((e) => e.className),
            offscreen: [...root.querySelectorAll("button,input,select")]
              .filter(visible)
              .filter((e) => {
                const r = e.getBoundingClientRect();
                return (
                  r.x < 0 ||
                  r.right > innerWidth + 1 ||
                  r.bottom > innerHeight + 1
                );
              })
              .map((e) => e.textContent),
          };
        });
      expect(dimensions).toEqual({ overflow: false, rows: [], offscreen: [] });
      const monitor = await page.locator(".run-monitor.is-minimized").boundingBox();
      expect(monitor).not.toBeNull();
      const before = await page.locator(".publish-wizard").boundingBox();
      const surface = page.locator(".run-monitor");
      await surface.evaluate(element => element.classList.remove("is-minimized"));
      expect(await page.locator(".publish-wizard").boundingBox()).toEqual(before);
      await surface.evaluate(element => element.classList.add("is-minimized"));
      await page.screenshot({
        path: test.info().outputPath(`${label}-${viewport.width}.png`),
      });
    };
    await checkLayout("source");
    await page.getByRole("button", { name: "Chọn máy", exact: true }).click();
    await page.getByRole("button", { name: /Ghép tự động/ }).click();
    await expect(page.locator(".pw-slot.is-filled").first()).toBeVisible();
    await checkLayout("board");
    const first = page.locator(".pw-slot.is-filled .pw-slot-body").first(),
      second = page.locator(".pw-slot.is-filled").nth(1);
    const source = await first.getAttribute("data-post-drag");
    await first.dragTo(second);
    await expect(second.locator(".pw-slot-body")).toHaveAttribute(
      "data-post-drag",
      source!,
    );
    await page.getByRole("button", { name: "Hoàn tác ghép máy" }).click();
    if (viewport.width === 820) {
      const incoming = page.locator(".pw-post-pick").first();
      const incomingId = await incoming.getAttribute("data-post-drag");
      const beforeSlot = await page
        .locator(".pw-slot")
        .first()
        .getAttribute("data-slot");
      const from = (await incoming.boundingBox())!;
      const next = (await page
        .getByRole("button", { name: "Máy nhận: trang tiếp" })
        .boundingBox())!;
      await page.mouse.move(from.x + 20, from.y + 20);
      await page.mouse.down();
      await page.mouse.move(next.x + next.width / 2, next.y + next.height / 2, {
        steps: 10,
      });
      await expect(page.locator(".pw-slot").first()).not.toHaveAttribute(
        "data-slot",
        beforeSlot!,
      );
      const destination = (await page
        .locator(".pw-slot")
        .first()
        .boundingBox())!;
      await page.mouse.move(destination.x + 30, destination.y + 45, {
        steps: 6,
      });
      await page.mouse.up();
      await expect(page.locator(".pw-slot-body").first()).toHaveAttribute(
        "data-post-drag",
        incomingId!,
      );
      await page.getByRole("button", { name: "Hoàn tác ghép máy" }).click();
    }
    await page
      .getByRole("button", { name: "Xem lại & kiểm tra", exact: true })
      .click();
    await page
      .getByRole("checkbox", { name: "Xóa ảnh đã chuyển trên máy" })
      .check();
    await checkLayout("review");
    for (const label of ["Ghi kết quả lên Sheet", "Xóa ảnh đã chuyển trên máy"]) {
      const row = await page.getByRole("checkbox", { name: label }).evaluate(input => {
        const parent = input.parentElement!;
        return { direction: getComputedStyle(parent).flexDirection, width: input.getBoundingClientRect().width, height: input.getBoundingClientRect().height };
      });
      expect(row.direction).toBe("row");
      expect(row.width).toBe(15);
      expect(row.height).toBe(15);
    }
    await page
      .getByRole("button", { name: "Kiểm tra 10 bài", exact: true })
      .click();
    await expect(
      page.getByRole("button", { name: "Xác nhận đăng 10 bài", exact: true }),
    ).toBeEnabled();
    await expect(
      page.getByText("Nhạc được chọn sau khi mở TikTok.", { exact: false }),
    ).toBeVisible();
    await page.screenshot({
      path: test.info().outputPath(`preflight-${viewport.width}.png`),
    });
    const axe = await new AxeBuilder({ page })
      .include(".publish-dialog[open]")
      .withTags(["wcag2a", "wcag2aa"])
      .analyze();
    expect(axe.violations).toEqual([]);
    await page
      .getByRole("button", { name: "Xác nhận đăng 10 bài", exact: true })
      .click();
    const publicConfirm = page.getByRole("alertdialog", {
      name: "Xác nhận đăng công khai?",
    });
    await expect(publicConfirm).toBeVisible();
    await expect(
      publicConfirm.getByRole("button", { name: "Đăng bài", exact: true }),
    ).toBeFocused();
    await publicConfirm
      .getByRole("button", { name: "Huỷ", exact: true })
      .click();
    await expect(
      page.getByRole("dialog", { name: "Xem lại trước khi bắt đầu" }),
    ).toBeVisible();
    const calls = await page.evaluate(
      () =>
        (
          window as unknown as {
            __PUBLISH_CALLS__: {
              command: string;
              args: {
                request?: {
                  udids: string[];
                  bundleIds: string[];
                  deleteAfterPublish: boolean;
                };
              };
            }[];
          }
        ).__PUBLISH_CALLS__,
    );
    expect(calls).toHaveLength(1);
    expect(calls[0].args.request?.udids).toHaveLength(10);
    expect(calls[0].args.request?.bundleIds).toHaveLength(10);
    expect(calls[0].args.request?.deleteAfterPublish).toBe(true);
    expect(errors).toEqual([]);
    // Closing/switching a workspace persists input, never campaign consent.
    await page.getByRole("dialog", { name: "Xem lại trước khi bắt đầu" }).getByRole("button", { name: "Đóng" }).click();
    await page.getByRole("button", { name: "Dữ liệu", exact: true }).click();
    await expect(page.getByRole("alertdialog")).toHaveCount(0);
    await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
    await expect(page.getByText("10 bài được chọn", { exact: true })).toBeVisible();
    await page.reload();
    await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
    await expect(page.getByText("10 bài được chọn", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Chọn máy", exact: true }).click();
    await expect(page.getByText("10 / 10 bài đã có máy", { exact: false })).toBeVisible();
  });
}
