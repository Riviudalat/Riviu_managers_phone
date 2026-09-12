import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";

test("nurture keeps the document at viewport height while its columns scroll", async ({ page }) => {
  await installTauriMock(page, { androidRoster: true, fleetSize: 30 });
  await page.goto("/");
  await page.getByRole("button", { name: "Nuôi TikTok", exact: true }).click();
  await expect(page.locator(".nurture-machine-grid input")).toHaveCount(30);
  await page.locator(".nurture-advanced > summary").click();
  for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }, { width: 1440, height: 1000 }]) {
    await page.setViewportSize(viewport);
    const settings = viewport.width < 1024 ? page.locator("#nurture-page-panel-setup") : page.locator(".nurture-setup-fields");
    for (const position of [0, 100000]) {
      await settings.evaluate((element, top) => { element.scrollTop = top; }, position);
      await page.evaluate(() => { window.scrollTo(0, 100000); });
      const geometry = await page.evaluate(() => ({
        viewport: innerHeight,
        document: document.documentElement.scrollHeight,
        scrollY,
        sidebarBottom: document.querySelector(".aside")!.getBoundingClientRect().bottom,
        footerBottom: document.querySelector(".nurture-session-footer")!.getBoundingClientRect().bottom,
      }));
      console.log(JSON.stringify({ width: viewport.width, position, ...geometry }));
      expect(geometry.document).toBe(geometry.viewport);
      expect(geometry.scrollY).toBe(0);
      expect(geometry.sidebarBottom).toBe(geometry.viewport);
      expect(geometry.viewport - geometry.footerBottom).toBeLessThanOrEqual(40);
    }
    const toggle = page.locator(".nu-switch").filter({ has: page.getByText("Mỏi dần", { exact: true }) }).locator('input[type="checkbox"]');
    const wasChecked = await toggle.isChecked();
    await page.getByText("Mỏi dần", { exact: true }).click();
    await expect(toggle).toBeChecked({ checked: !wasChecked });
    await page.screenshot({ path: test.info().outputPath(`no-bottom-gap-${viewport.width}x${viewport.height}.png`), fullPage: true });
    await page.getByRole("tab", { name: "Hẹn giờ", exact: true }).click();
    expect(await page.evaluate(() => document.documentElement.scrollHeight)).toBe(viewport.height);
    await page.getByRole("tab", { name: "Thiết lập", exact: true }).click();
  }
});
