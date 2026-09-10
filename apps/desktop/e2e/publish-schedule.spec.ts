import { expect, test, type Page, type Locator } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installTauriMock } from "./fixtures/tauriMock";

async function fixture(page: Page, count = 10, fleetSize = 10) {
  await installTauriMock(page, { androidRoster: true, fleetSize });
  await page.addInitScript(({ count }) => {
    const w = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> }; scheduleCalls: { command: string; args: Record<string, unknown> }[] };
    const original = w.__TAURI_INTERNALS__.invoke;
    w.scheduleCalls = [];
    w.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "plugin:dialog|open") return "C:/Bài đăng";
      if (command === "publish_scan_folder") return { sourceRoot: args.sourceRoot, scannedAt: new Date().toISOString(), notices: [], ignoredPartnerFiles: 0, ignoredHiddenFiles: 0, bundles: Array.from({ length: count }, (_, i) => ({ id: `b${i + 1}`, name: `Bài Đà Lạt ${i + 1}`, sourcePath: `C:/Bài đăng/${i}`, mediaKind: "image", images: [], captionPath: "caption.txt", caption: `Caption ${i}`, captionSha256: "a".repeat(64), totalBytes: 100 })) };
      if (command === "publish_schedule_preflight") {
        w.scheduleCalls.push({ command, args });
        const r = args.request as { slots: { udid: string }[] };
        return { inputDigest: "batch", canExecute: true, slots: r.slots.map(s => ({ inputDigest: "slot", canExecute: true, issues: [], assignments: [], targetSnapshot: { targetRef: { type: "explicit", udids: [s.udid] }, included: [], excluded: [], rosterSha256: "hash" } })) };
      }
      if (command === "publish_schedule_create") { w.scheduleCalls.push({ command, args }); return (args.request as { slots: unknown[] }).slots.map((_, i) => ({ id: `schedule-${i}` })); }
      if (command === "publish_execute" || command === "publish_create_campaign") { w.scheduleCalls.push({ command, args }); throw Error("Unexpected public action in schedule UI gate"); }
      return original(command, args);
    };
  }, { count });
  await page.goto("/");
  await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
  await expect(page.getByRole("button", { name: "Xem cùng thiết bị" })).toHaveCount(0);
  await page.getByRole("button", { name: "Chọn thư mục", exact: true }).click();
  await page.getByRole("button", { name: "Quét", exact: true }).click();
  await expect(page.getByRole("checkbox", { name: "Chọn Bài Đà Lạt 1", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Xem trước bài đăng", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Phóng to ảnh", exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "Hẹn giờ", exact: true }).click();
}
async function drag(page: Page, source: Locator, target: Locator, cancel = false) {
  await source.scrollIntoViewIfNeeded();
  const start = (await source.boundingBox())!;
  const end = (await target.boundingBox())!;
  await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2);
  await page.mouse.down();
  await page.mouse.move(start.x + start.width / 2 + 12, start.y + start.height / 2, { steps: 2 });
  await page.mouse.move(end.x + end.width / 2, end.y + end.height / 2, { steps: 6 });
  await expect(page.locator(".ps-drag-ghost")).toBeVisible();
  if (cancel) await page.keyboard.press("Escape");
  await page.mouse.up();
  await expect(page.locator(".ps-drag-ghost")).toHaveCount(0);
}
for (const viewport of [{ width: 1440, height: 900 }, { width: 900, height: 900 }, { width: 820, height: 560 }]) {
  test(`drag ten posts to ten machines and save one reviewed schedule ${viewport.width}`, async ({ page }) => {
    test.setTimeout(60000);
    await page.setViewportSize(viewport); await fixture(page);
    const errors: string[] = []; page.on("pageerror", error => errors.push(error.message));
    await page.getByRole("button", { name: "Chọn tất cả bài", exact: true }).click();
    await page.getByRole("button", { name: "Chọn máy sẵn sàng", exact: true }).click();
    await drag(page, page.getByRole("button", { name: "Kéo bài Bài Đà Lạt 1", exact: true }), page.locator(".ps-machines>header"));
    await expect(page.getByRole("status")).toHaveText("Đã gán 10 bài vào máy.");
    const values = await page.locator(".ps-plan-table select").evaluateAll(nodes => nodes.map(n => (n as HTMLSelectElement).value));
    expect(values).toHaveLength(10); expect(new Set(values).size).toBe(10); expect(values).not.toContain("");
    await page.locator(".ps-workspace").screenshot({ path: `../../target/schedule-drag-20260910/schedule-top-${viewport.width}.png` });
    await page.getByLabel("Ngày đăng", { exact: true }).fill("2099-09-10");
    await page.getByLabel("Giờ chung", { exact: true }).fill("20:00");
    await page.locator(".ps-plan-table tbody tr").first().scrollIntoViewIfNeeded();
    await expect(page.locator(".ps-plan-table tbody tr").first()).toBeVisible();
    await page.locator(".ps-workspace").screenshot({ path: `../../target/schedule-drag-20260910/schedule-allocation-${viewport.width}.png` });
    await page.getByRole("button", { name: "Kiểm tra lịch", exact: true }).click();
    await expect(page.getByRole("button", { name: "Lưu lịch 10 bài" })).toBeDisabled();
    await page.getByRole("checkbox", { name: /Tôi xác nhận/ }).check();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const footer = (await page.locator(".ps-footer").boundingBox())!;
    expect(footer.y + footer.height).toBeLessThanOrEqual(viewport.height);
    expect((await new AxeBuilder({ page }).include(".publish-daily-schedule").analyze()).violations).toEqual([]);
    await page.locator(".ps-workspace").screenshot({ path: `../../target/schedule-drag-20260910/schedule-review-${viewport.width}.png` });
    await page.getByRole("button", { name: "Lưu lịch 10 bài", exact: true }).click();
    await expect(page.getByRole("status")).toHaveText("Đã lưu lịch 10 bài. Xem từng lượt trong Theo dõi.");
    const calls = await page.evaluate(() => (window as unknown as { scheduleCalls: { command: string; args: { request: { slots: { bundleId: string; udid: string; runAt: string }[] } } }[] }).scheduleCalls);
    expect(calls.map(c => c.command)).toEqual(["publish_schedule_preflight", "publish_schedule_create"]);
    expect(calls[1].args.request.slots.map(s => s.bundleId)).toEqual(Array.from({ length: 10 }, (_, i) => `b${i + 1}`));
    expect(calls[1].args.request.slots.map(s => s.runAt)).toEqual(Array(10).fill("2099-09-10T20:00"));
    expect(errors).toEqual([]);
  });
}
test("successive direct drops, occupied machines, insufficient capacity, escape and undo", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 }); await fixture(page, 3, 2);
  const post = (n: number) => page.getByRole("button", { name: `Kéo bài Bài Đà Lạt ${n}`, exact: true });
  const machine = page.locator("[data-schedule-device]");
  await drag(page, post(1), machine.first(), true);
  await expect(page.locator("[data-schedule-row]")).toHaveCount(0);
  await drag(page, post(1), machine.first());
  const first = await page.getByLabel("Máy nhận Bài Đà Lạt 1", { exact: true }).inputValue();
  await drag(page, post(2), machine.first());
  await expect(page.getByLabel("Máy nhận Bài Đà Lạt 2", { exact: true })).toHaveValue("");
  await expect(page.getByLabel("Máy nhận Bài Đà Lạt 1", { exact: true })).toHaveValue(first);
  // A selected group is intentionally kept together; target the shared region to fill only its missing rows.
  await page.getByRole("button", { name: "Chọn máy sẵn sàng" }).click();
  await drag(page, post(2), page.locator(".ps-machines>header"));
  await drag(page, post(3), page.locator(".ps-machines>header"));
  await expect(page.getByLabel("Máy nhận Bài Đà Lạt 3", { exact: true })).toHaveValue("");
  expect(new Set(await page.locator(".ps-plan-table select").evaluateAll(nodes => nodes.map(n => (n as HTMLSelectElement).value).filter(Boolean))).size).toBe(2);
  await page.getByRole("button", { name: "Hoàn tác" }).click();
  await expect(page.locator("[data-schedule-row]")).toHaveCount(2);
  await page.getByLabel("Gỡ gán Bài Đà Lạt 1").click();
  await page.getByRole("button", { name: "Hoàn tác" }).click();
  await expect(page.getByLabel("Máy nhận Bài Đà Lạt 1", { exact: true })).toHaveValue(first);
  expect(await page.evaluate(() => (window as unknown as { scheduleCalls: unknown[] }).scheduleCalls)).toEqual([]);
});
test("drag preview follows auto-scroll and blur cancels without changing the draft", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 }); await fixture(page, 10, 20);
  await page.getByRole("button", { name: "Chọn tất cả bài", exact: true }).click();
  await page.getByRole("button", { name: "Chọn máy sẵn sàng", exact: true }).click();
  const start = (await page.locator("[data-schedule-drag]").first().boundingBox())!;
  const list = page.locator(".ps-machine-list");
  const end = (await list.boundingBox())!;
  await page.mouse.move(start.x + 30, start.y + 20); await page.mouse.down();
  await page.mouse.move(end.x + 60, end.y + end.height - 8, { steps: 8 });
  await expect(page.locator(".ps-drag-ghost")).toContainText("1 bài");
  await expect(page.locator(".ps-machine.is-drop-target")).toHaveCount(1);
  await expect.poll(() => list.evaluate(node => node.scrollTop)).toBeGreaterThan(0);
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await page.mouse.up();
  await expect(page.locator(".ps-drag-ghost")).toHaveCount(0);
  expect(await page.locator(".ps-plan-table select").evaluateAll(nodes => nodes.every(n => !(n as HTMLSelectElement).value))).toBe(true);
  expect(await page.evaluate(() => (window as unknown as { scheduleCalls: unknown[] }).scheduleCalls)).toEqual([]);
});

