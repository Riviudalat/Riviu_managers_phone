import { expect, test, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installTauriMock } from "./fixtures/tauriMock";
import { openOperatorPage } from "./fixtures/operatorNavigation";

type GoogleCall = { command: string; args: Record<string, unknown> };
type GoogleFixture = {
  googleCalls: GoogleCall[];
  completeGoogleBrowser: (selectedId?: string) => void;
};

async function installGoogleFixture(page: Page, connected = true, selectedFileId = "selected-file") {
  await installTauriMock(page, { androidRoster: true, fleetSize: 20 });
  await page.addInitScript(({ connected, selectedFileId }) => {
    const w = window as unknown as GoogleFixture & {
      __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> };
    };
    const original = w.__TAURI_INTERNALS__.invoke;
    w.googleCalls = [];
    const status = {
      configured: true, connected, active: connected, clientId: "fixture.apps.googleusercontent.com",
      pickerConfigured: true, phase: "idle", error: null as string | null,
      accountId: connected ? "account-a" : null, email: connected ? "operator@example.test" : null,
      selectedFileId: connected ? selectedFileId : null, selectedFileName: connected ? "Kết quả đăng bài" : null,
      sheetUrl: connected ? "https://docs.google.com/spreadsheets/d/selected-file/edit#gid=0" : null,
      writerId: connected ? "fixture-writer" : null,
    };
    const result = (book: string, gid: number) => ({
      sheetUrl: `https://docs.google.com/spreadsheets/d/${book}/edit#gid=${gid}`,
      spreadsheetId: book, sheetGid: gid, reportingEpoch: "fixture-epoch",
      readable: true, connectionVerified: true, reportingReady: true, layout: "internal", columns: [],
      message: "Đã xác minh đúng tệp và tab.",
    });
    w.completeGoogleBrowser = selectedId => {
      if (status.phase === "authorizing") {
        status.connected = true; status.accountId = "account-a"; status.email = "operator@example.test";
      } else if (status.phase === "picking" && selectedId) {
        status.selectedFileId = selectedId; status.selectedFileName = "Kết quả đăng bài";
      } else { throw Error("No matching browser operation to complete"); }
      status.phase = "idle";
    };
    w.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
      if (command === "publish_sheet_get_config") return {
        provider: status.active ? "googleDirect" : "appsScript", webhookUrl: "", hasToken: false,
        internalReporting: true, sheetUrl: status.sheetUrl ?? "",
      };
      if (command === "google_sheets_status") return { ...status };
      if (command.startsWith("google_sheets_") || command === "publish_sheet_check") w.googleCalls.push({ command, args });
      if (command === "google_sheets_login") { status.phase = "authorizing"; return { ...status }; }
      if (command === "google_sheets_pick_file") {
        if (!status.connected || status.phase !== "idle") throw Error("Picker requires idle authenticated session");
        status.phase = "picking"; return { ...status };
      }
      if (command === "google_sheets_cancel") { status.phase = "idle"; return { ...status }; }
      if (command === "google_sheets_connect") {
        if (status.phase !== "idle" || args.spreadsheetId !== status.selectedFileId || args.confirmed !== true) {
          throw Error("Connection must match the completed Picker grant");
        }
        const checked = result(String(args.spreadsheetId), Number(args.sheetId));
        status.active = true; status.sheetUrl = checked.sheetUrl; status.writerId = "fixture-writer";
        return checked;
      }
      if (command === "publish_sheet_check") {
        const url = new URL(String(args.sheetUrl));
        const gid = Number(new URLSearchParams(url.hash.slice(1)).get("gid") ?? url.searchParams.get("gid") ?? "0");
        const book = url.pathname.split("/")[3];
        const checked = result(book, gid);
        if (!status.active || checked.sheetUrl !== status.sheetUrl) throw Error("Check requires the exact active target");
        return checked;
      }
      if (command.startsWith("publish_create") || command === "publish_execute" || command === "publish_sheet_prepare"
        || command === "publish_sheet_save_config" || command === "google_sheets_list_tabs") {
        throw Error("Unexpected publication/legacy/tab-selector command in compact Google UI test");
      }
      return original(command, args);
    };
  }, { connected, selectedFileId });
  await page.goto("/");
  await openOperatorPage(page, "Đăng bài");
}

const calls = (page: Page, command: string) => page.evaluate(command =>
  (window as unknown as GoogleFixture).googleCalls.filter(call => call.command === command), command);

