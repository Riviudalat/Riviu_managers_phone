import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";

const pages = ["Thiết bị", "Chẩn đoán", "Nuôi TikTok", "Tương tác", "Đăng bài", "Flow",
  "Tác vụ", "Kho nội dung", "Trung tâm ứng dụng", "Dữ liệu", "API", "Cài đặt"];

test("all workspaces expose accessible controls and a real nonempty main surface", async ({ page }, testInfo) => {
  test.setTimeout(180_000);
  await installTauriMock(page, {androidRoster:true});
  await page.goto("/");
  await expect(page.getByTestId("device-tile")).toHaveCount(2);
  for (const name of pages) {
    if (name !== "Thiết bị") await page.getByRole("button", {name,exact:true}).click();
    await expect(page.locator(".loading-state")).toHaveCount(0);
    await expect(page.getByRole("heading", {level:1,name,exact:true})).toBeVisible();
    await expect(page.getByText(/Unknown mock command/)).toHaveCount(0);
    const bounds = await page.locator(".content").evaluate((element) => {
      const rect = element.getBoundingClientRect();
      return {width:rect.width,height:rect.height,scrollWidth:element.scrollWidth,
        clientWidth:element.clientWidth,text:element.textContent?.trim().length ?? 0};
    });
    expect(bounds.width).toBeGreaterThan(300);
    expect(bounds.height).toBeGreaterThan(200);
    expect(bounds.text).toBeGreaterThan(30);
    expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.clientWidth+1);
    const results = await new AxeBuilder({page}).include(".content")
      .withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"]).analyze();
    await testInfo.attach(`accessibility-${pages.indexOf(name)}.json`, {
      body:JSON.stringify(results.violations,null,2),contentType:"application/json",
    });
    expect.soft(results.violations.map(({id,nodes}) => ({id,targets:nodes.map((node)=>node.target)})), name).toEqual([]);
  }
});

test("save dialog traps keyboard focus and restores it on Escape", async ({ page }) => {
  await installTauriMock(page);
  await page.goto("/");
  await page.getByRole("button", {name:"Tương tác",exact:true}).click();
  await page.getByPlaceholder("Dán link TikTok, mỗi dòng một bài").fill("https://www.tiktok.com/@fixture/video/123");
  const destination = page.getByRole("button", {name:"Dữ liệu",exact:true});
  await destination.click();
  const dialog = page.getByRole("alertdialog", {name:"Thay đổi chưa được lưu"});
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("button", {name:"Lưu",exact:true})).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(dialog.getByRole("button", {name:"Ở lại",exact:true})).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(dialog.getByRole("button", {name:"Lưu",exact:true})).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(destination).toBeFocused();
});

test("Flow run command is readable immediately after validation and on hover", async ({ page }, testInfo) => {
  await installTauriMock(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Flow", exact: true }).click();
  const run = page.getByRole("button", { name: "Chạy Flow", exact: true });
  await expect(run).toBeEnabled();
  await page.getByRole("textbox", { name: "Tên Flow" }).fill("Draft requiring validation");
  await expect(run).toBeDisabled();
  const samples = page.evaluate(() => new Promise<Array<{ opacity: string; color: string; background: string }>>((resolve, reject) => {
    const result: Array<{ opacity: string; color: string; background: string }> = [];
    const deadline = performance.now() + 5000;
    const sample = () => {
      const button = document.querySelector<HTMLButtonElement>(".flow-run-command");
      if (button && !button.disabled) {
        const style = getComputedStyle(button);
        result.push({ opacity: style.opacity, color: style.color, background: style.backgroundColor });
      }
      if (result.length === 12) return resolve(result);
      if (performance.now() > deadline) return reject(new Error("Flow never returned to ready"));
      requestAnimationFrame(sample);
    };
    sample();
  }));
  await page.getByRole("button", { name: "Hoàn tác", exact: true }).click();
  const readySamples = await samples;
  await testInfo.attach("flow-ready-computed-styles.json", {
    body: JSON.stringify(readySamples, null, 2), contentType: "application/json",
  });
  expect(readySamples.every((sample) => sample.opacity === "1")).toBe(true);
  expect(new Set(readySamples.map((sample) => `${sample.color}|${sample.background}`)).size).toBe(1);
  await run.hover();
  const results = await new AxeBuilder({ page }).include(".flow-run-command")
    .withRules(["color-contrast"]).analyze();
  expect(results.violations).toEqual([]);
});
