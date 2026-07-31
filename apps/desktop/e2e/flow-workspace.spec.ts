import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  emitRiviuEvent,
  installTauriMock,
  mockCommandCalls,
  setNextRunMode,
} from "./fixtures/tauriMock";

const FLOW_NODE_TITLE = ".flow-node-title";

async function openFlow(page: Page, selectDevices = false): Promise<void> {
  await page.goto("/");
  await expect(page.locator(".dev-window")).toHaveCount(2);
  if (selectDevices) {
    for (const udid of ["MOCK-IPHONE-01", "MOCK-IPHONE-02"]) {
      await page.locator(".dev-window", { hasText: udid.replace("MOCK-IPHONE-", "Fixture iPhone ") })
        .getByRole("checkbox")
        .check({ force: true });
    }
  }
  await page.locator(".menu-item").getByText("Flow", { exact: true }).click();
  await expect(page.getByRole("region", { name: "Flow workspace" })).toHaveAttribute(
    "data-loading",
    "false",
  );
  await expect(page.getByLabel("Flow name")).toHaveValue("Fixture flow");
  await expect(page.getByTestId("flow-canvas")).toBeVisible();
}

async function insertActionOnFirstEdge(page: Page, action: string): Promise<Locator> {
  const actionKinds: Record<string, string> = {
    Screenshot: "screenshot",
    Tap: "tap",
    Wait: "wait",
  };
  const kind = actionKinds[action];
  if (!kind) throw new Error(`Unsupported E2E action: ${action}`);
  const before = await page.locator(FLOW_NODE_TITLE).filter({ hasText: action }).count();
  await page.locator(".react-flow__edge-interaction").first().evaluate((edge) => {
    edge.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, composed: true }));
  });
  const source = page.getByTestId("flow-palette").getByRole("button", {
    name: action,
    exact: true,
  });
  await source.evaluate((element, payload) => {
    const target = document.querySelector<HTMLElement>("[data-testid='flow-canvas'] .react-flow");
    if (!target) throw new Error("Flow canvas drop target is missing");
    const transfer = new DataTransfer();
    const targetRect = target.getBoundingClientRect();
    const sourceRect = element.getBoundingClientRect();
    element.dispatchEvent(new DragEvent("dragstart", {
      bubbles: true,
      cancelable: true,
      composed: true,
      clientX: sourceRect.left + sourceRect.width / 2,
      clientY: sourceRect.top + sourceRect.height / 2,
      dataTransfer: transfer,
    }));
    transfer.setData("application/riviu-flow-action", payload.kind);
    for (const type of ["dragenter", "dragover", "drop"] as const) {
      target.dispatchEvent(new DragEvent(type, {
        bubbles: true,
        cancelable: true,
        composed: true,
        clientX: targetRect.left + Math.min(360, targetRect.width / 2),
        clientY: targetRect.top + Math.min(220, targetRect.height / 2),
        dataTransfer: transfer,
      }));
    }
    element.dispatchEvent(new DragEvent("dragend", {
      bubbles: true,
      cancelable: true,
      composed: true,
      dataTransfer: transfer,
    }));
  }, { kind });
  const titles = page.locator(FLOW_NODE_TITLE).filter({ hasText: action });
  await expect(titles).toHaveCount(before + 1);
  const inserted = titles.last();
  await inserted.evaluate((title) => {
    title.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, composed: true }));
  });
  return inserted;
}

async function waitForEnabled(locator: Locator): Promise<void> {
  await expect(locator).toBeEnabled({ timeout: 5_000 });
}

test.beforeEach(async ({ page }, testInfo) => {
  testInfo.annotations.push({ type: "fixture", description: "FIXTURE_ONLY" });
  await page.route("https://fonts.googleapis.com/**", (route) => route.fulfill({
    status: 200,
    contentType: "text/css",
    body: "",
  }));
  await page.route("https://fonts.gstatic.com/**", (route) => route.abort("blockedbyclient"));
  await installTauriMock(page);
});