for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
  test(`compact Google controls connect the pasted exact gid at ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await installGoogleFixture(page);
    const panel = page.locator(".google-sheet-connection");
    const link = panel.getByRole("textbox", { name: "Link Google Sheet" });
    await expect(link).toBeEditable();
    await expect(panel.getByRole("button")).toHaveCount(2);
    await expect(panel.locator("input")).toHaveCount(1);
    await expect(panel.getByRole("combobox")).toHaveCount(0);
    await expect(panel.getByRole("checkbox")).toHaveCount(0);
    await expect(page.getByText(/Apps Script/)).toHaveCount(0);
    await link.fill("https://docs.google.com/spreadsheets/d/selected-file/edit#gid=7");
    await panel.getByRole("button", { name: "Kiểm tra kết nối", exact: true }).click();
    await expect(panel.locator(".publish-sheet-result.is-verified")).toBeVisible();
    expect(await calls(page, "google_sheets_connect")).toEqual([
      { command: "google_sheets_connect", args: { spreadsheetId: "selected-file", sheetId: 7, confirmed: true } },
    ]);
    expect(await calls(page, "google_sheets_pick_file")).toEqual([]);
    await expect(link).toHaveValue("https://docs.google.com/spreadsheets/d/selected-file/edit#gid=7");
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    expect((await new AxeBuilder({ page }).include(".google-sheet-connection").analyze()).violations).toEqual([]);
    await page.screenshot({ path: test.info().outputPath(`google-sheet-${viewport.width}.png`) });
  });
}

test("checking a new pasted file obtains its Picker grant and continues with its original gid", async ({ page }) => {
  await installGoogleFixture(page, false);
  const panel = page.locator(".google-sheet-connection");
  const link = panel.getByRole("textbox", { name: "Link Google Sheet" });
  await link.fill("https://docs.google.com/spreadsheets/d/new-file/edit#gid=12");
  await panel.getByRole("button", { name: "Đăng nhập Google", exact: true }).click();
  await expect.poll(() => calls(page, "google_sheets_login")).toHaveLength(1);
  await page.evaluate(() => (window as unknown as GoogleFixture).completeGoogleBrowser());
  await expect(panel.getByRole("button", { name: "Đăng nhập Google", exact: true })).toBeEnabled();
  expect(await calls(page, "google_sheets_pick_file")).toEqual([]);
  expect(await calls(page, "google_sheets_connect")).toEqual([]);
  await panel.getByRole("button", { name: "Kiểm tra kết nối", exact: true }).click();
  await expect.poll(() => calls(page, "google_sheets_pick_file")).toHaveLength(1);
  expect(await calls(page, "google_sheets_connect")).toEqual([]);
  await page.evaluate(() => (window as unknown as GoogleFixture).completeGoogleBrowser("new-file"));
  await expect(panel.locator(".publish-sheet-result.is-verified")).toBeVisible();
  expect(await calls(page, "google_sheets_connect")).toEqual([
    { command: "google_sheets_connect", args: { spreadsheetId: "new-file", sheetId: 12, confirmed: true } },
  ]);
  await expect(link).toHaveValue("https://docs.google.com/spreadsheets/d/new-file/edit#gid=12");
});

test("a mismatched Picker selection never connects or replaces the pasted target", async ({ page }) => {
  await installGoogleFixture(page);
  const panel = page.locator(".google-sheet-connection");
  const link = panel.getByRole("textbox", { name: "Link Google Sheet" });
  const url = "https://docs.google.com/spreadsheets/d/wanted-file/edit#gid=12";
  await link.fill(url);
  await panel.getByRole("button", { name: "Kiểm tra kết nối", exact: true }).click();
  await expect.poll(() => calls(page, "google_sheets_pick_file")).toHaveLength(1);
  await page.evaluate(() => (window as unknown as GoogleFixture).completeGoogleBrowser("other-file"));
  await expect(panel.getByRole("alert")).toBeVisible();
  await expect(link).toHaveValue(url);
  await expect(panel.locator(".publish-sheet-result.is-verified")).toHaveCount(0);
  expect(await calls(page, "google_sheets_connect")).toEqual([]);
});

test("an active target remains checkable after a different Picker file was last selected", async ({ page }) => {
  await installGoogleFixture(page, true, "other-file");
  const panel = page.locator(".google-sheet-connection");
  const link = panel.getByRole("textbox", { name: "Link Google Sheet" });
  await expect(link).toHaveValue("https://docs.google.com/spreadsheets/d/selected-file/edit#gid=0");
  await expect(panel.getByRole("button", { name: "Kiểm tra kết nối", exact: true })).toBeEnabled();
  await panel.getByRole("button", { name: "Kiểm tra kết nối", exact: true }).click();
  await expect(panel.locator(".publish-sheet-result.is-verified")).toBeVisible();
  expect(await calls(page, "google_sheets_connect")).toEqual([]);
  expect(await calls(page, "google_sheets_pick_file")).toEqual([]);
});

test("login remains an explicit browser action and cancelled login does not connect a Sheet", async ({ page }) => {
  await installGoogleFixture(page, false);
  const panel = page.locator(".google-sheet-connection");
  const link = panel.getByRole("textbox", { name: "Link Google Sheet" });
  await link.fill("https://docs.google.com/spreadsheets/d/wanted-file/edit#gid=12");
  await panel.getByRole("button", { name: "Đăng nhập Google", exact: true }).click();
  await expect.poll(() => calls(page, "google_sheets_login")).toHaveLength(1);
  await panel.getByRole("button", { name: /Hủy/ }).click();
  await expect.poll(() => calls(page, "google_sheets_cancel")).toHaveLength(1);
  await expect(panel.getByRole("button", { name: "Đăng nhập Google", exact: true })).toBeEnabled();
  expect(await calls(page, "google_sheets_connect")).toEqual([]);
  expect(await calls(page, "google_sheets_pick_file")).toEqual([]);
});
