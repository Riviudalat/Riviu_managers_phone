import { expect, test, type Locator, type Page } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";

type FixtureWindow = {
  __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> };
  __LAYOUT_FINISH_READ__?: (success: boolean) => void;
};

// Unlike locator.click(), this measurement never scrolls clipped controls into view.
async function fullyReachable(control: Locator): Promise<boolean> {
  return control.evaluate((element) => {
    const box = element.getBoundingClientRect();
    if (box.width <= 0 || box.height <= 0 || box.left < 0 || box.top < 0 || box.right > innerWidth || box.bottom > innerHeight) return false;
    for (let parent = element.parentElement; parent; parent = parent.parentElement) {
      const style = getComputedStyle(parent);
      const bounds = parent.getBoundingClientRect();
      if (/auto|scroll|hidden|clip/.test(style.overflowY) && (box.top < bounds.top - 1 || box.bottom > bounds.bottom + 1)) return false;
      if (/auto|scroll|hidden|clip/.test(style.overflowX) && (box.left < bounds.left - 1 || box.right > bounds.right + 1)) return false;
    }
    return element.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2));
  });
}

async function wheelToControl(page: Page, control: Locator, host: Locator): Promise<void> {
  for (let attempt = 0; attempt < 16 && !await fullyReachable(control); attempt++) {
    const point = await host.evaluate((element) => {
      const box = element.getBoundingClientRect();
      let top = Math.max(0, box.top), bottom = Math.min(innerHeight, box.bottom);
      for (let parent = element.parentElement; parent; parent = parent.parentElement) {
        if (!/auto|scroll|hidden|clip/.test(getComputedStyle(parent).overflowY)) continue;
        const bounds = parent.getBoundingClientRect();
        top = Math.max(top, bounds.top); bottom = Math.min(bottom, bounds.bottom);
      }
      return { x: box.left + 6, y: (top + bottom) / 2 };
    });
    await page.mouse.move(point.x, point.y);
    await page.mouse.wheel(0, 300);
    await page.waitForTimeout(60);
  }
  expect(await fullyReachable(control), "ordinary wheel scrolling exposes the complete control and its click target").toBe(true);
}

async function clickWithoutScrolling(page: Page, control: Locator): Promise<void> {
  expect(await fullyReachable(control), "control is fully visible before clicking").toBe(true);
  const box = await control.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.click(box!.x + box!.width / 2, box!.y + box!.height / 2);
}

async function installLayoutFixture(page: Page, interaction = false): Promise<void> {
  await installTauriMock(page, { androidRoster: true, fleetSize: interaction ? 30 : undefined });
  await page.addInitScript(() => {
    const w = window as unknown as FixtureWindow;
    const invoke = w.__TAURI_INTERNALS__.invoke;
    const handles: Record<string, string> = {};
    let saved: Record<string, unknown> | null = null;
    w.__TAURI_INTERNALS__.invoke = async (command, args = {}) => {
      if (command === "get_device_meta") return { udid: args.udid, handle: handles[String(args.udid)] ?? "", notes: "", tags: [], groupId: null, alias: "", number: null };
      if (command === "save_device_handle") {
        if (args.handle === "rejected.account") throw new Error("Nick đã thay đổi trên máy khác; tải lại nick đã lưu rồi thử lại.");
        handles[String(args.udid)] = String(args.handle);
        return args.handle;
      }
      if (command === "interaction_list" || command === "android_tool_problems") return [];
      if (command === "interaction_parse_links") {
        const url = String(args.rawText);
        return [{ lineNo: 1, original: url, target: { originalUrl: url, normalizedUrl: url, targetKey: "content:111", contentId: "111", author: "fixture", kind: "video" }, error: null }];
      }
      if (command === "interaction_preview_thread") return { lines: [], validTargetCount: 1, cohortCount: 1, streamCapacity: 8, plan: null };
      if (command === "interaction_read_account") return new Promise((resolve, reject) => {
        w.__LAYOUT_FINISH_READ__ = (success) => success
          ? resolve({ udid: args.udid, expectedHandle: handles[String(args.udid)] ?? "", observedHandle: "tai.khoan.kiem.thu.dai", status: "matched", checkedAt: "2026-09-11T08:00:00Z", snapshotSha256: "fixture-account-proof" })
          : reject(new Error("Chưa đọc được tài khoản trên máy. Kiểm tra màn hình Hồ sơ rồi đọc lại để đối chiếu."));
      });
      if (command === "orchestration_validate") return { document: args.document };
      if (command === "automation_list") return [{ id: "layout-profile", kind: "interaction", name: "Tương tác mẫu", latestRevision: 1, archived: false, createdAt: "2026-09-11T08:00:00Z", updatedAt: "2026-09-11T08:00:00Z" }];
      if (command === "orchestration_save_revision") {
        saved = { ...(args.document as Record<string, unknown>), revision: Number(args.expectedRevision ?? 0) + 1 };
        return { compiled: { document: saved, executionOrder: [], profiles: {}, canonicalJson: JSON.stringify(saved), sha256: "fixture-orchestration" }, createdAt: "2026-09-11T08:00:00Z" };
      }
      if (command === "orchestration_list" && saved) return [{ id: saved.id, name: saved.name, latestRevision: saved.revision, archived: false, updatedAt: "2026-09-11T08:00:00Z" }];
      return invoke(command, args);
    };
  });
  await page.goto("/");
  await expect(page.getByTestId("device-tile")).toHaveCount(interaction ? 30 : 2);
}

