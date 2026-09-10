import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";

test("partial nurture start names excluded devices and counts only proven cleanup", async ({ page }) => {
  await installTauriMock(page, { androidRoster: true });
  await page.addInitScript(() => {
    const w = window as unknown as { __nurtureStartDuration?: unknown; __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } };
    const original = w.__TAURI_INTERNALS__.invoke;
    let started = false;
    w.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
      if (cmd === "nurture_get_settings") return { ...await original(cmd, args) as object, commentEnabled: false, commentProb: 0 };
      if (cmd === "nurture_save_settings") return args.settings;
      if (cmd === "nurture_session_log") return [{ at: "2026-09-07T00:00:01Z", lastAt: "2026-09-07T00:00:01Z", text: "Đã mở TikTok trong phiên mới", repeats: 1 }];
      if (cmd === "nurture_start") { started = true; w.__nurtureStartDuration = args.durationMinutes; return ["MOCK-ANDROID-01"]; }
      if (cmd === "nurture_session_status") return started ? [{ udid: "MOCK-ANDROID-01", runId: "fixture-run", runSize: 1, running: false, phase: "finished", outcome: "done", videoTarget: 1, videosDone: 1, likes: 0, saves: 0, comments: 0, follows: 0, sessionPromptTokens: 0, sessionCompletionTokens: 0, startedAt: "2026-09-07T00:00:00Z", updatedAt: "2026-09-07T00:01:00Z", cleanupState: "processAbsent", cleanupProof: { bundleId: "com.fixture", oldPid: 12 }, lastMessage: "done" }] : [];
      return original(cmd, args);
    };
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Nuôi TikTok", exact: true }).click();
  await page.getByRole("button", { name: /Cân bằng/ }).click();
  await page.getByRole("combobox", { name: "Phạm vi thiết bị" }).selectOption("all");
  await page.getByRole("button", { name: "Kiểm tra & bắt đầu", exact: true }).click();
  await page.getByRole("button", { name: "Bắt đầu 2 máy", exact: true }).click();
  await expect(page.getByText("1/2 máy đã bắt đầu", { exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __nurtureStartDuration: number }).__nurtureStartDuration)).toBe(20);
  await expect(page.getByText(/Không bắt đầu: Máy 2/)).toBeVisible();
  await expect(page.getByText("TikTok đã tắt: 1/1 máy trong phiên", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: /Máy 1/ }).click();
  await expect(page.getByText("Đã mở TikTok trong phiên mới", { exact: true })).toBeVisible();
  await expect(page.locator('.nurture-log-at')).toHaveText(/\d{2}:\d{2}:\d{2}/);
  for (const viewport of [{ width: 1440, height: 900 }, { width: 900, height: 900 }, { width: 820, height: 560 }]) {
    await page.setViewportSize(viewport);
    await page.getByText("1/2 máy đã bắt đầu", { exact: true }).scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`nurture-scope-${viewport.width}.png`) });
  }
});
