import { expect, test, type Browser, type Page, type TestInfo } from "@playwright/test";

import { installTauriMock } from "./fixtures/tauriMock";

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

const DISPLAY_PROFILES = [
  {
    label: "125",
    deviceScaleFactor: 1.25,
    viewport: { width: 1_152, height: 720 },
    physical: { width: 1_440, height: 900 },
  },
  {
    label: "150",
    deviceScaleFactor: 1.5,
    viewport: { width: 960, height: 600 },
    physical: { width: 1_440, height: 900 },
  },
] as const;

function slug(value: string): string {
  return value
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[đĐ]/g, "d")
    .replace(/\s+/g, "-")
    .toLowerCase();
}

function pngDimensions(png: Buffer): { width: number; height: number } {
  const signature = png.subarray(0, 8).toString("hex");
  expect(signature).toBe("89504e470d0a1a0a");
  return { width: png.readUInt32BE(16), height: png.readUInt32BE(20) };
}

async function openProductionPage(page: Page, name: (typeof PAGES)[number]): Promise<void> {
  await installTauriMock(page, {
    androidRoster: name === "Chẩn đoán" || name === "Trung tâm ứng dụng",
  });
  await page.goto("/");
  await expect(page.locator("[data-testid='device-tile']")).toHaveCount(2);
  if (name !== "Thiết bị") {
    // At 150% the sidebar collapses and its visual label is clipped, while the button's
    // accessible name remains available to keyboard and screen-reader operators.
    await page.getByRole("button", { name, exact: true }).click();
  }
  await expect(page.locator(".loading-state")).toHaveCount(0);
  await page.evaluate(() => document.fonts.ready);
  await expect(page.getByRole("heading", { level: 1 })).toHaveCount(1);
  await expect(page.locator(".activity-center-current.is-error")).toHaveCount(0);
  await expect(page.getByText(/Unknown mock command/i)).toHaveCount(0);
}

async function assertNoPageWideOverflow(page: Page): Promise<void> {
  const widths = await page.evaluate(() => {
    const html = document.documentElement;
    const body = document.body;
    const main = document.querySelector<HTMLElement>(".main-col");
    const content = document.querySelector<HTMLElement>(".content");
    return {
      innerWidth,
      htmlClient: html.clientWidth,
      htmlScroll: html.scrollWidth,
      bodyClient: body.clientWidth,
      bodyScroll: body.scrollWidth,
      mainClient: main?.clientWidth ?? 0,
      mainScroll: main?.scrollWidth ?? 0,
      contentClient: content?.clientWidth ?? 0,
      contentScroll: content?.scrollWidth ?? 0,
    };
  });

  expect(widths.htmlScroll, JSON.stringify(widths)).toBeLessThanOrEqual(widths.htmlClient);
  expect(widths.bodyScroll, JSON.stringify(widths)).toBeLessThanOrEqual(widths.bodyClient);
  expect(widths.mainScroll, JSON.stringify(widths)).toBeLessThanOrEqual(widths.mainClient);
  expect(widths.contentScroll, JSON.stringify(widths)).toBeLessThanOrEqual(widths.contentClient);
  expect(widths.htmlClient).toBe(widths.innerWidth);
}

async function assertPaintedPixels(page: Page, png: Buffer): Promise<void> {
  const sample = await page.evaluate(async (base64) => {
    const bytes = Uint8Array.from(atob(base64), (character) => character.charCodeAt(0));
    const bitmap = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
    const canvas = document.createElement("canvas");
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (!context) throw new Error("2D canvas context is unavailable");
    context.drawImage(bitmap, 0, 0);
    bitmap.close();

    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
    const stride = Math.max(1, Math.floor(Math.min(canvas.width, canvas.height) / 80));
    const colours = new Set<number>();
    let dark = 0;
    let light = 0;
    let sampled = 0;
    let minimumLuminance = 255;
    let maximumLuminance = 0;
    for (let y = 0; y < canvas.height; y += stride) {
      for (let x = 0; x < canvas.width; x += stride) {
        const offset = (y * canvas.width + x) * 4;
        if (pixels[offset + 3] === 0) continue;
        const red = pixels[offset];
        const green = pixels[offset + 1];
        const blue = pixels[offset + 2];
        const luminance = Math.round(0.2126 * red + 0.7152 * green + 0.0722 * blue);
        colours.add((red >> 4) << 8 | (green >> 4) << 4 | (blue >> 4));
        minimumLuminance = Math.min(minimumLuminance, luminance);
        maximumLuminance = Math.max(maximumLuminance, luminance);
        if (luminance < 80) dark += 1;
        if (luminance > 180) light += 1;
        sampled += 1;
      }
    }
    return {
      colours: colours.size,
      contrast: maximumLuminance - minimumLuminance,
      darkRatio: dark / sampled,
      lightRatio: light / sampled,
      sampled,
    };
  }, png.toString("base64"));

  expect(sample.sampled, JSON.stringify(sample)).toBeGreaterThan(1_000);
  expect(sample.colours, JSON.stringify(sample)).toBeGreaterThan(16);
  expect(sample.contrast, JSON.stringify(sample)).toBeGreaterThan(120);
  // White navigation leaves only text/icon ink, not a large dark sidebar.
  expect(sample.darkRatio, JSON.stringify(sample)).toBeGreaterThan(0.001);
  expect(sample.lightRatio, JSON.stringify(sample)).toBeGreaterThan(0.2);
}

async function checkScaledPage(
  browser: Browser,
  name: (typeof PAGES)[number],
  profile: (typeof DISPLAY_PROFILES)[number],
  testInfo: TestInfo,
): Promise<void> {
  const context = await browser.newContext({
    viewport: profile.viewport,
    deviceScaleFactor: profile.deviceScaleFactor,
    colorScheme: "light",
    reducedMotion: "reduce",
    locale: "vi-VN",
  });
  const page = await context.newPage();
  try {
    await openProductionPage(page, name);
    await assertNoPageWideOverflow(page);
    const png = await page.screenshot({ animations: "disabled", scale: "device" });
    expect(pngDimensions(png)).toEqual(profile.physical);
    await assertPaintedPixels(page, png);
    await testInfo.attach(`dpi-${profile.label}-${slug(name)}.png`, {
      body: png,
      contentType: "image/png",
    });
  } finally {
    await context.close();
  }
}

test.describe("Windows display scaling", () => {
  test("the pixel gate rejects a blank light surface", async ({ page }) => {
    await page.setContent('<html><body style="margin:0;background:white"></body></html>');
    const blank = await page.screenshot();
    await expect(assertPaintedPixels(page, blank)).rejects.toThrow();
  });

  for (const profile of DISPLAY_PROFILES) {
    for (const name of PAGES) {
      test(`${name} paints at ${profile.label}% without page-wide overflow`, async ({ browser }, testInfo) => {
        await checkScaledPage(browser, name, profile, testInfo);
      });
    }
  }
});
