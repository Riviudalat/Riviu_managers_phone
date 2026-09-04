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
  "Thiết bị",
  "Chẩn đoán",
  "Nuôi TikTok",
  "Tương tác",
  "Đăng bài",
  "Flow",
  "Tác vụ",
  "Kho nội dung",
  "Trung tâm ứng dụng",
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
  if (name !== "Thiết bị") {
    await page.getByRole("button", { name, exact: true }).click();
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
    await expect(page.getByRole("status").filter({ hasText: "Toàn bộ 2" })).toBeVisible();
  }
  if (name === "Flow") {
    const runButton = page.getByRole("button", { name: "Chạy Flow" });
    await expect(runButton).toBeVisible();
    const box = await runButton.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(page.viewportSize()!.width);
    const geometry = await page.getByTestId("flow-toolbar").evaluate((toolbar) => {
      const layout = document.querySelector<HTMLElement>(".flow-layout");
      if (!layout) throw new Error("Flow layout is missing");
      const toolbarRect = toolbar.getBoundingClientRect();
      const layoutRect = layout.getBoundingClientRect();
      const childBottom = Math.max(
        ...Array.from(toolbar.children, (child) => child.getBoundingClientRect().bottom),
      );
      return {
        toolbarBottom: toolbarRect.bottom,
        toolbarHeight: toolbarRect.height,
        childBottom,
        layoutTop: layoutRect.top,
      };
    });
    expect(geometry.toolbarBottom, JSON.stringify(geometry)).toBeGreaterThanOrEqual(
      geometry.childBottom,
    );
    expect(geometry.layoutTop, JSON.stringify(geometry)).toBeGreaterThanOrEqual(
      geometry.toolbarBottom,
    );
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
  await expect(page.getByRole("table", { name: "Kết quả cài đặt gần nhất" })
    .getByText("Đã xác nhận", { exact: true })).toHaveCount(2);
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

test("the material library dispatches one bounded fleet batch", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await open(page, "Kho nội dung");

  await page.getByRole("button", { name: "Chuyển tới 2 máy" }).click();

  await expect(page.getByRole("region", { name: "Kết quả chuyển gần nhất" })).toBeVisible();
  await expect(page.getByText("Đã chuyển")).toHaveCount(2);
  const push = (await mockCommandCalls(page)).find(
    (call) => call.command === "push_material_batch",
  );
  expect(push?.args).toEqual({
    request: {
      materialId: "fixture-material",
      target: { type: "all" },
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

test("publish keeps setup separate from campaign monitoring", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await open(page, "Đăng bài");

  await expect(page.getByRole("tabpanel", { name: "Thiết lập" })).toBeVisible();
  await expect(page.getByRole("tabpanel", { name: "Theo dõi" })).toBeHidden();

  await page.getByRole("tab", { name: "Theo dõi" }).click();
  await expect(page.getByRole("tabpanel", { name: "Thiết lập" })).toBeHidden();
  await expect(page.getByRole("tabpanel", { name: "Theo dõi" })).toBeVisible();
  await expect(page.getByText("Chưa có chiến dịch")).toBeVisible();
  await expect(page.locator(".toast-error")).toHaveCount(0);
  await expect(page.getByText(/Unknown mock command/i)).toHaveCount(0);
});

test("publish workflow stays inside the viewport at supported widths", async ({ page }) => {
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 900, height: 900 },
    { width: 820, height: 560 },
  ]) {
    await page.setViewportSize(viewport);
    await open(page, "Đăng bài");
    await expect(page.getByRole("heading", { level: 1, name: "Đăng bài" })).toHaveCount(1);
    const workflow = page.getByRole("list", { name: "Quy trình đăng bài" });
    for (const label of ["Nguồn", "Ghép bài/máy", "Preflight", "Xác nhận công khai", "Theo dõi"]) {
      await expect(workflow).toContainText(label);
    }
    const overflow = await page.evaluate(() => ({
      viewport: document.documentElement.clientWidth,
      document: document.documentElement.scrollWidth,
      page: document.querySelector<HTMLElement>(".publish-page")?.scrollWidth ?? 0,
      pageClient: document.querySelector<HTMLElement>(".publish-page")?.clientWidth ?? 0,
    }));
    expect(overflow.document, JSON.stringify(overflow)).toBeLessThanOrEqual(overflow.viewport);
    expect(overflow.page, JSON.stringify(overflow)).toBeLessThanOrEqual(overflow.pageClient);
  }
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

  for (const name of PAGES) {
    test(`renders ${name} in the compact operator viewport`, async ({ page }) => {
      await page.setViewportSize({ width: 820, height: 560 });
      await open(page, name);

      await expect(page.getByRole("heading", { level: 1 })).toHaveCount(1);
      const widths = await page.evaluate(() => {
        const root = document.documentElement;
        const main = document.querySelector<HTMLElement>(".main-col");
        const content = document.querySelector<HTMLElement>(".content");
        return {
          viewport: root.clientWidth,
          document: root.scrollWidth,
          main: main?.scrollWidth ?? 0,
          mainClient: main?.clientWidth ?? 0,
          content: content?.scrollWidth ?? 0,
          contentClient: content?.clientWidth ?? 0,
        };
      });
      expect(widths.document, JSON.stringify(widths)).toBeLessThanOrEqual(widths.viewport);
      expect(widths.main, JSON.stringify(widths)).toBeLessThanOrEqual(widths.mainClient);
      expect(widths.content, JSON.stringify(widths)).toBeLessThanOrEqual(widths.contentClient);

      await expect(page).toHaveScreenshot(`page-compact-${screenshotName(name)}.png`, {
        fullPage: false,
        maxDiffPixelRatio: 0.002,
        animations: "disabled",
      });
    });
  }
});
