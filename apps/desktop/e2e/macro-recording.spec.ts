import { expect, test, type Page } from "@playwright/test";
import { emitRiviuEvent, installTauriMock } from "./fixtures/tauriMock";

interface MacroCall {
  command: string;
  args: Record<string, unknown>;
}

interface MacroFixtureWindow {
  __TAURI_INTERNALS__: {
    invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  };
  __MACRO_CALLS__: MacroCall[];
}

async function prepare(page: Page, viewport = { width: 1440, height: 900 }) {
  await page.setViewportSize(viewport);
  await installTauriMock(page, { androidRoster: true });
  // The fleet subscribes to roster events only after a successful startup check.
  await page.addInitScript(() => {
    const fixture = window as unknown as MacroFixtureWindow;
    const invoke = fixture.__TAURI_INTERNALS__.invoke;
    fixture.__TAURI_INTERNALS__.invoke = async (command, args = {}) =>
      command === "startup_error" ? null : invoke(command, args);
  });
  await page.goto("/");
  await expect(page.getByTestId("device-tile")).toHaveCount(2);
  await page.evaluate(() => {
    const fixture = window as unknown as MacroFixtureWindow;
    const invoke = fixture.__TAURI_INTERNALS__.invoke;
    fixture.__MACRO_CALLS__ = [];
    fixture.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
      if (["device_control_begin", "device_control_end", "device_key", "view_set_preset", "group_input"].includes(command)) {
        fixture.__MACRO_CALLS__.push({ command, args: structuredClone(args) });
        if (command === "group_input") {
          return { completedUdids: args.udids ?? [], skipped: [] };
        }
        return null;
      }
      return invoke(command, args);
    };
  });
}

test("macro keeps name and loops while switching and closing phones", async ({ page }) => {
  await prepare(page);
  await openMacro(page);
  await macroDialog(page).getByLabel("Tên macro", { exact: true }).fill("Bản nháp hai máy");
  await macroDialog(page).getByLabel("Số vòng lặp").fill("4");
  await beginRecording(page);
  await openPhone(page);
  await page.locator(".focus-overlay").getByRole("button", { name: "Home", exact: true }).click();
  await page.locator(".focus-overlay").getByRole("button", { name: "Đổi máy", exact: true }).click();
  await page.getByRole("group", { name: "Đổi máy", exact: true }).getByTitle("MOCK-ANDROID-02").click();
  await expect(page.getByRole("dialog", { name: "Điều khiển Máy Android 02", exact: true })).toBeVisible();
  await expect(recordingBar(page)).toHaveCount(1);
  await expect(recordingBar(page)).toContainText("1 bước");
  await page.locator(".focus-overlay").getByRole("button", { name: "Home", exact: true }).click();
  await expect(recordingBar(page)).toContainText("2 bước");
  await page.locator(".focus-overlay").getByRole("button", { name: "Đóng", exact: true }).click();
  await expect(recordingBar(page)).toHaveCount(1);
  await expect(page.locator(".focus-menu")).toHaveCount(0);
  await expect(recordingBar(page)).toContainText("2 bước");
  await recordingBar(page).getByRole("button", { name: "Dừng ghi", exact: true }).click();
  await expect(macroDialog(page).getByLabel("Tên macro", { exact: true })).toHaveValue("Bản nháp hai máy");
  await expect(macroDialog(page).getByLabel("Số vòng lặp")).toHaveValue("4");
  await expect(macroDialog(page).getByLabel("Tên macro", { exact: true })).toBeFocused();
  expect((await calls(page)).filter(call => call.command === "device_key").map(call => call.args.udid)).toEqual([
    "MOCK-ANDROID-01", "MOCK-ANDROID-02",
  ]);
  expect((await calls(page)).filter(call => call.command === "group_input")).toHaveLength(0);
});

