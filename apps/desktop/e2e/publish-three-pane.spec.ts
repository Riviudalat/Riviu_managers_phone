import { expect, test, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installTauriMock } from "./fixtures/tauriMock";
import { openOperatorPage } from "./fixtures/operatorNavigation";

async function openBoard(page: Page, options: { fleet?: number; posts?: number; longNames?: boolean; googleError?: boolean } = {}) {
  await installTauriMock(page, { androidRoster: true, fleetSize: options.fleet ?? 20 });
  await page.addInitScript(({ posts, longNames, googleError }) => {
    const w = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> };
      publishBoardCalls: string[];
    };
    const original = w.__TAURI_INTERNALS__.invoke;
    w.publishBoardCalls = [];
    w.__TAURI_INTERNALS__.invoke = async (cmd, args = {}) => {
      if (cmd === "startup_error") return null;
      if (cmd === "android_tool_problems") return [];
      if (cmd === "plugin:dialog|open") return "C:/fixture-three-pane";
      if (cmd === "google_sheets_status") {
        if (googleError) throw { code: "OperationFailed", message: "Không đọc được trạng thái Google; hãy thử lại." };
        return { configured: true, connected: false, active: false, clientId: "fixture.apps.googleusercontent.com", pickerConfigured: true, phase: "idle" };
      }
      if (cmd === "publish_scan_folder") return {
        sourceRoot: args.sourceRoot, scannedAt: "2026-09-17T00:00:00Z", notices: [], ignoredPartnerFiles: 0, ignoredHiddenFiles: 0,
        bundles: Array.from({ length: posts }, (_, i) => ({ id: `post-${i + 1}`, name: longNames ? `Bài ${i + 1} với tên rất dài để kiểm chứng nội dung nguồn không làm tràn bố cục ba khung` : `Bài ${i + 1}`,
          sourcePath: `C:/fixture-three-pane/${i}`, mediaKind: "image", images: [1, 2, 3].map(n => ({ path: `${i}/${n}.png`, fileName: `Ảnh ${n}.png`, order: n - 1, sha256: `${i}-${n}`, byteLen: 100, width: 100, height: 100 })),
          captionPath: `${i}/caption.txt`, caption: `Caption bài ${i + 1}`, captionSha256: `caption-${i}`, totalBytes: 300, partners: [`Đối tác ${i + 1}`] })),
      };
      if (cmd === "publish_image_preview") return "data:image/svg+xml," + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="100" height="100" fill="#cfe2d2"/></svg>');
      if (cmd === "publish_create_campaign" || cmd === "publish_execute" || cmd === "publish_preflight" || cmd.startsWith("google_sheets_") || cmd === "publish_sheet_prepare" || cmd === "publish_sheet_save_config") {
        w.publishBoardCalls.push(cmd); throw Error(`Không có tác vụ thật trong phép kiểm ba khung: ${cmd}`);
      }
      return original(cmd, args);
    };
  }, { posts: options.posts ?? 10, longNames: options.longNames ?? false, googleError: options.googleError ?? false });
  await page.goto("/"); await openOperatorPage(page, "Đăng bài");
  await page.getByRole("combobox", { name: "Phạm vi thiết bị", exact: true }).selectOption("all");
  await page.getByRole("button", { name: "Chọn thư mục", exact: true }).click();
  await page.getByRole("button", { name: "Quét", exact: true }).click();
  await expect(page.locator(".pq-posts article")).toHaveCount(options.posts ?? 10);
}

async function measureBoard(page: Page) {
  return page.evaluate(() => {
    const rect = (selector: string) => { const r = document.querySelector(selector)!.getBoundingClientRect(); return { x: r.x, y: r.y, right: r.right, bottom: r.bottom, width: r.width, height: r.height }; };
    return { panels: [".pq-library", ".pq-mapping", ".pq-devices"].map(rect), roster: rect(".pq-machine-grid"), footer: rect(".pq-footer"), search: rect('.pq-devices .pq-search'),
      ancestors: Object.fromEntries([".publish-page", "#publish-panel-setup", ".pq-setup-tools", ".pq-columns", ".pq-device-tools", ".pq-active-post"].map(selector => [selector, rect(selector)])),
      rows: [...document.querySelectorAll(".pq-link")].map(node => node.getBoundingClientRect().height),
      fullyVisibleDevices: [...document.querySelectorAll(".pq-device-row")].filter(node => { const r = node.getBoundingClientRect(), list = rect(".pq-machine-grid"); return r.y >= list.y && r.bottom <= list.bottom; }).length,
      overflow: document.documentElement.scrollWidth > innerWidth };
  });
}

