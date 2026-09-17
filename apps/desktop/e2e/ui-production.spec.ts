import { expect, test } from "@playwright/test";
import { installTauriMock, mockCommandCalls } from "./fixtures/tauriMock";
import { openOperatorPage } from "./fixtures/operatorNavigation";

const pages = [
  ["Thiết bị", "Thiết bị"], ["Nuôi TikTok", "Nuôi TikTok"],
  ["Tương tác", "Tương tác"], ["Đăng bài", "Đăng bài"],
  ["My Apps", "My Apps"], ["Tác vụ", "Lượt chạy"],
  ["Tác vụ đã lưu", "Tác vụ đã lưu"], ["Flow", "Flow"],
  ["Quản lý tài khoản", "Quản lý tài khoản"], ["Lịch chạy", "Lịch chạy"],
  ["Kho nội dung", "Kho nội dung"], ["Trung tâm ứng dụng", "Trung tâm ứng dụng"],
  ["API", "API"], ["Chẩn đoán", "Chẩn đoán"],
  ["Cài đặt", "Cài đặt"], ["Trợ giúp", "Trợ giúp"],
] as const;

test("light workspace keeps its chosen navigation readable on a dark operating system", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await installTauriMock(page);
  await page.goto("/");
  await expect(page.getByTestId("device-tile")).toHaveCount(2);
  await openOperatorPage(page, "API");
  const palette = await page.evaluate(() => {
    const root = getComputedStyle(document.documentElement);
    const active = getComputedStyle(document.querySelector(".menu-item.active")!);
    return {
      scheme: root.colorScheme,
      panel: root.getPropertyValue("--bg-elevated").trim(),
      background: active.backgroundColor,
      foreground: active.color,
    };
  });
  expect(palette.scheme).toContain("light");
  expect(palette.scheme).not.toContain("dark");
  expect(palette.panel).toBe("#ffffff");
  expect(palette.background).toBe("rgb(194, 65, 12)");
  expect(palette.foreground).toBe("rgb(255, 255, 255)");
});

for (const viewport of [{ width: 1440, height: 900 }, { width: 1024, height: 768 }, { width: 820, height: 560 }]) {
  test(`all 16 pages share the readable workspace at ${viewport.width}`, async ({ page }, testInfo) => {
    test.setTimeout(150_000);
    await page.setViewportSize(viewport);
    await page.emulateMedia({ reducedMotion: "reduce", colorScheme: "dark" });
    await installTauriMock(page, { androidRoster: true, fleetSize: 20 });
    await page.goto("/");
    await expect(page.getByTestId("device-tile")).toHaveCount(20);
    for (const [route, heading] of pages) {
      if (route !== "Thiết bị") await openOperatorPage(page, route);
      await expect(page.getByRole("heading", { level: 1, name: heading, exact: true })).toBeVisible();
      await expect(page.locator(".loading-state")).toHaveCount(0);
      await expect(page.getByText(/Unknown mock command/)).toHaveCount(0);
      await page.evaluate(() => document.fonts.ready);
      const dimensions = await page.evaluate(() => {
        const content = document.querySelector<HTMLElement>(".content")!;
        const main = document.querySelector<HTMLElement>(".main-col")!;
        return {
          pageWidth: document.documentElement.scrollWidth,
          viewportWidth: innerWidth,
          contentWidth: content.scrollWidth,
          available: content.clientWidth,
          mainWidth: main.scrollWidth,
          mainAvailable: main.clientWidth,
          headingSize: parseFloat(getComputedStyle(document.querySelector("h1")!).fontSize),
          controls: Array.from(content.querySelectorAll<HTMLElement>("button,input,select,textarea"))
            .filter(element => element.getClientRects().length > 0)
            .map(element => parseFloat(getComputedStyle(element).fontSize)),
        };
      });
      expect(dimensions.pageWidth, route).toBeLessThanOrEqual(dimensions.viewportWidth + 1);
      expect(dimensions.contentWidth, route).toBeLessThanOrEqual(dimensions.available + 1);
      expect(dimensions.mainWidth, route).toBeLessThanOrEqual(dimensions.mainAvailable + 1);
      expect(dimensions.headingSize, route).toBeGreaterThanOrEqual(20);
      expect(dimensions.controls.every(value => value >= 12), route).toBe(true);
      await page.screenshot({ path: testInfo.outputPath(`page-${pages.findIndex(([name]) => name === route)}-${viewport.width}.png`), animations: "disabled" });
    }
    const effects = (await mockCommandCalls(page)).filter(call => /^(publish_(execute|create_campaign)|nurture_start|interaction_start_thread|install_library_app_batch|push_material_batch|agent_(repair|bulk_repair))$/.test(call.command));
    expect(effects).toEqual([]);
  });
}
