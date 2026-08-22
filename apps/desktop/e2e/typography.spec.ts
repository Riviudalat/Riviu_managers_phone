import { expect, test } from "@playwright/test";

/**
 * These run in a real browser because the thing being checked cannot be seen in one.
 *
 * `index.css` sets `font-synthesis: none`, which is right — faux bold on a dense UI
 * looks smeared — but it also removes the safety net. If the bundled face does not
 * actually carry a bold, every semibold label in the app renders at regular and
 * nothing errors, nothing warns, and no unit test can tell: jsdom has no font engine,
 * so `getComputedStyle` happily reports `font-weight: 700` on text drawn at 400.
 *
 * Measuring the glyphs is the only way to know. Same string, same size, same family,
 * two weights: a real variable font makes the bold one wider.
 */
test.describe("the bundled typeface", () => {
  test("renders bold as bold, with no synthesis to fall back on", async ({ page }) => {
    await page.goto("/");
    await page.evaluate(() => document.fonts.ready);

    const widths = await page.evaluate(async () => {
      const measure = async (weight: number) => {
        const el = document.createElement("span");
        el.textContent = "Nuôi tài khoản — Riviu Manager 0123456789";
        el.style.cssText = `position:fixed;left:-9999px;top:0;white-space:pre;
          font-family:var(--font-body);font-size:32px;font-weight:${weight};`;
        document.body.appendChild(el);
        await document.fonts.ready;
        const width = el.getBoundingClientRect().width;
        el.remove();
        return width;
      };
      return {
        regular: await measure(400),
        semibold: await measure(600),
        bold: await measure(700),
        synthesis: getComputedStyle(document.documentElement).fontSynthesis,
      };
    });

    expect(widths.regular).toBeGreaterThan(0);
    // Heavier weights are wider in Noto Sans. Equal widths mean the browser found no
    // bold face and, with synthesis off, drew regular under a bold declaration.
    expect(
      widths.bold,
      `bold measured ${widths.bold}px against regular ${widths.regular}px — the bold face is not loading`,
    ).toBeGreaterThan(widths.regular);
    expect(widths.semibold).toBeGreaterThan(widths.regular);
    expect(widths.bold).toBeGreaterThanOrEqual(widths.semibold);
  });

  test("draws Vietnamese from the bundled face rather than a fallback", async ({ page }) => {
    await page.goto("/");
    await page.evaluate(() => document.fonts.ready);

    // Every phone label, status and error in this app is Vietnamese, so a subset that
    // omits it is not a cosmetic loss — it is most of the interface falling back.
    //
    // `load` before `check`, and the reason is the mechanism being tested: the faces are
    // split by `unicode-range`, so the vietnamese file is not fetched until something
    // needs a codepoint from it. Asking `check` first reports false on a subset that is
    // present and simply not wanted yet — which is a passing arrangement failing a test.
    const carried = await page.evaluate(async () => {
      const text = "Nuôi tài khoản, khởi động, đã tắt";
      const loaded = await document.fonts.load('16px "Noto Sans"', text);
      return { faces: loaded.length, available: document.fonts.check('16px "Noto Sans"', text) };
    });
    expect(carried.faces, "no face claims these codepoints").toBeGreaterThan(0);
    expect(carried.available).toBe(true);
  });

  test("asks the network for nothing", async ({ page }) => {
    // The app runs on a farm machine that may have no route out. A stylesheet or font
    // fetched at startup is one the operator does not get, and the layout shifts under
    // whatever Windows substitutes.
    const external: string[] = [];
    page.on("request", (request) => {
      const url = request.url();
      if (!url.startsWith("http://127.0.0.1:1421") && !url.startsWith("data:")) {
        external.push(url);
      }
    });
    await page.goto("/");
    await page.evaluate(() => document.fonts.ready);
    expect(external, `the page reached outside itself: ${external.join(", ")}`).toEqual([]);
  });
});
