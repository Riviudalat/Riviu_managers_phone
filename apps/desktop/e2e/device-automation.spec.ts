import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installTauriMock, mockCommandCalls } from "./fixtures/tauriMock";

for (const viewport of [{ width: 1673, height: 1000 }, { width: 1440, height: 900 }, { width: 900, height: 900 }, { width: 820, height: 560 }]) {
  test(`device dock and filled rows at ${viewport.width}x${viewport.height}`, async ({ page }, testInfo) => {
    test.setTimeout(90_000);
    await page.setViewportSize(viewport);
    await installTauriMock(page, { androidRoster: true, fleetSize: 20 });
    await page.goto("/");
    await expect(page.getByTestId("device-tile")).toHaveCount(20);
    await page.evaluate(() => document.fonts.ready);
    const before = await page.getByRole("grid", { name: "Lưới thiết bị" }).evaluate((grid) => {
      const tiles = [...grid.querySelectorAll<HTMLElement>(".dev-phone")].map((tile) => tile.getBoundingClientRect());
      const row = tiles.filter((tile) => Math.abs(tile.top - tiles[0].top) < 1);
      return { gap: grid.getBoundingClientRect().right - row.at(-1)!.right, widths: tiles.map((tile) => tile.width), ratio: tiles[0].height / tiles[0].width };
    });
    expect(Math.abs(before.gap)).toBeLessThan(2);
    expect(Math.max(...before.widths) - Math.min(...before.widths)).toBeLessThan(1);
    expect(before.ratio).toBeCloseTo(2, 1);
    if (viewport.width === 1673) {
      const tile = page.getByTestId("device-tile").first();
      const width = (await tile.boundingBox())!.width;
      await tile.hover();
      await page.keyboard.down("Control");
      try { await page.mouse.wheel(0, -100); } finally { await page.keyboard.up("Control"); }
      await expect.poll(async () => (await tile.boundingBox())!.width).toBeGreaterThan(width);
    }

    await page.getByTestId("device-tile").first().click();
    const tabs = page.getByRole("tablist", { name: "Tác vụ bên cạnh thiết bị" });
    await tabs.getByRole("tab", { name: "Nuôi TikTok" }).click();
    await page.getByRole("button", { name: "Dùng 1 máy đã chọn" }).click();
    const dock = page.locator(".automation-host.is-docked");
    await expect(dock.locator("summary").filter({ hasText: "Phạm vi thiết bị" })).toContainText("1 máy");
    for (const name of ["Nuôi TikTok", "Tương tác", "Đăng bài"]) {
      await tabs.getByRole("tab", { name, exact: true }).click();
      if (name === "Tương tác") await page.getByRole("button", { name: "Bỏ thay đổi" }).click();
      await expect(tabs.getByRole("tab", { name, exact: true })).toHaveAttribute("aria-selected", "true");
      await expect(page.locator(".loading-state")).toHaveCount(0);
      const geometry = await page.evaluate(() => {
        const wall = document.querySelector(".device-browser-content")!.getBoundingClientRect();
        const dockEl = document.querySelector<HTMLElement>(".automation-host.is-docked")!;
        const dock = dockEl.getBoundingClientRect();
        const content = document.querySelector<HTMLElement>(".content")!;
        return { wall: { right: wall.right, bottom: wall.bottom, height: wall.height },
          dock: { left: dock.left, top: dock.top, right: dock.right, bottom: dock.bottom, height: dock.height },
          overflow: content.scrollWidth - content.clientWidth, viewport: innerWidth };
      });
      expect(geometry.overflow, name).toBeLessThanOrEqual(1);
      expect(geometry.dock.right).toBeLessThanOrEqual(viewport.width);
      expect(geometry.dock.bottom).toBeLessThanOrEqual(viewport.height);
      expect(geometry.wall.height).toBeGreaterThanOrEqual(180);
      expect(geometry.dock.height).toBeGreaterThan(160);
      expect(geometry.dock.left).toBeGreaterThanOrEqual(geometry.wall.right);
      await expect(dock).toBeVisible();
      await expect(page.getByText(/Unknown mock command/)).toHaveCount(0);
      await expect(page.locator(".activity-center-current.is-error")).toHaveCount(0);
      await dock.getByRole("tab", { name: "Theo dõi", exact: true }).click();
      await expect(dock.getByRole("tab", { name: "Theo dõi", exact: true })).toHaveAttribute("aria-selected", "true");
      await expect(page.getByRole("grid", { name: "Lưới thiết bị" })).toBeVisible();
      await dock.getByRole("tab", { name: "Thiết lập", exact: true }).click();
      await page.screenshot({ path: testInfo.outputPath(`dock-${name}.png`) });
      if (viewport.width === 1440) {
        const result = await new AxeBuilder({ page }).include(".content").withTags(["wcag2a", "wcag2aa"]).analyze();
        expect(result.violations.map(({ id, nodes }) => ({ id, targets: nodes.map((node) => node.target) })), name).toEqual([]);
      }
    }
    const calls = await mockCommandCalls(page);
    expect(calls.filter(({ command }) => /^(nurture_start|interaction_start_thread|publish_execute|publish_create_campaign)$/.test(command))).toEqual([]);
  });
}

test("dock preserves unsaved interaction form across page layout changes", async ({ page }) => {
  await installTauriMock(page);
  await page.goto("/");
  await page.getByRole("tab", { name: "Tương tác", exact: true }).click();
  const input = page.getByPlaceholder("Dán link TikTok, mỗi dòng một bài");
  const value = "https://www.tiktok.com/@fixture/video/123";
  await input.fill(value);
  await page.getByRole("button", { name: "Mở trang tác vụ" }).click();
  await expect(input).toHaveValue(value);
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await page.getByRole("button", { name: "Xem cùng thiết bị" }).click();
  await expect(input).toHaveValue(value);
  await page.getByRole("button", { name: "Đóng khung tác vụ" }).click();
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await page.getByRole("button", { name: "Ở lại" }).click();
  await expect(input).toHaveValue(value);
  await page.getByRole("tab", { name: "Đăng bài", exact: true }).click();
  await page.getByRole("button", { name: "Bỏ thay đổi" }).click();
  await expect(page.locator(".publish-page")).toBeVisible();
});
