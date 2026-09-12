import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installTauriMock, mockCommandCalls } from "./fixtures/tauriMock";

test("nurture tabs and one machine grid preserve drafts at three viewports", async ({ page }) => {
  await installTauriMock(page, { androidRoster: true, fleetSize: 38 });
  await page.goto("/");
  await page.getByRole("button", { name: "Nuôi TikTok", exact: true }).click();
  await page.getByRole("button", { name: /Cân bằng/ }).click();
  await page.getByRole("region", { name: "Máy thực hiện", exact: true }).getByRole("button", { name: "Chọn tất cả sẵn sàng", exact: true }).click();
  const machines = page.getByRole("group", { name: "Danh sách chọn máy Nuôi TikTok" });
  await expect(page.getByRole("tablist", { name: "Chế độ Nuôi TikTok" }).getByRole("tab")).toHaveText(["Thiết lập", "Hẹn giờ", "Theo dõi"]);
  await expect(page.getByText("Chọn theo nhóm hoặc toàn bộ", { exact: true })).toHaveCount(0);
  const scope = page.getByRole("combobox", { name: "Phạm vi thiết bị", exact: true });
  expect(await scope.evaluate(element => element.parentElement?.classList.contains("nurture-machine-tools"))).toBe(true);
  await expect(machines.getByRole("checkbox")).toHaveCount(38);
  await expect(machines.getByRole("checkbox", { checked: true })).toHaveCount(38);
  await expect(page.getByRole("button", { name: "Trang máy tiếp" })).toHaveCount(0);
  const total = page.getByRole("spinbutton", { name: "Tổng số video muốn lướt", exact: true });
  await expect(total).toBeVisible();
  await expect(page.getByLabel("Giới hạn video", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("spinbutton", { name: "Vòng", exact: true })).toHaveCount(0);
  await expect(page.locator('.nurture-workspace input[data-nurture-field="numVideos"]')).toHaveCount(1);
  await total.fill("75");
  await expect(page.getByRole("spinbutton", { name: /Thời lượng tối đa/ })).toHaveValue("20");
  await expect(page.locator(".nurture-advanced")).not.toHaveAttribute("open", "");
  expect((await new AxeBuilder({ page }).include(".nurture-workspace").withTags(["wcag2a", "wcag2aa"]).analyze()).violations).toEqual([]);
  for (const viewport of [{ width: 1440, height: 900 }, { width: 900, height: 900 }, { width: 820, height: 560 }]) {
    await page.setViewportSize(viewport);
    expect(await machines.evaluate(element => getComputedStyle(element).gridTemplateColumns.split(" ").length)).toBe(2);
    await page.locator(".nurture-machines-card").scrollIntoViewIfNeeded();
    const pickerWidth = await page.locator(".nurture-machines-card").evaluate(element => element.getBoundingClientRect().width);
    expect(pickerWidth).toBeLessThanOrEqual(viewport.width >= 1024 ? 355 : viewport.width);
    const pickerLayout = await page.locator(".nurture-machines-card").evaluate(card => {
      const rect = card.getBoundingClientRect();
      const grid = card.querySelector(".nurture-machine-grid")!;
      const parent = card.closest(".nurture-workspace-body")!;
      return { bottom: rect.bottom, viewport: innerHeight, gridScrolls: grid.scrollHeight > grid.clientHeight, outerScrolls: parent.scrollHeight > parent.clientHeight + 1 };
    });
    if (viewport.width >= 1024) expect(pickerLayout.bottom).toBeLessThanOrEqual(pickerLayout.viewport);
    expect(pickerLayout.gridScrolls).toBe(true);
    expect(pickerLayout.outerScrolls).toBe(false);
    const clipped = await page.locator(".nurture-setup-fields").evaluate(element => {
      const edge = element.getBoundingClientRect().right;
      return [...element.querySelectorAll<HTMLElement>("*")].filter(child => child.getClientRects().length && child.getBoundingClientRect().right > edge + 1 && !child.closest('[hidden],details:not([open])'))
        .slice(0, 10).map(child => `${child.tagName}.${child.className}: ${Math.round(child.getBoundingClientRect().right - edge)}px`);
    });
    expect(clipped).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const footer = await page.locator(".nurture-session-footer").boundingBox();
    expect(footer!.y + footer!.height).toBeLessThanOrEqual(viewport.height);
    await page.getByRole("region", { name: "Thiết lập phiên", exact: true }).scrollIntoViewIfNeeded();
    await page.screenshot({ path: test.info().outputPath(`nurture-setup-${viewport.width}.png`) });
  }
  await page.getByRole("tab", { name: "Thiết lập", exact: true }).click();
  await page.locator(".nurture-advanced > summary").click();
  await expect(page.getByRole("tab", { name: "Hành vi", exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "AI", exact: true }).click();
  await expect(page.locator('input[list="riviu-comment-models"]')).toBeVisible();
  await page.getByRole("tab", { name: "Hẹn giờ", exact: true }).click();
  await expect(page.getByRole("checkbox", { name: /Lịch tự chạy/ })).toBeVisible();
  await page.getByRole("button", { name: "+ Thêm khung giờ", exact: true }).click();
  await expect(page.getByLabel("Giờ bắt đầu khung 1", { exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "Thiết lập", exact: true }).click();
  await expect(total).toHaveValue("75");
  await expect(machines.getByRole("checkbox", { checked: true })).toHaveCount(38);
  await page.getByRole("searchbox", { name: "Tìm máy Nuôi TikTok" }).fill("38");
  await expect(machines.getByRole("checkbox")).toHaveCount(1);
  await machines.getByRole("checkbox").uncheck();
  await page.getByRole("searchbox", { name: "Tìm máy Nuôi TikTok" }).fill("");
  await expect(machines.getByRole("checkbox", { checked: true })).toHaveCount(37);
  await expect(page.locator(".nurture-count")).toHaveText("Đã chọn 37");
  await expect(scope).toHaveValue("explicit");
  await expect(scope.locator("option:checked")).toHaveText("37 máy đã chọn");
  const calls = await mockCommandCalls(page);
  expect(calls.filter(call => call.command === "nurture_start")).toEqual([]);
});
