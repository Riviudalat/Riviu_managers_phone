import { expect, test } from "@playwright/test";
import type { InteractionAssignmentRecord, InteractionCampaignDetail, PublicActionResult } from "../src/types";
import { installTauriMock } from "./fixtures/tauriMock";

function makeCampaign(id: string, running: boolean): InteractionCampaignDetail {
  const assignments = Array.from({ length: 20 }, (_, index): InteractionAssignmentRecord => {
    const failed = !running && index >= 11;
    const makeAction = (kind: PublicActionResult["kind"]): PublicActionResult => ({
      kind,
      state: running ? "planned" : failed ? kind === "like" ? "failedBeforeEffect" : "planned" : kind === "like" && index === 10 ? "noOp" : "confirmed",
      effectIntent: running || failed || kind === "like" && index === 10 ? null : kind === "like" ? "tap" : "send",
      revision: 1,
      evidence: !running && !failed && kind === "like" && index === 10 ? '{"verdict":"alreadyLiked"}' : null,
      error: failed && kind === "like" ? index < 13 ? "target_open_no_baseline" : index < 19 ? "target_open_screen_unchanged" : "target_open_no_post_page" : null,
    });
    return {
      id: `${id}-assignment-${index + 1}`, targetKey: `content:${running ? "222" : "111"}`,
      ordinal: index, actorUdid: `MOCK-FLEET-${index + 1}`, parentAssignmentId: null,
      state: running ? "queued" : failed ? "failed" : "succeeded", preparedText: null,
      errorCode: failed ? "Like không an toàn để tiếp tục assignment" : null,
      actions: [makeAction("like"), makeAction("comment")],
    };
  });
  return {
    summary: {
      id, requestId: `${id}-request`, state: running ? "running" : "partial", messageCount: 20,
      targetCount: 1, succeededMessages: running ? 0 : 11, failedMessages: running ? 0 : 9,
      errorCode: running ? null : "xong 11, lỗi 9, còn dở 0",
      updatedAt: running ? "2026-09-07T07:45:00Z" : "2026-09-07T06:30:00Z",
      brief: {
        firstAuthor: running ? "dang.chay" : "da.ket.thuc", firstContentId: running ? "222" : "111",
        mode: "standalone", shape: "star", cohortSize: null, actorCount: 20, manual: true,
        likeTarget: true, actions: { like: true, save: false, comment: true },
      },
      actionCounters: { planned: 40, attempted: running ? 0 : 21, confirmed: running ? 0 : 21, noOp: running ? 0 : 1, uncertain: 0 },
    },
    assignments,
    actionAggregate: running ? null : "partial",
  };
}

