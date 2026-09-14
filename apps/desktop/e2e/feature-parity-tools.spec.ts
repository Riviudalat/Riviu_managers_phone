import { openOperatorPage } from "./fixtures/operatorNavigation";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import type { OcrRequest } from "../src/api";
import { installTauriMock, mockCommandCalls } from "./fixtures/tauriMock";

const directory = resolve("../../target/feature-parity-complete-20260913/ui");

async function setup(page: Page, nodeKind: "ocrReadText" | "httpRequest") {
  await installTauriMock(page);
  await page.addInitScript(({ nodeKind }) => {
    type RecordValue = Record<string, unknown>;
    const host = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args: RecordValue) => Promise<unknown> }; __OCR_REQUESTS__: OcrRequest[] };
    host.__OCR_REQUESTS__ = [];
    const original = host.__TAURI_INTERNALS__.invoke;
    host.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "flow_action_catalog") {
        const catalog = await original(command, args) as RecordValue[];
        const base = catalog.find((entry) => entry.kind === "wait")!;
        return [...catalog, { ...base, kind: "ocrReadText", label: "Read OCR", category: "evidence", configSchema: { type: "object", properties: {} } },
          { ...base, kind: "httpRequest", label: "HTTP Request", category: "control", configSchema: { type: "object", properties: {} } }];
      }
      if (command === "flow_get") {
        const revision = await original(command, args) as { document: { nodes: RecordValue[] } };
        revision.document.nodes = revision.document.nodes.map((node) => node.kind === "wait"
          ? { ...node, kind: "launchApp", config: { bundleId: "com.fixture.app" } }
          : node.kind === "tap" ? { ...node, kind: nodeKind, config: nodeKind === "ocrReadText"
            ? { name: "caption", languages: ["vi", "en"], minConfidence: 0.7 }
            : { name: "response", url: "https://api.example.test/posts", method: "GET", credential: "test_credential" }, postcondition: null } : node);
        return revision;
      }
      if (command === "flow_connector_info") return { root: "C:\\Riviu Manager\\user-data\\flow-data", credentialNames: ["test_credential"], sheetConfigured: true };
      if (command === "flow_coordinate_frame") {
        const canvas = document.createElement("canvas"); canvas.width = 400; canvas.height = 650;
        const context = canvas.getContext("2d")!;
        context.fillStyle = "#f4f1ed"; context.fillRect(0, 0, 400, 650);
        context.fillStyle = "#203344"; context.fillRect(0, 0, 400, 90);
        context.fillStyle = "white"; context.font = "bold 25px sans-serif"; context.fillText("Riviu Manager", 24, 55);
        context.fillStyle = "#203344"; context.font = "25px sans-serif";
        context.fillText("Xin chào Việt Nam", 25, 160); context.fillText("Local OCR", 25, 220);
        return { jpegBase64: canvas.toDataURL("image/jpeg", 0.95).split(",")[1], imageWidth: 400, imageHeight: 650, orientation: "portrait", profileId: "fixture" };
      }
      if (command === "gui_ocr") {
        const request = args.request as OcrRequest;
        host.__OCR_REQUESTS__.push(request);
        return { protocolVersion: 1, requestId: request.requestId, observationId: request.observationId,
          sessionEpoch: request.sessionEpoch, generation: request.generation, screenshotSha256: request.screenshot.sha256,
          status: "resolved", text: "Xin chào Việt Nam\nLocal OCR", engine: "fixture Tesseract", elapsedMs: 202,
          lines: [{ text: "Xin chào Việt Nam", confidence: 0.964, bounds: { x: 25, y: 135, width: 240, height: 30 } },
            { text: "Local OCR", confidence: 0.95, bounds: { x: 25, y: 195, width: 180, height: 30 } }] };
      }
      return original(command, args);
    };
  }, { nodeKind });
  await page.goto("/");
  await page.getByTestId("device-tile").first().click({ modifiers: ["ControlOrMeta"] });
  await openOperatorPage(page, 'Flow');
  await expect(page.getByLabel("Tên Flow")).toHaveValue("Cuộn nội dung");
}