test("authors, saves, runs, and reloads a selected-device flow", async ({ page }) => {
  await openFlow(page, true);
  await page.getByRole("button", { name: "New flow" }).click();
  await page.getByLabel("Flow name").fill("E2E flow");

  await insertActionOnFirstEdge(page, "Wait");
  await page.getByLabel("Duration (ms)").fill("250");

  await insertActionOnFirstEdge(page, "Tap");
  await page.getByLabel("Accessibility ID", { exact: true }).fill("like-button");
  await page.getByLabel("Evidence type").selectOption("frameDigestChanged");
  await page.getByLabel("Minimum distance").fill("8");

  await insertActionOnFirstEdge(page, "Screenshot");
  await page.getByLabel("Label", { exact: true }).fill("E2E evidence");
  await page.getByLabel("Evidence type").selectOption("artifactDecodedAndHashed");

  const save = page.getByRole("button", { name: "Save revision" });
  await waitForEnabled(save);
  await page.getByRole("button", { name: "Validate flow" }).click();
  await expect(page.getByRole("dialog", { name: "Compile preview" })).toContainText("Valid");
  await page.getByRole("dialog", { name: "Compile preview" }).getByRole("button", {
    name: "Close",
  }).click();
  await save.click();

  const run = page.getByRole("button", { name: "Run flow" });
  await waitForEnabled(run);
  await run.click();
  const runDialog = page.getByRole("dialog", { name: "Run flow" });
  await runDialog.getByRole("radio", { name: "Selected" }).check();
  await expect(runDialog.getByText("2 selected")).toBeVisible();
  await runDialog.getByRole("button", { name: "Run on devices" }).click();

  await expect(page.getByRole("row", { name: /MOCK-IPHONE-01.*Wait.*Succeeded/ })).toBeVisible();
  await expect(page.getByRole("row", { name: /MOCK-IPHONE-02.*Wait.*Succeeded/ })).toBeVisible();
  await page.getByRole("button", { name: "Fixture screenshot 1" }).click();
  const artifact = page.getByRole("dialog", { name: "Artifact" });
  await expect(artifact).toContainText("Fixture screenshot");
  await expect.poll(async () => artifact.getByRole("img").evaluate((image) =>
    (image as HTMLImageElement).naturalWidth
  )).toBeGreaterThan(0);
  await artifact.getByRole("button", { name: "Close" }).click();
  const calls = await mockCommandCalls(page);
  expect(calls).toContainEqual(expect.objectContaining({
    command: "flow_save_revision",
    args: expect.objectContaining({ expectedRevision: null }),
  }));
  expect(calls).toContainEqual(expect.objectContaining({
    command: "flow_run",
    args: expect.objectContaining({
      selection: {
        mode: "selected",
        udids: ["MOCK-IPHONE-01", "MOCK-IPHONE-02"],
      },
    }),
  }));

  await page.reload();
  await page.locator(".menu-item").getByText("Flow", { exact: true }).click();
  await expect(page.getByRole("region", { name: "Flow workspace" })).toHaveAttribute(
    "data-loading",
    "false",
  );
  await expect(page.getByLabel("Flow name")).toHaveValue("E2E flow");
  await expect(page.locator(FLOW_NODE_TITLE).filter({ hasText: "Wait" })).toHaveCount(1);
  await expect(page.locator(FLOW_NODE_TITLE).filter({ hasText: "Tap" })).toHaveCount(1);
  await expect(page.locator(FLOW_NODE_TITLE).filter({ hasText: "Screenshot" })).toHaveCount(1);
});

test("keeps uncertain Tap non-retryable and cancels a running Wait", async ({ page }) => {
  await openFlow(page, true);
  await setNextRunMode(page, "uncertainTap");
  await expect.poll(async () =>
    (await mockCommandCalls(page)).filter((call) => call.command === "flow_validate").length
  ).toBeGreaterThan(0);
  await page.getByRole("button", { name: "Validate flow" }).click();
  const preview = page.getByRole("dialog", { name: "Compile preview" });
  await expect(preview).toContainText("Valid");
  await preview.getByRole("button", { name: "Close" }).click();
  const run = page.getByRole("button", { name: "Run flow" });
  await waitForEnabled(run);
  await run.click();
  await page.getByRole("dialog", { name: "Run flow" })
    .getByRole("button", { name: "Run on devices" })
    .click();
  await expect(page.getByTestId("flow-monitor")).toContainText("Uncertain");
  await expect(page.getByRole("button", { name: /Retry Tap/ })).toHaveCount(0);

  await setNextRunMode(page, "runningWait");
  await run.click();
  await page.getByRole("dialog", { name: "Run flow" })
    .getByRole("button", { name: "Run on devices" })
    .click();
  await expect(page.getByTestId("flow-monitor")).toContainText("Running");
  const runId = await page.locator(".flow-run-history select").inputValue();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await emitRiviuEvent(page, { type: "flowRunUpdated", runId, revision: 2 });
  await expect(page.getByTestId("flow-monitor")).toContainText("Cancelled");
  await expect(page.getByRole("button", { name: "Cancel", exact: true })).toBeDisabled();
});

