import { expect, test, type Page } from "@playwright/test";

import { installTauriMock, mockCommandCalls } from "./fixtures/tauriMock";

/**
 * One baseline per page in the sidebar.
 *
 * Until now the whole app had two screenshots, both of the Flow workspace, so eight of
 * the nine pages had no visual coverage at all. That is the reason the design pass could
 * tokenise colours and collapse the type scale but had to stop before touching spacing:
 * with nothing watching those pages, a sweep across 298 padding and gap declarations is
 * a change nobody can see until an operator does.
 *
 * These are not assertions about what the design *should* be. They are a record of what
 * it *is*, so the next change to it is reviewable.
 */
const PAGES = [
  "Quản lý cửa sổ",
  "Kho nội dung",
  "Trung tâm ứng dụng",
  "Flow",
  "Tác vụ",
  "Đăng bài",
  "Chẩn đoán",
  "Dữ liệu",
  "API",
  "Cài đặt",
] as const;

async function open(page: Page, name: string): Promise<void> {
  await installTauriMock(page, {
    androidRoster: name === "Chẩn đoán" || name === "Trung tâm ứng dụng",
  });
  await page.goto("/");
  // Wait for the fleet the mock serves, so no page is captured mid-bootstrap.
  await expect(page.locator("[data-testid='device-tile']")).toHaveCount(2);
  if (name !== "Quản lý cửa sổ") {
    await page.locator("[data-testid='nav-item']").getByText(name, { exact: true }).click();
  }
  // **Loaded before pixels, and the Flow baseline is why this line exists.** Waiting for
  // the fleet says the shell is up; it says nothing about a page that fetches its own
  // content afterwards. The Flow page shows `LoadingState` while it does, and the committed
  // baseline had captured exactly that — a spinner, blessed as "the Flow page". It proved
  // nothing about the page, and it made the test a race: whoever won it decided whether CI
  // was green. It went red the first time a runner loaded faster than the machine the
  // baseline was taken on.
  //
  // `.loading-state` is the shared component every page uses, so this covers the pages that
  // grow one later rather than only the one that has it today.
  await expect(page.locator(".loading-state")).toHaveCount(0);
  // Fonts before pixels: a screenshot taken while a face is still loading captures the
  // fallback, which is what made the two Flow baselines racy before they were bundled.
  await page.evaluate(() => document.fonts.ready);
  await expect(page.locator(".toast-error")).toHaveCount(0);
  await expect(page.getByText(/Unknown mock command/i)).toHaveCount(0);
  if (name === "Đăng bài") {
    await expect(page.getByText("Phạm vi: 2 máy", { exact: true })).toBeVisible();
  }
}

function screenshotName(name: string): string {
  return name.normalize("NFD").replace(/[\u0300-\u036f]/g, "").replace(/[đĐ]/g, "d").replace(/\s+/g, "-").toLowerCase();
}

test("the Android app library dispatches and renders one fleet batch", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await open(page, "Trung tâm ứng dụng");

  await page.getByRole("button", { name: "Cài → 2 Android" }).click();

  await expect(page.getByRole("heading", { name: "Kết quả cài đặt" })).toBeVisible();
  await expect(page.getByText("Đã xác nhận")).toHaveCount(2);
  const install = (await mockCommandCalls(page)).find(
    (call) => call.command === "install_library_app_batch",
  );
  expect(install?.args).toMatchObject({
    request: {
      appId: "fixture-app",
      udids: ["MOCK-ANDROID-01", "MOCK-ANDROID-02"],
      allowDowngrade: false,
    },
  });
});

test("fleet diagnostics probes each Android and retries only the chosen row", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 900 });
  await open(page, "Chẩn đoán");

  const healthCalls = () => mockCommandCalls(page).then((calls) =>
    calls.filter((call) => call.command === "device_health")
  );
  const beforeRetry = await healthCalls();
  expect(new Set(beforeRetry.map((call) => call.args.udid))).toEqual(
    new Set(["MOCK-ANDROID-01", "MOCK-ANDROID-02"]),
  );
  await page.getByRole("button", { name: /Kiểm lại Máy 1/ }).click();
  await expect.poll(async () => (await healthCalls()).length).toBe(beforeRetry.length + 1);
  expect((await healthCalls()).slice(beforeRetry.length)).toEqual([
    expect.objectContaining({ args: { udid: "MOCK-ANDROID-01" } }),
  ]);
});

test.describe("every page in the sidebar", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
  });

  for (const name of PAGES) {
    test(`renders ${name}`, async ({ page }) => {
      await open(page, name);
      await expect(page.locator(".riviu-shell, #root")).toBeVisible();
      await expect(page).toHaveScreenshot(
        `page-${screenshotName(name)}.png`,
        {
          fullPage: false,
          // The same tolerance the Flow baselines use: antialiasing differs by a few
          // pixels between runs and is not what these are watching for.
          maxDiffPixelRatio: 0.002,
          animations: "disabled",
        },
      );
    });
  }

  for (const name of PAGES) {
    test(`renders ${name} in the narrow operator viewport`, async ({ page }) => {
      await page.setViewportSize({ width: 900, height: 900 });
      await open(page, name);
      await expect(page.locator(".riviu-shell, #root")).toBeVisible();
      await expect(page).toHaveScreenshot(`page-narrow-${screenshotName(name)}.png`, {
        fullPage: false,
        maxDiffPixelRatio: 0.002,
        animations: "disabled",
      });
    });
  }
});