for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
  test(`publish device scope and picker remain readable at ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await openBoard(page);
    const geometry = await page.locator(".pq-devices-header").evaluate(header => {
      const select = header.querySelector<HTMLSelectElement>(".machine-scope-select")!;
      const pick = header.querySelector<HTMLButtonElement>(".machine-scope-pick")!;
      const title = header.querySelector<HTMLHeadingElement>("h2")!;
      const context = header.querySelector<HTMLElement>(".pq-active-post")!;
      const rect = (element: Element) => {
        const box = element.getBoundingClientRect();
        return { left: box.left, right: box.right, top: box.top, bottom: box.bottom, width: box.width };
      };
      const style = getComputedStyle(select);
      const canvas = document.createElement("canvas");
      const measure = canvas.getContext("2d")!;
      measure.font = style.font;
      const labelWidth = measure.measureText(select.selectedOptions[0].text).width;
      const padding = parseFloat(style.paddingLeft) + parseFloat(style.paddingRight);
      return { header: rect(header), select: rect(select), pick: rect(pick), title: rect(title), context: rect(context),
        labelWidth, padding };
    });
    await test.info().attach("Publish device header geometry", { body: JSON.stringify(geometry, null, 2), contentType: "application/json" });
    expect(geometry.select.width, JSON.stringify(geometry)).toBeGreaterThanOrEqual(geometry.labelWidth + geometry.padding + 20);
    expect(geometry.select.right, JSON.stringify(geometry)).toBeLessThanOrEqual(geometry.pick.left);
    expect(geometry.select.left, JSON.stringify(geometry)).toBeGreaterThanOrEqual(geometry.header.left);
    expect(geometry.pick.right, JSON.stringify(geometry)).toBeLessThanOrEqual(geometry.header.right);
    expect(geometry.title.right <= geometry.context.left || geometry.title.bottom <= geometry.context.top,
      JSON.stringify(geometry)).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)).toBe(false);
    await page.getByRole("button", { name: "Chọn thiết bị", exact: true }).click();
    await expect(page.getByRole("dialog", { name: "Chọn thiết bị thực hiện" })).toBeVisible();
    await page.getByRole("button", { name: "Xong", exact: true }).click();
    expect(await page.evaluate(() => (window as unknown as { publishBoardCalls: string[] }).publishBoardCalls)).toEqual([]);
  });
}
for (const viewport of [{ width: 1440, height: 900 }, { width: 1024, height: 768 }, { width: 820, height: 560 }]) {
  test(`ba khung và Google thật giữ vùng danh sách tại ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport); await openBoard(page);
    await page.getByRole("button", { name: "Chọn nhanh", exact: true }).click();
    await expect(page.locator(".pq-links .pq-link")).toHaveCount(10);
    const geometry = await measureBoard(page);
    await test.info().attach("Hình học ba khung", { body: JSON.stringify(geometry, null, 2), contentType: "application/json" });
    expect(geometry.overflow).toBe(false);
    expect(geometry.panels[0].right).toBeLessThanOrEqual(geometry.panels[1].x);
    expect(geometry.panels[1].right).toBeLessThanOrEqual(geometry.panels[2].x);
    expect(Math.max(...geometry.panels.map(r => r.y)) - Math.min(...geometry.panels.map(r => r.y))).toBeLessThanOrEqual(1);
    expect(Math.max(...geometry.rows)).toBeLessThanOrEqual(100);
    // The compact shell leaves less vertical space at 820x560; the next check requires two complete rows.
    expect(geometry.roster.height).toBeGreaterThanOrEqual(viewport.width <= 820 ? 116 : 150);
    expect(geometry.fullyVisibleDevices).toBeGreaterThanOrEqual(2);
    expect(geometry.roster.bottom).toBeLessThanOrEqual(geometry.footer.y);
    expect(geometry.search.bottom).toBeLessThanOrEqual(geometry.roster.y);
    await expect(page.getByRole("textbox", { name: "Tìm số máy" })).toBeVisible();
    const settings = page.getByRole("button", { name: "Thiết lập Google Sheet", exact: true });
    if (await settings.isVisible()) await settings.click();
    await expect(page.getByRole("textbox", { name: "Link Google Sheet" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Đăng nhập Google", exact: true })).toBeVisible();
    if (await settings.isVisible()) { await page.getByRole("textbox", { name: "Link Google Sheet" }).press("Escape"); await expect(settings).toBeFocused(); }
    await page.screenshot({ path: test.info().outputPath(`publish-board-${viewport.width}.png`) });
    expect((await new AxeBuilder({ page }).include(".publish-page").analyze()).violations).toEqual([]);
    expect(await page.evaluate(() => (window as unknown as { publishBoardCalls: string[] }).publishBoardCalls)).toEqual([]);
  });
}