test("empty recording returns only Macro on another page and later tools open normally", async ({ page }) => {
  await prepare(page);
  await openMacro(page);
  await beginRecording(page);
  const navigation = page.getByRole("navigation", { name: "Điều hướng chính" });
  await navigation.getByRole("button", { name: "Cài đặt", exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Cài đặt", exact: true })).toBeVisible();
  await expect(recordingBar(page)).toHaveCount(1);
  await expect(recordingBar(page)).toContainText("0 bước");
  await recordingBar(page).getByRole("button", { name: "Dừng ghi", exact: true }).click();
  await expect(macroDialog(page)).toBeVisible();
  await expect(macroDialog(page).getByRole("tab")).toHaveCount(1);
  await expect(macroDialog(page).getByRole("tab", { name: "Macro", exact: true })).toHaveAttribute("aria-selected", "true");
  await expect(macroDialog(page)).toContainText("Chưa có bước nào");
  await page.keyboard.press("Escape");
  await expect(macroDialog(page)).toHaveCount(0);
  await navigation.getByRole("button", { name: "Thiết bị", exact: true }).click();
  await page.getByRole("button", { name: "Công cụ", exact: true }).click();
  await expect(macroDialog(page).getByRole("tab")).toHaveCount(8);
  await expect(macroDialog(page).getByRole("tab", { name: "Phân phối văn bản", exact: true })).toHaveAttribute("aria-selected", "true");
  expect((await calls(page)).filter(call => call.command === "group_input" || call.command === "device_key")).toHaveLength(0);
});

test("macro playback retains the targets frozen before selection changes", async ({ page }) => {
  await prepare(page);
  await page.getByTestId("device-tile").first().click();
  await expect(page.getByTestId("device-tile").first()).toHaveAttribute("aria-selected", "true");
  await openMacro(page);
  await beginRecording(page);
  await page.getByTestId("device-tile").nth(1).click();
  await expect(page.getByTestId("device-tile").nth(1)).toHaveAttribute("aria-selected", "true");
  await openPhone(page, 1);
  await page.locator(".focus-overlay").getByRole("button", { name: "Home", exact: true }).click();
  await recordingBar(page).getByRole("button", { name: "Dừng ghi", exact: true }).click();
  await macroDialog(page).getByLabel("Tên macro", { exact: true }).fill("Phạm vi ban đầu");
  await macroDialog(page).getByRole("button", { name: "Lưu macro", exact: true }).click();
  expect((await calls(page)).filter(call => call.command === "group_input")).toHaveLength(0);
  await macroDialog(page).getByRole("button", { name: "Chạy", exact: true }).click();
  await expect.poll(async () => (await calls(page)).filter(call => call.command === "group_input")).toEqual([
    expect.objectContaining({ args: expect.objectContaining({ udids: ["MOCK-ANDROID-01"], kind: "key", key: "home" }) }),
  ]);
});

test("recording survives device disappearance and empty frozen scope never expands", async ({ page }) => {
  await prepare(page);
  const roster = await page.evaluate(() => (window as unknown as MacroFixtureWindow).__TAURI_INTERNALS__.invoke("list_devices"));
  await openMacro(page);
  await emitRiviuEvent(page, { type: "devicesUpdated", devices: [] });
  await expect(page.getByTestId("device-tile")).toHaveCount(0);
  await beginRecording(page);
  await emitRiviuEvent(page, { type: "devicesUpdated", devices: roster });
  await expect(page.getByTestId("device-tile")).toHaveCount(2);
  await openPhone(page);
  await page.locator(".focus-overlay").getByRole("button", { name: "Home", exact: true }).click();
  await emitRiviuEvent(page, { type: "devicesUpdated", devices: [] });
  await expect(page.locator(".focus-overlay")).toHaveCount(0);
  await expect(recordingBar(page)).toHaveCount(1);
  await expect(recordingBar(page)).toContainText("1 bước");
  await emitRiviuEvent(page, { type: "devicesUpdated", devices: roster });
  await expect(page.getByTestId("device-tile")).toHaveCount(2);
  await recordingBar(page).getByRole("button", { name: "Dừng ghi", exact: true }).click();
  await macroDialog(page).getByLabel("Tên macro", { exact: true }).fill("Không có máy đích");
  await macroDialog(page).getByRole("button", { name: "Lưu macro", exact: true }).click();
  await macroDialog(page).getByRole("button", { name: "Chạy", exact: true }).click();
  await expect(page.getByText("Chưa có máy", { exact: true })).toBeVisible();
  expect((await calls(page)).filter(call => call.command === "group_input")).toHaveLength(0);
});

function macroDialog(page: Page) {
  return page.getByRole("dialog", { name: "Công cụ nhóm", exact: true });
}

function recordingBar(page: Page) {
  return page.getByRole("region", { name: "Ghi Macro", exact: true });
}

async function openMacro(page: Page) {
  await page.getByRole("button", { name: "Công cụ", exact: true }).click();
  await macroDialog(page).getByRole("tab", { name: "Macro", exact: true }).click();
}

async function beginRecording(page: Page) {
  await macroDialog(page).getByRole("button", { name: "Bắt đầu ghi", exact: true }).click();
  await expect(macroDialog(page)).toBeHidden();
  await expect(recordingBar(page)).toHaveCount(1);
  await expect(recordingBar(page)).toContainText("0 bước");
}

async function openPhone(page: Page, index = 0) {
  await page.getByTestId("device-tile").nth(index).getByRole("button", { name: /Mở màn hình/ }).click();
  await expect(page.locator(".focus-overlay").getByTestId("focus-control-status")).toContainText("Điều khiển sẵn sàng");
}

async function calls(page: Page) {
  return page.evaluate(() => (window as unknown as MacroFixtureWindow).__MACRO_CALLS__);
}

for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
  test(`macro recording hands off to phone and saves at ${viewport.width}`, async ({ page }, info) => {
    await prepare(page, viewport);
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    await openMacro(page);
    await macroDialog(page).getByLabel("Số vòng lặp").fill("3");
    await beginRecording(page);
    await openPhone(page);
    await expect(page.locator(".focus-menu").getByRole("region", { name: "Ghi Macro" })).toBeVisible();
    await expect(recordingBar(page)).toHaveCount(1);
    await page.locator(".focus-overlay").getByRole("button", { name: "Home", exact: true }).click();
    await expect(recordingBar(page)).toContainText("1 bước");
    await expect(recordingBar(page).getByRole("button", { name: "Dừng ghi", exact: true })).toBeInViewport();
    await page.screenshot({ path: info.outputPath("macro-recording-phone.png") });
    await recordingBar(page).getByRole("button", { name: "Dừng ghi", exact: true }).click();
    await expect(macroDialog(page)).toBeVisible();
    await expect(page.locator(".focus-overlay")).toBeVisible();
    await expect(recordingBar(page)).toHaveCount(0);
    await expect(macroDialog(page).getByLabel("Tên macro", { exact: true })).toBeFocused();
    await expect(macroDialog(page).getByLabel("Số vòng lặp")).toHaveValue("3");
    await macroDialog(page).getByLabel("Tên macro", { exact: true }).fill("Về màn hình chính");
    await macroDialog(page).getByRole("button", { name: "Lưu macro", exact: true }).click();
    await expect(macroDialog(page)).toContainText("Macro đã lưu (1)");
    expect(await page.evaluate(() => JSON.parse(localStorage.getItem("riviu.macros") ?? "[]"))).toEqual([
      expect.objectContaining({ name: "Về màn hình chính", steps: [{ kind: "key", key: "home", afterMs: 0 }] }),
    ]);
    await page.keyboard.press("Escape");
    await expect(macroDialog(page)).toHaveCount(0);
    await expect(page.locator(".focus-overlay")).toBeVisible();
    expect(await page.evaluate(() => Boolean(document.activeElement?.closest(".focus-overlay")))).toBe(true);
    await page.keyboard.press("Escape");
    await expect(page.locator(".focus-overlay")).toHaveCount(0);
    expect((await calls(page)).filter(call => call.command === "device_key")).toEqual([
      { command: "device_key", args: { udid: "MOCK-ANDROID-01", key: "home" } },
    ]);
    expect((await calls(page)).filter(call => call.command === "group_input")).toHaveLength(0);
    expect(errors).toEqual([]);
  });
}