for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
  test(`quick selection is visible, assigns ten and guides keyboard focus ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport); await fixture(page, 10, 12);
    const quick = page.getByRole("button", { name: "Chọn nhanh", exact: true });
    await expect(quick).toBeVisible();
    const bounds = (await quick.boundingBox())!;
    expect(bounds.height).toBeGreaterThanOrEqual(36);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(viewport.height);
    await quick.focus(); await page.keyboard.press("Enter");
    await expect(page.getByRole("status")).toHaveText("Đã gán 10/10 bài · mỗi bài đã có máy");
    const rows = page.locator(".ps-plan-table select");
    const before = await rows.evaluateAll(nodes => nodes.map(n => (n as HTMLSelectElement).value));
    expect(new Set(before).size).toBe(10);
    await quick.click(); expect(await rows.evaluateAll(nodes => nodes.map(n => (n as HTMLSelectElement).value))).toEqual(before);
    await page.getByRole("button", { name: "Đặt giờ", exact: true }).click();
    await expect(page.getByLabel("Giờ chung", { exact: true })).toBeFocused();
    await page.locator(".ps-workspace").screenshot({ path: `../../target/schedule-quick-20260910/quick-${viewport.width}.png` });
    await page.getByLabel("Ngày đăng", { exact: true }).fill("2099-09-10");
    await page.getByLabel("Giờ chung", { exact: true }).fill("20:00");
    await page.getByRole("button", { name: "Kiểm tra lịch", exact: true }).click();
    await page.getByRole("button", { name: "Tới xác nhận", exact: true }).click();
    await expect(page.getByRole("checkbox", { name: /Tôi xác nhận/ })).toBeFocused();
    const footer = (await page.locator(".ps-footer").boundingBox())!;
    expect(footer.y + footer.height).toBeLessThanOrEqual(viewport.height);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    expect((await new AxeBuilder({ page }).include(".publish-daily-schedule").analyze()).violations).toEqual([]);
    const calls = await page.evaluate(() => (window as unknown as { scheduleCalls: { command: string }[] }).scheduleCalls);
    expect(calls.map(c => c.command)).toEqual(["publish_schedule_preflight"]);
  });
}

test("after quick selection a direct drag moves only its origin and reveals matching devices", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 }); await fixture(page, 10, 12);
  await page.getByRole("button", { name: "Chọn nhanh", exact: true }).click();
  const before = await page.locator(".ps-plan-table select").evaluateAll(nodes => nodes.map(n => (n as HTMLSelectElement).value));
  await page.getByLabel("Tìm máy hẹn giờ").fill("Máy 11");
  const free = page.locator("[data-schedule-device]").first();
  const target = await free.getAttribute("data-schedule-device");
  await drag(page, page.getByRole("button", { name: "Kéo bài Bài Đà Lạt 1", exact: true }), free);
  const after = await page.locator(".ps-plan-table select").evaluateAll(nodes => nodes.map(n => (n as HTMLSelectElement).value));
  expect(after[0]).toBe(target); expect(after.slice(1)).toEqual(before.slice(1));
  await page.getByRole("button", { name: "Kéo bài Bài Đà Lạt 1", exact: true }).focus();
  await expect(free).toHaveClass(/is-linked/);
  await expect(page.locator('[data-schedule-row="b1"]')).toHaveClass(/is-linked/);
  await page.getByLabel("Tìm máy hẹn giờ").fill("Máy 2");
  await drag(page, page.getByRole("button", { name: "Kéo bài Bài Đà Lạt 1", exact: true }), page.locator("[data-schedule-device]").first());
  expect(await page.getByLabel("Máy nhận Bài Đà Lạt 1", { exact: true }).inputValue()).toBe(target);
  await page.getByLabel("Tìm máy hẹn giờ").fill("no machine");
  await expect(page.getByText("Không có máy khớp từ khóa.")).toBeVisible();
  await page.getByRole("button", { name: "Xóa tìm kiếm", exact: true }).click();
  await expect(page.getByLabel("Tìm máy hẹn giờ")).toHaveValue("");
  await page.getByRole("button", { name: "Hoàn tác", exact: true }).click();
  expect(await page.locator(".ps-plan-table select").evaluateAll(nodes => nodes.map(n => (n as HTMLSelectElement).value))).toEqual(before);
  expect(await page.evaluate(() => (window as unknown as { scheduleCalls: unknown[] }).scheduleCalls)).toEqual([]);
});
test("footer takes the operator to the Sheet field in Setup", async ({ page }) => {
  await page.setViewportSize({ width: 820, height: 560 }); await fixture(page, 3, 3);
  await page.getByRole("tab", { name: "Thiết lập", exact: true }).click();
  await page.getByRole("checkbox", { name: "Ghi kết quả lên Sheet", exact: true }).check();
  await page.getByRole("textbox", { name: "Link Google Sheet", exact: true }).fill("https://docs.google.com/spreadsheets/d/fixture/edit");
  await page.getByRole("tab", { name: "Hẹn giờ", exact: true }).click();
  await page.getByRole("button", { name: "Chọn nhanh", exact: true }).click();
  await page.getByLabel("Ngày đăng", { exact: true }).fill("2099-09-10");
  await page.getByLabel("Giờ chung", { exact: true }).fill("20:00");
  await page.getByRole("button", { name: "Kiểm tra Sheet", exact: true }).click();
  await expect(page.getByRole("tab", { name: "Thiết lập", exact: true })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("textbox", { name: "Link Google Sheet", exact: true })).toBeFocused();
  expect(await page.evaluate(() => (window as unknown as { scheduleCalls: unknown[] }).scheduleCalls)).toEqual([]);
});
