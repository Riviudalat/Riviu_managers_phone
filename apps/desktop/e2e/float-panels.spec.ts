import { expect, test, type Page } from "@playwright/test";

import { installTauriMock } from "./fixtures/tauriMock";
import { openOperatorPage } from "./fixtures/operatorNavigation";

/** Page workspaces keep their tabs reachable while only the content body scrolls. */
async function openWorkspace(page: Page, button: string, region: string): Promise<void> {
  await installTauriMock(page);
  if (button === "Tương tác") await page.addInitScript(() => {
    const w = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> } };
    const invoke = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "interaction_parse_links") return [{ lineNo: 1, original: args.rawText, target: { originalUrl: args.rawText, normalizedUrl: args.rawText, author: "fixture", kind: "video", contentId: "1234567890123456789", targetKey: "content:1234567890123456789" }, error: null }];
      return invoke(command, args);
    };
  });
  await page.goto("/");
  await expect(page.locator("[data-testid='device-tile']")).toHaveCount(2);
  await openOperatorPage(page,button);
  await expect(page.getByRole("region", { name: region })).toBeVisible();
}

/** The tab strip's top edge, and the workspace's, in viewport coordinates. */
async function edges(page: Page, card: string, tabs: string) {
  return page.evaluate(
    ([cardSel, tabSel]) => {
      const cardEl = document.querySelector(cardSel) as HTMLElement;
      const tabEl = cardEl.querySelector(tabSel) as HTMLElement;
      return {
        card: Math.round(cardEl.getBoundingClientRect().top),
        tabs: Math.round(tabEl.getBoundingClientRect().top),
        scrollTop: Math.round(cardEl.scrollTop),
      };
    },
    [card, tabs],
  );
}

test("picking phones in Tương tác never scrolls its tabs away", async ({ page }) => {
  await openWorkspace(page, "Tương tác", "Không gian Tương tác");
  await page.getByPlaceholder("Dán link TikTok, mỗi dòng một bài").fill("https://www.tiktok.com/@fixture/video/1234567890123456789");
  await page.getByRole("button", { name: "Chọn hành động & máy →" }).click();
  await page.getByRole("combobox", { name: "Phạm vi thiết bị" }).selectOption("all");
  await expect(page.getByRole("combobox", { name: "Phạm vi thiết bị" })).toHaveValue("all");

  const before = await edges(page, ".interaction-workspace-inner", ".automation-page-tabs");
  expect(before.tabs).toBeGreaterThanOrEqual(before.card);

  // Real checkbox interaction may scroll the picker; the page tabs stay fixed.
  const choose = page.getByRole("button", { name: "Chọn máy", exact: true });
  await choose.click();
  const picker = page.getByRole("dialog", { name: "Chọn máy Tương tác", exact: true });
  const tiles = picker.getByRole("group", { name: "Danh sách máy thực hiện", exact: true }).getByRole("checkbox");
  const count = await tiles.count();
  expect(count).toBeGreaterThan(0);
  for (let i = 0; i < count; i += 1) {
    await tiles.nth(i).click();
  }
  await picker.getByRole("button", { name: "Xong", exact: true }).click();
  await expect(choose).toBeFocused();

  const after = await edges(page, ".interaction-workspace-inner", ".automation-page-tabs");
  expect(after.scrollTop, "the workspace itself must never scroll").toBe(0);
  expect(
    after.tabs,
    "the tabs have to stay inside the workspace",
  ).toBeGreaterThanOrEqual(after.card);
  expect((await page.locator(".interaction-workspace").innerText()).trim().length).toBeGreaterThan(50);
});

test("the Tương tác panels scroll while wizard tabs and actions stay fixed", async ({ page }) => {
  await openWorkspace(page, "Tương tác", "Không gian Tương tác");

  const scroll = await page.evaluate(() => {
    const body = document.querySelector(".interaction-float-body") as HTMLElement;
    const card = document.querySelector(".interaction-workspace-inner") as HTMLElement;
    const panel = document.querySelector(".iw-links") as HTMLElement;
    const table = document.querySelector(".iw-links .iw-table-scroll") as HTMLElement;
    const footer = document.querySelector(".iw-footer")!.getBoundingClientRect();
    const stage = document.querySelector(".iw-stage")!.getBoundingClientRect();
    card.scrollTop = 500;
    body.scrollTop = 500;
    return {
      cardScrolled: Math.round(card.scrollTop),
      bodyScrolled: Math.round(body.scrollTop),
      footerFits: footer.bottom <= innerHeight,
      panelsAboveFooter: stage.bottom <= footer.top,
      cardOverflow: getComputedStyle(card).overflowY,
      bodyOverflow: getComputedStyle(body).overflowY,
      panelOverflow: getComputedStyle(panel).overflowY,
      tableOverflow: getComputedStyle(table).overflowY,
    };
  });
  expect(scroll.cardScrolled, "the card must not").toBe(0);
  expect(scroll.cardOverflow).toBe("clip");
  expect(scroll.bodyOverflow, "the body bounds the panels without scrolling its controls").toBe("hidden");
  expect(scroll.bodyScrolled, "the body itself has no overflow to scroll").toBe(0);
  expect(scroll.footerFits, "the actions remain in the viewport").toBe(true);
  expect(scroll.panelsAboveFooter, "panels never cover the actions").toBe(true);
  expect(scroll.panelOverflow, "the content panel scrolls").toBe("auto");
  expect(scroll.tableOverflow, "long lists scroll inside their panel").toBe("auto");
});

test("the Nuôi TikTok workspace cannot be scrolled as a whole", async ({ page }) => {
  await openWorkspace(page, "Nuôi TikTok", "Không gian Nuôi TikTok");
  const choose = page.getByRole("button", { name: "Chọn máy", exact: true });
  await choose.click();
  const picker = page.getByRole("dialog", { name: "Chọn máy Nuôi TikTok", exact: true });
  await expect(picker.getByRole("group", { name: "Danh sách chọn máy Nuôi TikTok" })).toBeVisible();

  const result = await page.evaluate(() => {
    const card = document.querySelector(".nurture-workspace-inner") as HTMLElement;
    const body = card.querySelector(".nurture-float-body") as HTMLElement;
    card.scrollTop = 500;
    const bodyStyle = getComputedStyle(body);
    return {
      cardScrollTop: Math.round(card.scrollTop),
      cardOverflow: getComputedStyle(card).overflowY,
      bodyOverflow: bodyStyle.overflowY,
      settingsOverflow: getComputedStyle(card.querySelector(".nurture-setup-fields")!).overflowY,
      machinesOverflow: getComputedStyle(card.querySelector(".nurture-machine-grid")!).overflowY,
    };
  });
  expect(result.cardScrollTop, "the workspace clips; it does not scroll").toBe(0);
  expect(result.cardOverflow, "`hidden` would leave it scrollable").toBe("clip");
  expect(result.bodyOverflow, "the body keeps tabs and footer fixed").toBe("hidden");
  expect(result.settingsOverflow).toBe("auto");
  expect(result.machinesOverflow).toBe("auto");
  await picker.getByRole("button", { name: "Xong", exact: true }).click();
  await expect(choose).toBeFocused();
});
