import { expect, test } from "@playwright/test";

import { installTauriMock } from "./fixtures/tauriMock";
import { openOperatorPage } from "./fixtures/operatorNavigation";

test("shell keeps navigation, current page and fleet status usable at operator sizes", async ({ page }, testInfo) => {
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 820, height: 560 },
  ]) {
    await page.setViewportSize(viewport);
    await installTauriMock(page);
    await page.goto("/");
    await expect(page.locator("[data-testid='device-tile']")).toHaveCount(2);

    const navigation = page.getByRole("navigation", { name: "Điều hướng chính" });
    const fleetStatus = page.getByLabel("Trạng thái hệ thống");
    await expect(fleetStatus).toBeVisible();
    const systemGroup = navigation.getByRole("button", { name: "Hệ thống" });
    if (await systemGroup.getAttribute("aria-expanded") === "false") await systemGroup.click();
    await openOperatorPage(page, "Cài đặt");
    await expect(page.getByTestId("page-title")).toHaveText("Cài đặt");
    await expect(navigation.getByRole("button", { name: "Cài đặt" })).toHaveAttribute("aria-current", "page");

    await systemGroup.click();
    await expect(systemGroup).toHaveAttribute("aria-expanded", "false");
    await expect(navigation.locator(".menu-group[data-active='true']")).toHaveCount(1);

    const geometry = await page.evaluate(() => {
      const rect = (selector: string) => {
        const element = document.querySelector<HTMLElement>(selector);
        if (!element) throw new Error(`Missing ${selector}`);
        const box = element.getBoundingClientRect();
        return { top: box.top, bottom: box.bottom, right: box.right, width: box.width,
          scrollWidth: element.scrollWidth, clientWidth: element.clientWidth };
      };
      return {
        sidebar: rect(".aside"), navigation: rect(".aside-scroll"),
        status: rect(".aside-stats"), header: rect(".main-col > .page-header"),
      };
    });
    expect(geometry.navigation.bottom).toBeLessThanOrEqual(geometry.status.top + 1);
    expect(geometry.status.bottom).toBeLessThanOrEqual(viewport.height + 1);
    expect(geometry.header.right).toBeLessThanOrEqual(viewport.width + 1);
    expect(geometry.header.scrollWidth).toBeLessThanOrEqual(geometry.header.clientWidth + 1);
    await expect(page.getByText("Toàn hệ thống", { exact: true })).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath(`shell-${viewport.width}x${viewport.height}.png`) });
  }
});

test("icon rail preserves navigation and makes room for compact workspaces", async ({ page }, testInfo) => {
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 820, height: 560 },
  ]) {
    await page.setViewportSize(viewport);
    await installTauriMock(page);
    await page.goto("/");
    await expect(page.locator("[data-testid='device-tile']")).toHaveCount(2);
    await page.getByRole("button", { name: "Thu gọn thanh điều hướng" }).click();
    const sidebar = page.getByRole("complementary", { name: "Riviu Manager" });
    await expect(sidebar).toHaveAttribute("data-rail-collapsed", "true");
    const width = await sidebar.evaluate((element) => element.getBoundingClientRect().width);
    expect(width).toBeLessThanOrEqual(60);
    await expect(sidebar.getByLabel("Trạng thái hệ thống")).toBeVisible();
    await sidebar.getByRole("button", { name: "My Apps" }).click();
    await expect(page.getByTestId("page-title")).toHaveText("My Apps");
    await expect(sidebar.getByRole("button", { name: "My Apps" })).toHaveAttribute("aria-current", "page");
    await page.screenshot({ path: testInfo.outputPath(`icon-rail-${viewport.width}x${viewport.height}.png`) });
    await page.getByRole("button", { name: "Mở rộng thanh điều hướng" }).click();
    await expect(sidebar).toHaveAttribute("data-rail-collapsed", "false");
  }
});

test("workflow editor keeps an icon route rail beside the canvas", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 820, height: 560 });
  await installTauriMock(page);
  await page.goto("/");
  await expect(page.locator("[data-testid='device-tile']")).toHaveCount(2);
  await openOperatorPage(page, "My Apps");
  await page.getByRole("button", { name: "Ứng dụng mới" }).click();
  await expect(page.locator(".app-workflow-editor")).toBeVisible();
  const sidebar = page.getByRole("complementary", { name: "Riviu Manager" });
  await expect(sidebar).toHaveAttribute("data-rail-collapsed", "true");
  await expect(sidebar.getByRole("button", { name: "Đăng bài" })).toBeVisible();
  const geometry = await page.evaluate(() => {
    const rail = document.querySelector<HTMLElement>(".aside")!.getBoundingClientRect();
    const editor = document.querySelector<HTMLElement>(".app-workflow-editor")!.getBoundingClientRect();
    return { railRight: rail.right, editorLeft: editor.left, editorRight: editor.right };
  });
  expect(geometry.editorLeft).toBeGreaterThanOrEqual(geometry.railRight - 1);
  expect(geometry.editorRight).toBeLessThanOrEqual(820 + 1);
  await page.screenshot({ path: testInfo.outputPath("workflow-editor-icon-rail-820x560.png") });
});
