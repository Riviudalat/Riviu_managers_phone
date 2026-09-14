import { openOperatorPage } from "./fixtures/operatorNavigation";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { expect, test } from "@playwright/test";
import type { TemplateMatchRequest } from "../src/api";
import { installTauriMock, mockCommandCalls } from "./fixtures/tauriMock";

for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 700 }]) {
  test(`crop template diagnostic reviews results before use at ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await installTauriMock(page);
    await page.addInitScript(() => {
      type RecordValue = Record<string, unknown>;
      const host = window as unknown as {
        __TAURI_INTERNALS__: { invoke: (command: string, args: RecordValue) => Promise<unknown> };
        __TEMPLATE_REQUESTS__: TemplateMatchRequest[];
      };
      host.__TEMPLATE_REQUESTS__ = [];
      const original = host.__TAURI_INTERNALS__.invoke;
      host.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (command === "flow_get") {
          const revision = await original(command, args) as { document: { nodes: RecordValue[] } };
          revision.document.nodes = revision.document.nodes.map((node) => {
            if (node.kind === "wait") return { ...node, kind: "launchApp", config: { bundleId: "com.fixture.app" } };
            if (node.kind === "tap") return { ...node, kind: "tapVision", config: { threshold: 0.9 }, postcondition: null };
            return node;
          });
          return revision;
        }
        if (command === "flow_coordinate_frame") {
          const canvas = document.createElement("canvas");
          canvas.width = 400; canvas.height = 800;
          const context = canvas.getContext("2d")!;
          context.fillStyle = "#f4f1ed"; context.fillRect(0, 0, 400, 800);
          context.fillStyle = "#222c39"; context.fillRect(0, 0, 400, 90);
          context.font = "bold 26px sans-serif"; context.fillStyle = "white"; context.fillText("Riviu - Sample", 24, 58);
          context.fillStyle = "#466177"; context.fillRect(40, 180, 320, 180);
          context.fillStyle = "#e6af64"; context.fillRect(75, 210, 90, 85);
          context.fillStyle = "white"; context.font = "24px sans-serif"; context.fillText("Profile", 190, 260);
          context.fillStyle = "#253a50"; context.font = "21px sans-serif"; context.fillText("Local image diagnostic", 40, 430);
          return { jpegBase64: canvas.toDataURL("image/jpeg", 0.95).split(",")[1], imageWidth: 400, imageHeight: 800, orientation: "portrait", profileId: "fixture-profile" };
        }
        if (command === "gui_template_match") {
          const request = args.request as TemplateMatchRequest;
          host.__TEMPLATE_REQUESTS__.push(request);
          return {
            protocolVersion: request.protocolVersion, requestId: request.requestId, observationId: request.observationId,
            sessionEpoch: request.sessionEpoch, generation: request.generation,
            screenshotSha256: request.screenshot.sha256, templateSha256: request.template.sha256,
            status: "resolved", reason: "template_unique_match", elapsedMs: 123, searchedScales: request.scales,
            candidates: [{ bounds: { x: 40, y: 180, width: request.template.width, height: request.template.height }, score: 0.9912, scale: 1, method: "templateCorrelation" }],
          };
        }
        return original(command, args);
      };
    });
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/");
    await page.getByTestId("device-tile").first().click({ modifiers: ["ControlOrMeta"] });
    await expect(page.locator("[data-testid='device-tile'].selected")).toHaveCount(1);
    await openOperatorPage(page, 'Flow');
    await expect(page.getByLabel("Tên Flow")).toHaveValue("Cuộn nội dung");
    await page.getByTestId("flow-node-title").filter({ hasText: "Chạm theo ảnh" }).click();
    await page.getByRole("button", { name: "Chụp mẫu từ thiết bị" }).click();
    const screenshot = page.getByRole("img", { name: "Khung hình thiết bị" });
    await expect(screenshot).toBeVisible();
    const rect = await screenshot.boundingBox();
    if (!rect) throw new Error("capture image missing");
    const scale = Math.min(rect.width / 400, rect.height / 800);
    const left = (rect.width - 400 * scale) / 2;
    const top = (rect.height - 800 * scale) / 2;
    await screenshot.click({ position: { x: left + 40 * scale, y: top + 180 * scale } });
    await screenshot.click({ position: { x: left + 360 * scale, y: top + 360 * scale } });
    await expect(page.getByRole("button", { name: "Kiểm tra ảnh mẫu" })).toBeVisible();
    expect(await page.evaluate(() => (window as unknown as { __TEMPLATE_REQUESTS__: unknown[] }).__TEMPLATE_REQUESTS__)).toHaveLength(0);
    await page.getByLabel("Thử nhiều tỷ lệ (75%, 100%, 125%, 150%)").check();
    await page.getByRole("button", { name: "Kiểm tra ảnh mẫu" }).click();
    await expect(page.getByText("Tìm thấy một vị trí")).toBeVisible();
    await expect(page.locator(".flow-template-test")).toContainText("0.9912");
    const requests = await page.evaluate(() => (window as unknown as { __TEMPLATE_REQUESTS__: TemplateMatchRequest[] }).__TEMPLATE_REQUESTS__);
    expect(requests).toHaveLength(1);
    expect(requests[0].screenshot.sha256).toMatch(/^[a-f0-9]{64}$/);
    expect(requests[0].template.sha256).not.toEqual(requests[0].screenshot.sha256);
    expect(requests[0].scales).toEqual([0.75, 1, 1.25, 1.5]);
    expect(requests[0].roi).toBeNull();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const review = page.getByRole("region", { name: "Chụp ảnh mẫu" });
    const reviewBounds = await review.boundingBox();
    expect(reviewBounds).not.toBeNull();
    expect(reviewBounds!.y).toBeGreaterThanOrEqual(0);
    expect(reviewBounds!.y + reviewBounds!.height).toBeLessThanOrEqual(viewport.height);
    await expect(page.getByRole("button", { name: "Dùng ảnh mẫu" })).toBeInViewport();
    const directory = resolve("../../target/feature-parity-20260913/ui");
    await mkdir(directory, { recursive: true });
    await page.screenshot({ path: resolve(directory, `template-diagnostic-${viewport.width}.png`) });
    await page.getByRole("button", { name: "Dùng ảnh mẫu" }).click();
    await expect(page.getByRole("img", { name: "Xem trước ảnh mẫu" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Kiểm tra ảnh mẫu" })).toHaveCount(0);
    expect((await mockCommandCalls(page)).filter((call) => ["flow_run", "tap", "type_text", "send_comment"].includes(call.command))).toEqual([]);
    expect(errors).toEqual([]);
  });
}
