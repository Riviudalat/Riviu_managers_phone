import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";
for (const width of [1440, 820]) {
  test(`GUI perception configuration at ${width}px`, async ({ page }, info) => {
    await page.setViewportSize({ width, height: width === 820 ? 560 : 900 });
    await installTauriMock(page);
    await page.addInitScript(() => {
      const w = window as unknown as {
        __TAURI_INTERNALS__: {
          invoke: (
            command: string,
            args?: Record<string, unknown>,
          ) => Promise<unknown>;
        };
      };
      const original = w.__TAURI_INTERNALS__.invoke;
      let config = { enabled: true, baseUrl: "", model: "", maxRequests: 20 };
      w.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (command === "gui_service_status")
          return {
            config,
            running: false,
            providerReady: false,
            protocolVersion: 1,
          };
        if (command === "gui_service_save") {
          config = args?.config as typeof config;
          return;
        }
        if (command === "gui_service_check")
          return "Dịch vụ đã sẵn sàng, protocol v1 khớp.";
        return original(command, args);
      };
    });
    await page.goto("/");
    await page.getByRole("button", { name: "Cài đặt", exact: true }).click();
    await page.getByRole("link", { name: "Kết nối và API" }).click();
    const section = page.getByRole("region", {
      name: "Nhận diện giao diện",
      exact: true,
    });
    await expect(section).toBeVisible();
    await section.getByLabel("Giới hạn request mỗi phiên").fill("12");
    await section
      .getByRole("button", { name: "Lưu cấu hình nhận diện" })
      .click();
    await expect(section.getByText("Đã lưu cấu hình nhận diện.")).toBeVisible();
    await section.getByRole("button", { name: "Kiểm tra dịch vụ" }).click();
    await expect(section.getByText(/Dịch vụ đã sẵn sàng/)).toBeVisible();
    await section.scrollIntoViewIfNeeded();
    await page.screenshot({
      path: info.outputPath(`gui-service-${width}.png`),
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
  });
}