for (const viewport of [{ width: 820, height: 560 }, { width: 900, height: 900 }, { width: 1023, height: 900 }]) {
  test(`interaction account controls remain wheel-accessible with 30 machines at ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await installLayoutFixture(page, true);
    await page.getByRole("button", { name: "Tương tác", exact: true }).click();
    await page.getByPlaceholder("Dán link TikTok, mỗi dòng một bài").fill("https://www.tiktok.com/@fixture/video/111");
    await page.getByRole("button", { name: "Chọn hành động & máy →" }).click();
    await page.getByRole("combobox", { name: "Phạm vi thiết bị" }).selectOption("all");
    await page.getByRole("button", { name: "Tài khoản TikTok của Máy thử 1", exact: true }).click();
    const machines = page.getByRole("region", { name: "Máy thực hiện", exact: true });
    const editor = machines.locator(".iw-account-editor");
    const read = editor.getByRole("button", { name: "Đọc tài khoản từ máy", exact: true });
    await expect(editor.getByRole("textbox", { name: "Nick đã gán" })).toHaveAttribute("aria-invalid", "false");
    await expect(machines.locator(".machine-choice")).toHaveCount(30);
    await wheelToControl(page, read, machines);
    await page.screenshot({ path: test.info().outputPath(`interaction-account-ready-${viewport.width}.png`) });
    await clickWithoutScrolling(page, read);
    await expect(editor.getByRole("button", { name: "Đang đọc tài khoản…", exact: true })).toBeDisabled();
    await page.evaluate(() => (window as unknown as FixtureWindow).__LAYOUT_FINISH_READ__!(false));
    const readError = editor.getByRole("alert");
    await wheelToControl(page, readError, machines);
    await expect(readError).toContainText("Chưa đọc được tài khoản trên máy");
    await page.screenshot({ path: test.info().outputPath(`interaction-account-error-${viewport.width}.png`) });
    // An unsuccessful nick save adds another row. Reload must remain reachable too.
    const nick = editor.getByRole("textbox", { name: "Nick đã gán" });
    await nick.fill("rejected.account");
    await nick.press("Tab");
    const reload = editor.getByRole("button", { name: "Tải lại nick đã lưu", exact: true });
    await expect(reload).toBeAttached();
    await wheelToControl(page, reload, machines);
    await clickWithoutScrolling(page, reload);
    await expect(nick).toHaveValue("");
    await expect(nick).toHaveAttribute("aria-invalid", "false");
    await wheelToControl(page, read, machines);
    await clickWithoutScrolling(page, read);
    await expect(editor.getByRole("button", { name: "Đang đọc tài khoản…", exact: true })).toBeDisabled();
    await page.evaluate(() => (window as unknown as FixtureWindow).__LAYOUT_FINISH_READ__!(true));
    const result = editor.getByRole("status");
    await wheelToControl(page, result, machines);
    await expect(result).toHaveText("Khớp tài khoản · @tai.khoan.kiem.thu.dai");
    await page.screenshot({ path: test.info().outputPath(`interaction-account-result-${viewport.width}.png`) });
    expect(await machines.locator(".iw-machine-scroll").evaluate((element) => getComputedStyle(element).gridTemplateColumns.split(" ").length)).toBe(2);
    await expect(machines).toHaveAttribute("tabindex", "0");
    await machines.focus();
    await expect(machines).toBeFocused();
    await machines.press("Home");
    await expect.poll(() => machines.evaluate((element) => element.scrollTop)).toBe(0);
    await expect(page.locator(".iw-footer")).toBeInViewport();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`interaction-account-keyboard-${viewport.width}.png`) });
  });
}

async function checkToolbar(toolbar: Locator): Promise<void> {
  for (const button of await toolbar.getByRole("button").all()) {
    expect(await fullyReachable(button), `toolbar action ${await button.innerText()} is entirely inside the editor`).toBe(true);
    expect(await button.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    expect((await button.boundingBox())!.height).toBeGreaterThanOrEqual(36);
    expect(await button.evaluate((element) => [...element.childNodes].every((node) => {
      if (node.nodeType !== Node.TEXT_NODE || !node.textContent?.trim()) return true;
      const range = document.createRange();
      range.selectNodeContents(node);
      return range.getClientRects().length <= 1;
    })), "button labels stay on one line").toBe(true);
  }
}

for (const width of [1024, 1025, 1050, 1100, 1101, 1440]) {
  test(`orchestration new, saved and monitor toolbars fit at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await installLayoutFixture(page);
    await page.getByRole("button", { name: "Flow", exact: true }).click();
    await page.getByRole("tab", { name: "Điều phối", exact: true }).click();
    await page.getByRole("button", { name: "Tạo điều phối", exact: true }).click();
    await page.getByLabel("Tên điều phối", { exact: true }).fill("Điều phối nội dung chiến dịch tháng chín — nhóm máy và tài khoản dành riêng cho kiểm thử");
    await page.getByRole("combobox", { name: "Chọn hồ sơ Tương tác để thêm", exact: true }).selectOption("layout-profile");
    await page.getByRole("button", { name: "Thêm Tương tác", exact: true }).click();
    const toolbar = page.locator(".orchestration-toolbar");
    await expect(toolbar.getByRole("button", { name: "Lưu bản", exact: true })).toBeEnabled();
    await checkToolbar(toolbar);
    await page.screenshot({ path: test.info().outputPath(`orchestration-new-${width}.png`) });
    await clickWithoutScrolling(page, toolbar.getByRole("button", { name: "Lưu bản", exact: true }));
    await expect(page.getByText("Đã lưu bản 1", { exact: true })).toBeVisible();
    await expect(toolbar.getByRole("button", { name: "Chạy điều phối", exact: true })).toBeEnabled();
    await checkToolbar(toolbar);
    await page.screenshot({ path: test.info().outputPath(`orchestration-saved-${width}.png`) });
    await clickWithoutScrolling(page, toolbar.getByRole("button", { name: "Chạy điều phối", exact: true }));
    await page.getByRole("alertdialog").getByRole("button", { name: "Chạy", exact: true }).click();
    await expect(page.locator(".orchestration-monitor")).toBeVisible();
    await expect(toolbar.getByRole("button", { name: "Dừng điều phối", exact: true })).toBeEnabled();
    await checkToolbar(toolbar);
    await clickWithoutScrolling(page, toolbar.getByRole("button", { name: "Đối soát", exact: true }));
    await expect(toolbar.getByRole("button", { name: "Đối soát", exact: true })).toBeEnabled();
    expect(await page.locator(".orchestration-editor").evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`orchestration-monitor-${width}.png`) });
  });
}
