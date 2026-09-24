import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";
import { openOperatorPage } from "./fixtures/operatorNavigation";

for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
  test(`schedule list and detail stay separate at ${viewport.width}x${viewport.height}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await installTauriMock(page);
    await page.addInitScript(() => {
      const bridge = (window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> } }).__TAURI_INTERNALS__;
      const invoke = bridge.invoke;
      bridge.invoke = async (command, args) => {
        if (command === "automation_list") return [{ id: "profile-1", name: "Hồ sơ buổi sáng", kind: "nurture", latestRevision: 2, archived: false, createdAt: "2026-09-24T00:00:00Z", updatedAt: "2026-09-24T00:00:00Z" }];
        if (command === "automation_schedule_list") return [{ id: "schedule-1", revision: 1, name: "Lịch buổi sáng", definitionId: "profile-1", definitionRevision: 2, enabled: true, schedule: { schemaVersion: 1, kind: "interval", everyMinutes: 60 }, nextDueAt: "2026-09-24T08:00:00Z", lastErrorCode: null, createdAt: "2026-09-24T00:00:00Z", updatedAt: "2026-09-24T00:00:00Z" }];
        return invoke(command, args);
      };
    });
    await page.goto("/");
    await openOperatorPage(page, "Lịch chạy");
    const list = page.getByRole("region", { name: "Danh sách lịch" });
    const detail = page.getByRole("region", { name: "Cấu hình lịch" });
    await expect(list.getByText("Lịch buổi sáng")).toBeVisible();
    await expect(detail.getByRole("combobox", { name: "Cấu hình ứng dụng" })).toBeVisible();
    const listBounds = await list.boundingBox();
    const detailBounds = await detail.boundingBox();
    expect(listBounds).not.toBeNull();
    expect(detailBounds).not.toBeNull();
    if (viewport.width > 1050) expect(listBounds!.x + listBounds!.width).toBeLessThanOrEqual(detailBounds!.x + 1);
    else expect(listBounds!.y + listBounds!.height).toBeLessThanOrEqual(detailBounds!.y + 1);
    expect(detailBounds!.x + detailBounds!.width).toBeLessThanOrEqual(viewport.width);
    if (viewport.width <= 1050) {
      const name = await list.getByText("Lịch buổi sáng").first().boundingBox();
      expect(name).not.toBeNull();
      expect(name!.x).toBeGreaterThanOrEqual(listBounds!.x);
      expect(await list.evaluate((element) => element.scrollWidth <= element.clientWidth + 1)).toBe(true);
    }
    await list.getByRole("button", { name: "Chỉnh sửa" }).click();
    await expect(detail.getByRole("combobox", { name: "Cấu hình ứng dụng" })).toHaveValue("profile-1");
    await page.screenshot({ path: test.info().outputPath(`schedules-${viewport.width}.png`), animations: "disabled" });
  });
}
