import { expect, test, type Page } from "@playwright/test";

import { installTauriMock } from "./fixtures/tauriMock";

/**
 * A floating panel must clip, never scroll.
 *
 * Both float panels are taller than the height they are clamped to, and both used
 * `overflow: hidden` — which clips *and* leaves the box programmatically scrollable. So the
 * first time focus landed on a control the browser could not see, it scrolled **the card**
 * to reveal it, taking the header and the tab strip out of the clip box. There is no
 * scrollbar on a hidden-overflow box, so nothing brought them back.
 *
 * Measured in the running app on 24/08/2026: deselecting one phone in the Tương tác panel
 * moved its header from y=123 to y=-286 and the operator was left looking at a blank white
 * rectangle for the rest of the session. The actor tiles are what make it so easy to hit —
 * they keep their checkbox off-screen on purpose and let the tile be the target, so choosing
 * a phone focuses a control that is by definition not visible.
 *
 * These are not screenshot tests: they assert the geometry, which is the thing that broke.
 */

async function openPanel(page: Page, button: string, card: string): Promise<void> {
  await installTauriMock(page);
  await page.goto("/");
  await expect(page.locator("[data-testid='device-tile']")).toHaveCount(2);
  await page.getByRole("button", { name: button }).click();
  await expect(page.locator(card)).toBeVisible();
}

/** The header's top edge, and the card's, in viewport coordinates. */
async function edges(page: Page, card: string, header: string) {
  return page.evaluate(
    ([cardSel, headerSel]) => {
      const cardEl = document.querySelector(cardSel) as HTMLElement;
      const headerEl = cardEl.querySelector(headerSel) as HTMLElement;
      return {
        card: Math.round(cardEl.getBoundingClientRect().top),
        header: Math.round(headerEl.getBoundingClientRect().top),
        scrollTop: Math.round(cardEl.scrollTop),
      };
    },
    [card, header],
  );
}

test("picking phones in the interaction panel never scrolls its header away", async ({ page }) => {
  await openPanel(page, "Tương tác", ".interaction-float");

  const before = await edges(page, ".interaction-float", ".interaction-title");
  expect(before.header).toBeGreaterThanOrEqual(before.card);

  // Every tile, so the run includes one far enough down the list that revealing its
  // off-screen checkbox would need a scroll.
  const tiles = page.locator(".interaction-float .tile-pick");
  const count = await tiles.count();
  expect(count).toBeGreaterThan(0);
  for (let i = 0; i < count; i += 1) {
    await tiles.nth(i).click({ force: true });
  }

  const after = await edges(page, ".interaction-float", ".interaction-title");
  expect(after.scrollTop, "the card itself must never scroll").toBe(0);
  expect(
    after.header,
    "the header has to stay inside the card — it left it, and the panel went blank",
  ).toBeGreaterThanOrEqual(after.card);
  // And the panel still says something, which is the symptom an operator reports.
  expect((await page.locator(".interaction-float").innerText()).trim().length).toBeGreaterThan(50);
});

test("the interaction body is the box that scrolls, and it reaches its end", async ({ page }) => {
  await openPanel(page, "Tương tác", ".interaction-float");

  const scroll = await page.evaluate(() => {
    const body = document.querySelector(".interaction-float-body") as HTMLElement;
    body.scrollTop = body.scrollHeight;
    const card = document.querySelector(".interaction-float") as HTMLElement;
    const last = body.lastElementChild as HTMLElement | null;
    return {
      bodyScrolled: Math.round(body.scrollTop),
      cardScrolled: Math.round(card.scrollTop),
      lastBottom: last ? Math.round(last.getBoundingClientRect().bottom) : null,
      cardBottom: Math.round(card.getBoundingClientRect().bottom),
    };
  });
  // Clipping without a scrollable body would trade a blank panel for an unreachable one.
  expect(scroll.bodyScrolled, "the body must scroll").toBeGreaterThan(0);
  expect(scroll.cardScrolled, "the card must not").toBe(0);
  expect(scroll.lastBottom!).toBeLessThanOrEqual(scroll.cardBottom + 2);
});

test("the nurture panel cannot be scrolled as a whole either", async ({ page }) => {
  await openPanel(page, "Nuôi TT", ".nurture-float");

  const result = await page.evaluate(() => {
    const card = document.querySelector(".nurture-float") as HTMLElement;
    const body = card.querySelector(".nurture-float-body") as HTMLElement;
    card.scrollTop = 500;
    const bodyStyle = getComputedStyle(body);
    return {
      cardScrollTop: Math.round(card.scrollTop),
      cardOverflow: getComputedStyle(card).overflowY,
      bodyOverflow: bodyStyle.overflowY,
    };
  });
  expect(result.cardScrollTop, "a float panel clips; it does not scroll").toBe(0);
  expect(result.cardOverflow, "`hidden` would leave it scrollable").toBe("clip");
  // Asserted as the rule rather than as "the body is scrolling right now": with the two
  // fixture phones this panel's content happens to fit, and a test that only passes when it
  // does not fit would say nothing on the day it fits.
  expect(result.bodyOverflow, "the body is the scroller").toBe("auto");
});
