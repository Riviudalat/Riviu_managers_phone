import { expect, test } from "@playwright/test";
import { installTauriMock, mockCommandCalls } from "./fixtures/tauriMock";

test("stream changes wait for Apply and keep multi-digit FPS intact", async ({ page }) => {
  await installTauriMock(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Cài đặt", exact: true }).click();
  const fps = page.getByLabel("FPS overlay");
  await expect(fps).toHaveValue("24");
  await fps.focus();
  await fps.press("ControlOrMeta+A");
  await fps.pressSequentially("15", { delay: 100 });
  await expect(fps).toHaveValue("15");
  expect((await mockCommandCalls(page)).filter((call) => call.command === "set_stream_settings")).toHaveLength(0);
  await page.getByRole("button", { name: "Áp dụng chất lượng hình", exact: true }).click();
  await expect(page.getByText("Đã áp dụng chất lượng hình.", { exact: true })).toBeVisible();
  expect((await mockCommandCalls(page)).filter((call) => call.command === "set_stream_settings")).toEqual([
    { command: "set_stream_settings", args: { settings: { fps: 15, gridQuality: "medium", focusQuality: "high" } } },
  ]);
});

test("settings navigation preserves a draft on Stay and discards only explicitly", async ({ page }) => {
  await installTauriMock(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Cài đặt", exact: true }).click();
  const port = page.getByLabel("Cổng", { exact: true });
  await expect(port).toHaveValue("17999");
  await port.fill("45555");
  await page.getByRole("button", { name: "Dữ liệu", exact: true }).click();
  await page.getByRole("button", { name: "Ở lại", exact: true }).click();
  await expect(port).toHaveValue("45555");
  await page.getByRole("button", { name: "Dữ liệu", exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Bỏ thay đổi", exact: true }).click();
  await page.getByRole("button", { name: "Cài đặt", exact: true }).click();
  await expect(page.getByLabel("Cổng", { exact: true })).toHaveValue("17999");
  expect((await mockCommandCalls(page)).filter((call) => call.command === "local_api_set_config")).toHaveLength(0);
});

test("help stays inside the viewport near scroll edges and after resizing", async ({ page }) => {
  await page.setViewportSize({ width: 820, height: 560 });
  await installTauriMock(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Nuôi TikTok", exact: true }).click();
  const help = page.locator(".nu-info").first();
  await expect(help).toBeVisible();
  await help.evaluate((element) => element.scrollIntoView({ block: "start" }));
  await help.click();
  const tooltip = page.getByRole("tooltip");
  for (const viewport of [{ width: 820, height: 560 }, { width: 900, height: 900 }]) {
    await page.setViewportSize(viewport);
    await expect(tooltip).toBeVisible();
    await expect.poll(async () => {
      const box = await tooltip.boundingBox();
      return box !== null && box.x >= 0 && box.y >= 0 && box.x + box.width <= viewport.width && box.y + box.height <= viewport.height;
    }).toBe(true);
  }
  await page.keyboard.press("Escape");
  await expect(tooltip).toHaveCount(0);
});
