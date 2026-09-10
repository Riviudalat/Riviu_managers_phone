import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";
import AxeBuilder from "@axe-core/playwright";

for (const viewport of [{ width: 1440, height: 900 }, { width: 900, height: 700 }, { width: 820, height: 560 }]) {
  test(`interaction three-step workspace keeps actions and confirmation scoped at ${viewport.width}`, async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.setViewportSize(viewport);
    await installTauriMock(page, { androidRoster: true, fleetSize: 20 });
    await page.addInitScript(() => {
      const w = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> }; __IW_EFFECTS__: string[] };
      const invoke = w.__TAURI_INTERNALS__.invoke;
      w.__IW_EFFECTS__ = [];
      w.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (command === "startup_error") return null;
        if (command === "android_tool_problems" || command === "interaction_list") return [];
        if (command === "interaction_parse_links") return String(args.rawText).split("\n").filter(Boolean).map((url, index) => {
          const match = url.match(/@([^/]+)\/(video|photo)\/(\d+)/);
          return { lineNo: index + 1, original: url, error: match ? null : "invalidUrl", target: match ? { originalUrl: url, normalizedUrl: url, author: match[1], kind: match[2], contentId: match[3], targetKey: `content:${match[3]}` } : null };
        });
        if (command === "interaction_preview_thread") {
          const request = args.request as { requestId: string; actorUdids: string[]; targets: { targetKey: string }[] };
          return { lines: [], validTargetCount: request.targets.length, cohortCount: 1, streamCapacity: 8, plan: { requestId: request.requestId, assignments: request.targets.flatMap((target) => request.actorUdids.map((actorUdid, ordinal) => ({ targetKey: target.targetKey, actorUdid, ordinal, parentOrdinal: null, cohort: 0 }))) } };
        }
        if (["interaction_start_thread", "interaction_retry", "interaction_measure_post", "interaction_read_account"].includes(command)) {
          w.__IW_EFFECTS__.push(command);
          throw new Error("Effect blocked by UI fixture");
        }
        return invoke(command, args);
      };
    });
    await page.goto("/");
    await expect(page.getByTestId("device-tile")).toHaveCount(20);
    await page.getByRole("button", { name: "Tương tác", exact: true }).click();
    const workspace = page.getByRole("region", { name: "Không gian Tương tác" });
    await workspace.getByLabel("Link TikTok — mỗi dòng một link").fill("https://www.tiktok.com/@studio.trips/video/7512030405060708011\nhttps://www.tiktok.com/@coffee.corner/photo/7512030405060708022");
    await expect(workspace.getByText("Đúng định dạng", { exact: true })).toHaveCount(2);
    await page.screenshot({ path: test.info().outputPath(`interaction-links-${viewport.width}.png`) });
    await workspace.getByRole("button", { name: "Chọn hành động & máy →" }).click();
    await workspace.getByRole("combobox", { name: "Phạm vi thiết bị" }).selectOption("all");
    await workspace.getByRole("button", { name: "Tuỳ chỉnh nâng cao" }).focus();
    await page.keyboard.press("Enter");
    await expect(workspace.getByLabel("Số từ tối đa mỗi câu")).toBeVisible();
    await workspace.getByRole("button", { name: "Ẩn tuỳ chỉnh nâng cao" }).click();
    await workspace.getByRole("checkbox", { name: "Tim", exact: true }).check();
    await workspace.getByRole("checkbox", { name: "Lưu", exact: true }).check();
    await workspace.getByRole("checkbox", { name: "Bình luận", exact: true }).uncheck();
    await expect(workspace.getByLabel("Nội dung bình luận", { exact: true })).toHaveCount(0);
    const machines = workspace.getByRole("group", { name: "Danh sách máy thực hiện" });
    await expect(machines.getByRole("checkbox")).toHaveCount(20);
    expect(await machines.evaluate(element => getComputedStyle(element).gridTemplateColumns.split(" ").length)).toBe(2);
    await expect(workspace.getByRole("button", { name: "Trang máy sau" })).toHaveCount(0);
    await workspace.getByRole("button", { name: "Chọn tất cả", exact: true }).click();
    await expect(machines.getByRole("checkbox", { checked: true })).toHaveCount(20);
    await workspace.getByRole("tab", { name: "Hẹn giờ", exact: true }).click();
    await expect(workspace.getByRole("tabpanel", { name: "Hẹn giờ", exact: true })).toBeVisible();
    await workspace.getByRole("tab", { name: "Thiết lập", exact: true }).click();
    await expect(machines.getByRole("checkbox", { checked: true })).toHaveCount(20);
    await workspace.getByRole("button", { name: "Bỏ chọn", exact: true }).click();
    await workspace.getByRole("checkbox", { name: "Máy thử 9", exact: true }).check();
    await workspace.getByRole("textbox", { name: "Tìm máy hoặc tài khoản" }).fill("20");
    await expect(machines.getByRole("checkbox")).toHaveCount(1);
    await machines.getByRole("checkbox").check();
    await workspace.getByRole("textbox", { name: "Tìm máy hoặc tài khoản" }).fill("");
    await workspace.getByRole("checkbox", { name: "Máy thử 20", exact: true }).uncheck();
    await expect(workspace.getByRole("button", { name: "Kiểm tra lượt chạy →" })).toBeEnabled();
    await page.screenshot({ path: test.info().outputPath(`interaction-actions-${viewport.width}.png`) });
    const accessibility = await new AxeBuilder({ page }).include(".interaction-workspace").withTags(["wcag2a", "wcag2aa"]).analyze();
    expect(accessibility.violations).toEqual([]);
    await workspace.getByRole("button", { name: "Kiểm tra lượt chạy →" }).click();
    await expect(workspace.getByRole("tabpanel", { name: "Kiểm tra & chạy" })).toContainText("Máy thử 9");
    await page.screenshot({ path: test.info().outputPath(`interaction-review-${viewport.width}.png`) });
    expect(await page.evaluate(() => {
      const footer = document.querySelector(".iw-footer")!.getBoundingClientRect();
      const stage = document.querySelector(".iw-stage")!.getBoundingClientRect();
      return { horizontal: document.documentElement.scrollWidth <= innerWidth, footerVisible: footer.bottom <= innerHeight && footer.top >= 0, panelsFit: stage.bottom <= footer.top };
    })).toEqual({ horizontal: true, footerVisible: true, panelsFit: true });
    await workspace.getByRole("button", { name: "Bắt đầu tương tác" }).click();
    const confirm = page.getByRole("alertdialog", { name: "Xác nhận tương tác" });
    await expect(confirm).toContainText("2 bài sẽ được mở trên 1 máy");
    await expect(confirm).toContainText("Tim → Lưu");
    await page.keyboard.press("Escape");
    await expect(confirm).toHaveCount(0);
    expect(await page.evaluate(() => (window as unknown as { __IW_EFFECTS__: string[] }).__IW_EFFECTS__)).toEqual([]);
    expect(errors).toEqual([]);
  });
}
