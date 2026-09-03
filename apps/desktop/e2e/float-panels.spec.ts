import { expect, test, type Page } from "@playwright/test";

import { installTauriMock } from "./fixtures/tauriMock";

/** Page workspaces keep their tabs reachable while only the content body scrolls. */
async function openWorkspace(page: Page, button: string, region: string): Promise<void> {
  await installTauriMock(page);
  await page.goto("/");
  await expect(page.locator("[data-testid='device-tile']")).toHaveCount(2);
  await page.locator("[data-testid='nav-item']").getByText(button, { exact: true }).click();
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

  const before = await edges(page, ".interaction-workspace-inner", ".interaction-tabs");
  expect(before.tabs).toBeGreaterThanOrEqual(before.card);

  // Every tile, so the run includes one far enough down the list that revealing its
  // off-screen checkbox would need a scroll.
  const tiles = page.locator(".interaction-workspace .tile-pick");
  const count = await tiles.count();
  expect(count).toBeGreaterThan(0);
  for (let i = 0; i < count; i += 1) {
    await tiles.nth(i).click({ force: true });
  }

  const after = await edges(page, ".interaction-workspace-inner", ".interaction-tabs");
  expect(after.scrollTop, "the workspace itself must never scroll").toBe(0);
  expect(
    after.tabs,
    "the tabs have to stay inside the workspace",
  ).toBeGreaterThanOrEqual(after.card);
  expect((await page.locator(".interaction-workspace").innerText()).trim().length).toBeGreaterThan(50);
});

test("the Tương tác body remains the designated scroller", async ({ page }) => {
  await openWorkspace(page, "Tương tác", "Không gian Tương tác");

  const scroll = await page.evaluate(() => {
    const body = document.querySelector(".interaction-float-body") as HTMLElement;
    const card = document.querySelector(".interaction-workspace-inner") as HTMLElement;
    card.scrollTop = 500;
    return {
      cardScrolled: Math.round(card.scrollTop),
      cardOverflow: getComputedStyle(card).overflowY,
      bodyOverflow: getComputedStyle(body).overflowY,
    };
  });
  expect(scroll.cardScrolled, "the card must not").toBe(0);
  expect(scroll.cardOverflow).toBe("clip");
  expect(scroll.bodyOverflow, "the body is the designated scroller").toBe("auto");
});

test("the Nuôi TikTok workspace cannot be scrolled as a whole", async ({ page }) => {
  await openWorkspace(page, "Nuôi TikTok", "Không gian Nuôi TikTok");

  const result = await page.evaluate(() => {
    const card = document.querySelector(".nurture-workspace-inner") as HTMLElement;
    const body = card.querySelector(".nurture-float-body") as HTMLElement;
    card.scrollTop = 500;
    const bodyStyle = getComputedStyle(body);
    return {
      cardScrollTop: Math.round(card.scrollTop),
      cardOverflow: getComputedStyle(card).overflowY,
      bodyOverflow: bodyStyle.overflowY,
    };
  });
  expect(result.cardScrollTop, "the workspace clips; it does not scroll").toBe(0);
  expect(result.cardOverflow, "`hidden` would leave it scrollable").toBe("clip");
  // Asserted as the rule rather than as "the body is scrolling right now": with the two
  // fixture phones this panel's content happens to fit, and a test that only passes when it
  // does not fit would say nothing on the day it fits.
  expect(result.bodyOverflow, "the body is the scroller").toBe("auto");
});