async function screenshot(page: Page, name: string) {
  await mkdir(directory, { recursive: true });
  await page.screenshot({ path: resolve(directory, name) });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
}

for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 700 }]) {
  test(`OCR inspector diagnosis stays bounded at ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await setup(page, "ocrReadText");
    await page.getByTestId("flow-node-title").filter({ hasText: "Đọc chữ từ ảnh" }).click();
    await expect(page.getByLabel("Biến lưu nội dung")).toHaveValue("caption");
    await page.getByRole("button", { name: "Chụp ảnh kiểm tra OCR" }).click();
    await page.getByRole("button", { name: "Kiểm tra OCR", exact: true }).click();
    await expect(page.getByLabel("Nội dung OCR")).toHaveValue("Xin chào Việt Nam\nLocal OCR");
    await screenshot(page, `ocr-diagnostic-${viewport.width}.png`);
    const bounds = await page.getByRole("region", { name: "Chụp ảnh mẫu" }).boundingBox();
    expect(bounds!.y).toBeGreaterThanOrEqual(0);
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(viewport.height);
    await expect(page.getByRole("button", { name: "Đóng kiểm tra OCR" })).toBeInViewport();
    const requests = await page.evaluate(() => (window as unknown as { __OCR_REQUESTS__: OcrRequest[] }).__OCR_REQUESTS__);
    expect(requests).toHaveLength(1);
    expect(requests[0].languages).toEqual(["vi", "en"]);
    expect(requests[0].screenshot.sha256).toMatch(/^[a-f0-9]{64}$/);
    await page.getByRole("button", { name: "Đóng kiểm tra OCR" }).click();
    expect((await mockCommandCalls(page)).filter((call) => ["flow_run", "tap", "type_text"].includes(call.command))).toEqual([]);
  });

  test(`connector inspector settings stay readable at ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await setup(page, "httpRequest");
    await page.getByTestId("flow-node-title").filter({ hasText: "Gọi HTTP" }).click();
    await page.getByText("Kết nối dữ liệu: tệp, HTTP và Google Sheet", { exact: true }).click();
    await page.getByLabel("Token", { exact: true }).scrollIntoViewIfNeeded();
    await expect(page.getByLabel("Token", { exact: true })).toHaveAttribute("type", "password");
    await expect(page.getByRole("button", { name: "Lưu token" })).toBeDisabled();
    await screenshot(page, `connector-settings-${viewport.width}.png`);
    expect(await page.getByTestId("flow-inspector").evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    expect((await mockCommandCalls(page)).filter((call) => ["flow_connector_save_secret", "flow_connector_import_file", "flow_run"].includes(call.command))).toEqual([]);
  });

  test(`subflow composition has accessible inputs and footer at ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await setup(page, "ocrReadText");
    await page.getByRole("button", { name: "Ghép hoặc chỉnh Flow con", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Ghép Flow con" });
    await expect(dialog).toBeVisible();
    await page.getByLabel("Cách thực hiện").selectOption("repeat");
    await page.getByLabel("Số lượt Flow con").fill("3");
    await screenshot(page, `subflow-composition-top-${viewport.width}.png`);
    await page.getByRole("button", { name: "Áp dụng Flow con" }).scrollIntoViewIfNeeded();
    await expect(page.getByLabel("Biến đầu ra (biến cha: biến con)")).toBeInViewport();
    await expect(page.getByRole("button", { name: "Áp dụng Flow con" })).toBeInViewport();
    await screenshot(page, `subflow-composition-bottom-${viewport.width}.png`);
    expect(await dialog.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    const bounds = await dialog.boundingBox();
    expect(bounds!.y).toBeGreaterThanOrEqual(0);
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(viewport.height);
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
  });
}