test("gán bị khóa khi active ẩn, dropdown tường minh vẫn đổi chỗ; thay bài phải xác nhận", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 }); await openBoard(page);
  await page.getByRole("button", { name: "Chọn nhanh", exact: true }).click();
  const one = page.getByRole("combobox", { name: "Máy nhận bài Bài 1", exact: true });
  const two = page.getByRole("combobox", { name: "Máy nhận bài Bài 2", exact: true });
  const first = await one.inputValue(), second = await two.inputValue();
  await page.getByRole("textbox", { name: "Tìm bài đăng" }).fill("Bài 2");
  await expect(page.getByRole("button", { name: /^Đổi chỗ Bài 1 ·/ }).first()).toBeDisabled();
  await one.selectOption(second);
  await expect(two).toHaveValue(first);
  await page.getByRole("button", { name: "Hiện bài", exact: true }).click();
  await expect(page.getByRole("button", { name: "Chọn bài đang gán · Bài 1", exact: true })).toBeFocused();
  await page.getByRole("button", { name: "Bỏ ghép bài Bài 1", exact: true }).click();
  await one.selectOption(first);
  await expect(page.getByRole("alertdialog")).toContainText("chờ ghép máy");
  await page.getByRole("button", { name: "Hủy", exact: true }).click();
  await expect(one).toHaveValue(""); await expect(two).toHaveValue(first);
  await one.selectOption(first);
  await page.getByRole("button", { name: "Thay bài trong bản nháp", exact: true }).click();
  await expect(one).toHaveValue(first); await expect(two).toHaveValue("");
  expect(await page.evaluate(() => (window as unknown as { publishBoardCalls: string[] }).publishBoardCalls)).toEqual([]);
});

test("caption bàn phím trả đúng origin trái/giữa, đủ slide và giữ bản nháp Google", async ({ page }) => {
  await openBoard(page); await page.getByRole("button", { name: "Chọn nhanh", exact: true }).click();
  const google = page.getByRole("textbox", { name: "Link Google Sheet" });
  await google.fill("https://docs.google.com/spreadsheets/d/unsaved/edit#gid=7");
  for (const label of ["Xem ảnh và sửa caption · Bài 1", "Sửa caption · Bài 1"]) {
    const origin = page.getByRole("button", { name: label, exact: true });
    await origin.focus(); await page.keyboard.press("Enter");
    const dialog = page.getByRole("dialog", { name: "Ảnh & caption · Bài 1", exact: true });
    await expect(dialog).toContainText("Đối tác 1");
    await dialog.getByRole("button", { name: "Ảnh tiếp" }).click(); await dialog.getByRole("button", { name: "Ảnh tiếp" }).click();
    await expect(dialog.getByRole("img", { name: "Ảnh 3.png" })).toBeVisible();
    await dialog.getByRole("textbox", { name: "Nội dung bài đăng" }).fill("Caption sửa trong bản nháp");
    await page.keyboard.press("Escape"); await expect(origin).toBeFocused();
    await expect(google).toHaveValue("https://docs.google.com/spreadsheets/d/unsaved/edit#gid=7");
  }
  expect(await page.evaluate(() => (window as unknown as { publishBoardCalls: string[] }).publishBoardCalls)).toEqual([]);
});

test("100 máy, tên dài và lỗi Google vẫn cuộn nội bộ; reduced motion không đổi mapping", async ({ page }) => {
  await page.setViewportSize({ width: 820, height: 560 }); await page.emulateMedia({ reducedMotion: "reduce" });
  await openBoard(page, { fleet: 100, posts: 105, longNames: true, googleError: true });
  await page.getByRole("button", { name: "Chọn nhanh", exact: true }).click();
  await expect(page.locator(".pq-links .pq-link")).toHaveCount(100);
  await expect(page.locator(".google-sheet-connection")).toContainText("Không đọc được trạng thái Google");
  expect((await measureBoard(page)).overflow).toBe(false);
  await page.locator(".pq-links").evaluate(node => { node.scrollTop = node.scrollHeight; });
  await expect(page.locator(".pq-link").last()).toBeInViewport();
  await expect(page.locator(".pq-footer")).toBeInViewport();
  await page.screenshot({ path: test.info().outputPath("publish-board-long-reduced.png") });
  expect(await page.evaluate(() => (window as unknown as { publishBoardCalls: string[] }).publishBoardCalls)).toEqual([]);
});