test("imports supported legacy JSON and preserves the draft on diagnostics", async ({ page }) => {
  await openFlow(page);
  await page.getByRole("button", { name: "Import flow" }).click();
  let dialog = page.getByRole("dialog", { name: "Import legacy flow" });
  await dialog.getByLabel("Legacy script JSON").fill(JSON.stringify({
    version: 1,
    name: "supported",
    steps: [{ action: "wait", milliseconds: 250 }],
  }));
  await dialog.getByRole("button", { name: "Import", exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByLabel("Flow name")).toHaveValue("Imported legacy flow");
  const nodeCount = await page.locator(".flow-node").count();

  await page.getByRole("button", { name: "Import flow" }).click();
  dialog = page.getByRole("dialog", { name: "Import legacy flow" });
  await dialog.getByLabel("Legacy script JSON").fill(JSON.stringify({
    version: 1,
    name: "unsupported",
    steps: [{ action: "wait", milliseconds: 60_001 }],
  }));
  await dialog.getByRole("button", { name: "Import", exact: true }).click();
  await expect(dialog.getByText("WaitOutOfRange")).toBeVisible();
  await expect(page.locator(".flow-node")).toHaveCount(nodeCount);
});

test("legacy scripts and jobs remain reachable", async ({ page }) => {
  await openFlow(page);
  await page.getByRole("tab", { name: "Legacy" }).click();
  await expect(page.getByRole("heading", { name: "Scripts" })).toBeVisible();
  await page.getByRole("button", { name: "Use in Jobs" }).first().click();
  await expect(page.locator(".topbar-title", { hasText: "Jobs" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Run script" })).toBeVisible();
});

interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

function overlaps(a: Box, b: Box): boolean {
  return a.x < b.x + b.width && a.x + a.width > b.x &&
    a.y < b.y + b.height && a.y + a.height > b.y;
}

for (const viewport of [
  { width: 1440, height: 900 },
  { width: 900, height: 700 },
]) {
  test(`contains the FIXTURE_ONLY Flow workspace at ${viewport.width}x${viewport.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await openFlow(page);
    await page.locator(FLOW_NODE_TITLE).filter({ hasText: "Tap" }).click();
    if (viewport.width <= 1100) {
      await page.getByRole("button", { name: "Toggle action palette" }).click();
      await expect(page.getByTestId("flow-palette")).toHaveAttribute("data-open", "false");
      await expect(page.getByRole("button", { name: "Run flow" })).toBeInViewport();
      await expect(page.getByRole("button", { name: "Toggle inspector" })).toBeInViewport();
    }

    const canvas = await page.getByTestId("flow-canvas").boundingBox();
    expect(canvas).not.toBeNull();
    expect(canvas?.width).toBeGreaterThanOrEqual(420);
    expect(await page.locator(".flow-node").count()).toBeGreaterThan(0);
    const regionBoxes = (await Promise.all([
      page.getByTestId("flow-toolbar").boundingBox(),
      page.getByTestId("flow-palette").boundingBox(),
      page.getByTestId("flow-canvas").boundingBox(),
      page.getByTestId("flow-inspector").boundingBox(),
      page.getByTestId("flow-monitor").boundingBox(),
    ])).filter((box): box is Box => box !== null);
    for (let left = 0; left < regionBoxes.length; left += 1) {
      for (let right = left + 1; right < regionBoxes.length; right += 1) {
        expect(overlaps(regionBoxes[left], regionBoxes[right])).toBe(false);
      }
    }
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    expect(await page.locator("button:visible, .flow-node-title:visible").evaluateAll((elements) =>
      elements.every((element) => element.scrollWidth <= element.clientWidth)
    )).toBe(true);
    await expect(page).toHaveScreenshot(
      `fixture-only-flow-${viewport.width}x${viewport.height}.png`,
      {
        fullPage: true,
        animations: "disabled",
        maxDiffPixelRatio: 0.002,
      },
    );
  });
}