for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
  test(`interaction monitor keeps selection and historical outcomes truthful at ${viewport.width}x${viewport.height}`, async ({ page }) => {
    const runtimeErrors: string[] = [];
    page.on("pageerror", (error) => runtimeErrors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") runtimeErrors.push(message.text());
    });
    await page.setViewportSize(viewport);
    await installTauriMock(page, { androidRoster: true, fleetSize: 20 });
    const completed = makeCampaign("partial-fixture", false);
    const running = makeCampaign("running-fixture", true);
    await page.addInitScript(({ completed, running }) => {
      const w = window as unknown as {
        __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> };
        __INTERACTION_MONITOR_TEST__: { unknown: string[]; effects: string[]; releaseRunning: (() => void) | null };
      };
      const invoke = w.__TAURI_INTERNALS__.invoke;
      w.__INTERACTION_MONITOR_TEST__ = { unknown: [], effects: [], releaseRunning: null };
      let held = true;
      w.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (command === "startup_error") return null;
        if (command === "android_tool_problems") return [];
        if (command === "interaction_list") return [running.summary, completed.summary];
        if (command === "interaction_list_artifacts" || command === "interaction_list_target_notes") return [];
        if (command === "interaction_get") {
          if (args.campaignId === completed.summary.id) return structuredClone(completed);
          if (args.campaignId === running.summary.id) {
            if (held) await new Promise<void>((resolve) => {
              w.__INTERACTION_MONITOR_TEST__.releaseRunning = () => { held = false; resolve(); };
            });
            return structuredClone(running);
          }
          return null;
        }
        if (["interaction_start_thread", "interaction_retry", "interaction_cancel", "public_cleanup_execute"].includes(command)) {
          w.__INTERACTION_MONITOR_TEST__.effects.push(command);
          throw new Error("monitor fixture must remain read-only");
        }
        try { return await invoke(command, args); }
        catch (error) {
          if (String(error).includes("Unknown mock command")) w.__INTERACTION_MONITOR_TEST__.unknown.push(command);
          throw error;
        }
      };
    }, { completed, running });
    await page.goto("/");
    await expect(page.getByTestId("device-tile")).toHaveCount(20);
    await page.getByRole("button", { name: "Tương tác", exact: true }).click();
    await page.getByRole("tab", { name: "Theo dõi", exact: true }).click();
    const list = page.getByRole("region", { name: "Danh sách chiến dịch" });
    const detail = page.getByRole("complementary", { name: "Chi tiết chiến dịch" });
    const completedRow = list.getByRole("button", { name: /@da.ket.thuc/ });
    const runningRow = list.getByRole("button", { name: /@dang.chay/ });

    await completedRow.click();
    await expect(completedRow).toHaveAttribute("aria-current", "true");
    const campaignBounds = await completedRow.evaluate((button) => {
      const row = button.closest(".interaction-campaign-row")!.getBoundingClientRect();
      return [...button.querySelectorAll(".interaction-campaign-head, small")].every((element) => element.getBoundingClientRect().right <= row.right);
    });
    expect(campaignBounds).toBe(true);
    await expect(detail.getByText(/40\/40 hành động đã có kết quả/)).toBeVisible();
    await expect(detail.locator(".interaction-assignment")).toHaveCount(20);
    await expect(detail.getByLabel("18 chưa thực hiện", { exact: true })).toBeVisible();
    await expect(detail.getByLabel("21 xác nhận", { exact: true })).toBeVisible();
    await expect(detail.getByText("Bình luận · Chưa thực hiện: lượt đã dừng", { exact: true })).toHaveCount(9);
    await expect(detail.getByText("Bình luận · Đang chờ", { exact: true })).toHaveCount(0);
    await expect(detail.getByRole("progressbar", { name: "Tiến trình chiến dịch đang xem" })).toHaveAttribute("aria-valuenow", "100");
    const generic = detail.getByText("Like không an toàn để tiếp tục assignment", { exact: true });
    await expect(generic).toHaveCount(9);
    await expect(generic.first()).not.toBeVisible();
    await detail.getByText(/40\/40 hành động đã có kết quả/).scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: test.info().outputPath(`interaction-monitor-overview-${viewport.width}.png`) });
    await detail.getByText("Bình luận · Chưa thực hiện: lượt đã dừng", { exact: true }).first().scrollIntoViewIfNeeded();
    await page.screenshot({ path: test.info().outputPath(`interaction-monitor-stopped-${viewport.width}.png`) });

    await runningRow.click();
    await expect(runningRow).toHaveAttribute("aria-current", "true");
    await expect(completedRow).not.toHaveAttribute("aria-current", "true");
    await expect(detail.getByText("Đang mở chiến dịch…", { exact: true })).toBeVisible();
    await expect(detail.getByRole("button", { name: "Thử lại phần hỏng", exact: true })).toHaveCount(0);
    await expect(detail.getByText(/40\/40 hành động đã có kết quả/)).toHaveCount(0);
    await page.evaluate(() => {
      (window as unknown as { __INTERACTION_MONITOR_TEST__: { releaseRunning: () => void } }).__INTERACTION_MONITOR_TEST__.releaseRunning();
    });
    await expect(detail.getByText(/0\/40 hành động đã có kết quả/)).toBeVisible();
    await expect(detail.getByRole("button", { name: "Dừng", exact: true })).toBeVisible();
    await expect(detail.getByText("Bình luận · Đang chờ", { exact: true })).toHaveCount(20);
    await expect(detail.getByText("Bình luận · Chưa thực hiện: lượt đã dừng", { exact: true })).toHaveCount(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await expect(page.getByRole("alert")).toHaveCount(0);
    await expect(page.getByText(/Unknown mock command/)).toHaveCount(0);
    expect(runtimeErrors).toEqual([]);
    expect(await page.evaluate(() => {
      const fixture = (window as unknown as { __INTERACTION_MONITOR_TEST__: { unknown: string[]; effects: string[] } }).__INTERACTION_MONITOR_TEST__;
      return { unknown: fixture.unknown, effects: fixture.effects };
    })).toEqual({ unknown: [], effects: [] });
  });
}
