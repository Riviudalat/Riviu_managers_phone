import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";

test("progress expands to per-device timed logs, stays available after navigation and fits narrow screens", async ({ page }) => {
  await installTauriMock(page, { androidRoster: true });
  await page.addInitScript(() => {
    const w = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> } };
    const invoke = w.__TAURI_INTERNALS__.invoke;
    const summary = { id: "publish:progress", sourceId: "progress", kind: "publish", title: "Đăng bài", state: "running", targetCount: 2, totalItems: 2, completedItems: 1, issueCount: 0, retryableCount: 0, retryScope: null, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() };
    w.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "operation_query_runs") return { runs: [summary], counts: { active: 1, succeeded: 0, attention: 0 }, total: 1, hasMore: false };
      if (command === "operation_get_run") return { summary, items: ["MOCK-ANDROID-01", "MOCK-ANDROID-02"].map((udid, index) => ({ id: udid, udid, kind: "assignment", label: `Bài ${index + 1}`, state: index ? "running" : "succeeded", errorCode: null, detail: null, evidence: null, retryable: false })) };
      if (command === "operation_device_log") return { entries: [
        { id: "one", at: "2026-09-07T12:34:56", action: "publish", state: "transferring", text: null, detail: null },
        { id: "two", at: "2026-09-07T12:35:12", action: "publish", state: "posting", text: null, detail: null },
      ], truncated: false };
      return invoke(command, args);
    };
  });
  await page.goto("/");
  const center = page.locator(".run-monitor");
  await expect(center.getByRole("progressbar", { name: "Tiến độ công việc" })).toHaveAttribute("aria-valuenow", "50");
  await page.getByLabel("Tiến trình công việc", { exact: true }).press("Enter");
  await center.getByRole("button", { name: /Máy 1/ }).click();
  await expect(center.getByText("12:34:56", { exact: true })).toBeVisible();
  await expect(center.getByText("Đang tải ảnh/video vào điện thoại", { exact: true })).toBeVisible();
  for (const viewport of [{ width: 1440, height: 900 }, { width: 900, height: 900 }, { width: 820, height: 560 }, { width: 390, height: 844 }]) {
    await page.setViewportSize(viewport);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await expect(center.getByRole("region", { name: "Chi tiết máy", exact: true })).toBeVisible();
    await page.screenshot({ path: test.info().outputPath(`progress-${viewport.width}.png`), fullPage: true });
  }
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.getByRole("button", { name: "Tác vụ", exact: true }).click();
  await expect(center.getByText("12:34:56", { exact: true })).toBeVisible();
  await center.getByRole("button", { name: "Thu nhỏ tiến trình" }).press("Escape");
  await expect(center).toHaveClass(/is-minimized/);
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("landscape decoder frame retains aspect and a usable menu across viewport sizes", async ({ page }) => {
  await installTauriMock(page, { androidRoster: true });
  // The transport fixture paints a nonblank diagnostic bitmap through the real canvas store.
  await page.addInitScript(() => {
    const runtime = (window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> } }).__TAURI_INTERNALS__;
    const invoke = runtime.invoke;
    runtime.invoke = async (command, args) => ["device_control_begin", "device_control_end"].includes(command) ? null : invoke(command, args);
    class DecodeFixture {
      onmessage: ((event: { data: unknown }) => void) | null = null;
      postMessage(message: { type: string; udid: string; canvas?: OffscreenCanvas }) {
        if (message.type !== "attach" || !message.canvas) return;
        const canvas = message.canvas;
        canvas.width = 832; canvas.height = 400;
        const context = canvas.getContext("2d")!;
        context.fillStyle = "#f4f6f8"; context.fillRect(0, 0, 832, 400);
        context.fillStyle = "#c2410c"; context.fillRect(40, 40, 360, 220);
        context.fillStyle = "#17483a"; context.fillRect(440, 70, 350, 250);
        setTimeout(() => this.onmessage?.({ data: { type: "painted", udid: message.udid, width: 832, height: 400, generation: 1 } }), 0);
      }
      terminate() {}
    }
    Object.defineProperty(window, "Worker", { value: DecodeFixture });
  });
  await page.goto("/");
  await page.locator("[data-testid='device-tile']").first().dblclick();
  const stage = page.locator(".focus-stage");
  await expect(stage).toHaveClass(/is-landscape/);
  await expect(page.getByRole("button", { name: "Đưa về màn hình dọc", exact: true })).toBeVisible();
  for (const viewport of [{ width: 1440, height: 900 }, { width: 900, height: 900 }, { width: 820, height: 560 }, { width: 390, height: 844 }]) {
    await page.setViewportSize(viewport);
    const pane = await page.getByTestId("focus-screen").boundingBox();
    const menu = await page.locator(".focus-menu").boundingBox();
    const bounds = await stage.boundingBox();
    expect(pane!.width / pane!.height).toBeCloseTo(832 / 400, 1);
    expect(menu!.height).toBeGreaterThanOrEqual(250);
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(viewport.width);
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(viewport.height);
    await page.screenshot({ path: test.info().outputPath(`landscape-${viewport.width}.png`) });
  }
  await expect(page.getByRole("alert")).toHaveCount(0);
});
