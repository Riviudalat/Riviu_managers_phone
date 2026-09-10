import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";

for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
  test(`focus keys and tile handoff at ${viewport.width}`, async ({ page }, info) => {
    await page.setViewportSize(viewport);
    await installTauriMock(page, { androidRoster: true, fleetSize: 10 });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.goto("/");
    await expect(page.getByTestId("device-tile")).toHaveCount(10);
    await page.evaluate(() => {
      const app = window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> }; __FOCUS_CALLS__: { cmd: string; args: Record<string, unknown> }[] };
      app.__FOCUS_CALLS__ = [];
      const original = app.__TAURI_INTERNALS__.invoke;
      app.__TAURI_INTERNALS__.invoke = async (cmd, args = {}) => {
        if (["device_control_begin", "device_control_end", "device_key", "view_set_preset"].includes(cmd)) {
          app.__FOCUS_CALLS__.push({ cmd, args });
          return null;
        }
        return original(cmd, args);
      };
    });
    const first = page.getByTestId("device-tile").first();
    await first.getByRole("button", { name: /Mở màn hình/ }).click();
    const focus = page.locator(".focus-overlay");
    await expect(focus.getByTestId("focus-control-status")).toContainText("Điều khiển sẵn sàng");
    await expect(first.getByText("Đang mở phóng to")).toBeVisible();
    for (const name of ["Home", "Back", "Recents", "Giảm âm lượng", "Tăng âm lượng", "Nguồn"]) {
      await focus.getByRole("button", { name, exact: true }).click();
    }
    const calls = await page.evaluate(() => (window as unknown as { __FOCUS_CALLS__: { cmd: string; args: Record<string, unknown> }[] }).__FOCUS_CALLS__);
    expect(calls.filter(call => call.cmd === "device_key").map(call => call.args.key)).toEqual(["home", "back", "recents", "volumeDown", "volumeUp", "power"]);
    const bounds = await focus.locator(".focus-navbar").boundingBox();
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(viewport.height);
    await page.screenshot({ path: info.outputPath("focus-controls.png") });
    await focus.getByRole("button", { name: "Đóng", exact: true }).click();
    await expect(first.getByText("Đang mở phóng to")).toHaveCount(0);
    expect(errors).toEqual([]);
  });
}
