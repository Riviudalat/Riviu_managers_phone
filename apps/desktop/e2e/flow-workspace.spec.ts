import { expect, test, type Locator, type Page } from "@playwright/test";
import {
  emitRiviuEvent,
  installTauriMock,
  mockCommandCalls,
  setNextRunMode,
} from "./fixtures/tauriMock";

const FLOW_NODE_TITLE = "[data-testid='flow-node-title']";

async function openFlow(page: Page, selectDevices = false): Promise<void> {
  await page.goto("/");
  await expect(page.locator("[data-testid='device-tile']")).toHaveCount(2);
  if (selectDevices) {
    // Ctrl-click, because the tile's corner checkbox was removed on request: selection is the
    // tile's own click now, additive on ctrl/meta/shift (`onSelect` in App.tsx). Held down for
    // the first tile too — additive on an empty selection still just adds it, and asking for
    // "additive" explicitly says what this loop means rather than relying on plain-click
    // replace-then-extend ordering.
    for (const udid of ["MOCK-IPHONE-01", "MOCK-IPHONE-02"]) {
      await page
        .locator("[data-testid='device-tile']", {
          hasText: udid.replace("MOCK-IPHONE-", "Fixture iPhone "),
        })
        .click({ modifiers: ["ControlOrMeta"] });
    }
    await expect(page.locator("[data-testid='device-tile'].selected")).toHaveCount(2);
  }
  await page.locator("[data-testid='nav-item']").getByText("Flow", { exact: true }).click();
  await expect(page.getByRole("region", { name: "Không gian Flow" })).toHaveAttribute(
    "data-loading",
    "false",
  );
  await expect(page.getByLabel("Tên Flow")).toHaveValue("Fixture flow");
  await expect(page.getByTestId("flow-canvas")).toBeVisible();
}

async function insertActionOnFirstEdge(page: Page, action: string): Promise<Locator> {
  const actionKinds: Record<string, string> = {
    "Chụp màn hình": "screenshot",
    "Chạm": "tap",
    "Chờ": "wait",
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
    // **Set the payload before `dragstart`, not after, and check it survived.**
    //
    // A `DataTransfer` is only guaranteed writable while its drag is in the read/write state.
    // Writing after the synthetic `dragstart` has been dispatched relies on Chromium leaving
    // it writable, and when that does not hold `setData` fails silently -- `getData` in
    // `FlowCanvas.drop` then returns `""`, the drop is refused, and the assertion sees zero
    // new nodes with a perfectly healthy-looking DOM. That is one CI failure spent on a
    // 5-second timeout and a screenshot that showed nothing wrong (run 33069434865).
    //
    // The palette's own `onDragStart` sets the same key, so this is belt and braces -- but
    // belt and braces is the point: neither path is guaranteed to run in a synthetic drag,
    // and the assert below turns "silently empty" into a named failure.
    transfer.setData("application/riviu-flow-action", payload.kind);
    element.dispatchEvent(new DragEvent("dragstart", {
      bubbles: true,
      cancelable: true,
      composed: true,
      clientX: sourceRect.left + sourceRect.width / 2,
      clientY: sourceRect.top + sourceRect.height / 2,
      dataTransfer: transfer,
    }));
    if (transfer.getData("application/riviu-flow-action") !== payload.kind) {
      throw new Error(
        `drag payload lost before drop: getData -> ${JSON.stringify(
          transfer.getData("application/riviu-flow-action"),
        )}. The DataTransfer went read-only, so the drop would be refused with no node.`,
      );
    }
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

test("says why it refused a drop instead of doing nothing", async ({ page }) => {
  await openFlow(page);
  const before = await page.locator(FLOW_NODE_TITLE).count();

  // A drag that carries nothing — a file, a text selection, a link from another window. The
  // real palette always sets the payload, so this branch is only reachable from outside it.
  await page.getByTestId("flow-canvas").evaluate((canvas) => {
    const target = canvas.querySelector<HTMLElement>(".react-flow");
    if (!target) throw new Error("Flow canvas drop target is missing");
    const rect = target.getBoundingClientRect();
    for (const type of ["dragenter", "dragover", "drop"] as const) {
      target.dispatchEvent(new DragEvent(type, {
        bubbles: true,
        cancelable: true,
        composed: true,
        clientX: rect.left + rect.width / 2,
        clientY: rect.top + rect.height / 2,
        dataTransfer: new DataTransfer(),
      }));
    }
  });

  // The point of the change: a refused gesture produces a sentence, not silence.
  await expect(
    page.getByRole("region", { name: "Thông báo" }).getByText("Không nhận ra thứ được kéo vào"),
  ).toBeVisible();
  // And it stays a refusal — no node is invented to make the drag look like it worked.
  await expect(page.locator(FLOW_NODE_TITLE)).toHaveCount(before);
});

test("deleting a node keeps the path it was standing in", async ({ page }) => {
  // The one level where this is a real test. `FlowCanvas` is mocked in the unit suite, so nothing
  // below e2e exercises React Flow's actual deletion sequence -- and that sequence is the bug:
  // `deleteElements` fires `onEdgesChange` for the incident edges *before* `onNodesDelete`, so
  // while those two callbacks committed separately, the node delete ran on a document that had
  // already lost the edges it needed in order to reconnect. One Delete keypress on `Start -> Chờ
  // -> End` left Start and End with no path between them, and the operator's next save wrote that
  // broken graph.
  await openFlow(page);
  const nodes = page.locator("[data-testid='flow-node']");
  const edges = page.locator(".react-flow__edge");
  const waits = page.locator(FLOW_NODE_TITLE).filter({ hasText: "Chờ" });
  const nodesBefore = await nodes.count();
  const edgesBefore = await edges.count();
  // The fixture flow already contains a Chờ node, so the inserted one is counted, not named.
  const waitsBefore = await waits.count();

  const inserted = await insertActionOnFirstEdge(page, "Chờ");
  await expect(nodes).toHaveCount(nodesBefore + 1);
  await expect(edges).toHaveCount(edgesBefore + 1);
  // A real click, not a dispatched one: React Flow listens for the delete key on `document`, and
  // the synthetic click `insertActionOnFirstEdge` uses selects the node without moving focus, so
  // the keypress never reaches the handler. The operator's click does both.
  await inserted.click();
  await expect(page.locator("[data-testid='flow-node'][data-selected='true']")).toHaveCount(1);

  await page.keyboard.press("Delete");

  await expect(waits).toHaveCount(waitsBefore);
  await expect(nodes).toHaveCount(nodesBefore);
  // The assertion that matters: the graph is back to the path it had, not one edge short of it.
  await expect(edges).toHaveCount(edgesBefore);

  // And it cost one history entry, so one Undo brings back the node *and* its wiring. Two
  // separate mutations meant the first Undo restored a node with nothing attached to it.
  await page.getByRole("button", { name: "Hoàn tác" }).click();
  await expect(waits).toHaveCount(waitsBefore + 1);
  await expect(edges).toHaveCount(edgesBefore + 1);
});

test("authors, saves, runs, and reloads a selected-device flow", async ({ page }) => {
  await openFlow(page, true);
  await page.getByRole("button", { name: "Flow mới" }).click();
  await page.getByLabel("Tên Flow").fill("E2E flow");

  await insertActionOnFirstEdge(page, "Chờ");
  await page.getByLabel("Thời lượng (ms)").fill("250");

  await insertActionOnFirstEdge(page, "Chạm");
  await page.getByLabel("Accessibility ID", { exact: true }).fill("like-button");
  await page.getByLabel("Loại bằng chứng").selectOption("frameDigestChanged");
  await page.getByLabel("Khoảng cách tối thiểu").fill("8");

  await insertActionOnFirstEdge(page, "Chụp màn hình");
  await page.getByLabel("Nhãn", { exact: true }).fill("E2E evidence");
  await page.getByLabel("Loại bằng chứng").selectOption("artifactDecodedAndHashed");

  const save = page.getByRole("button", { name: "Lưu bản" });
  await waitForEnabled(save);
  await page.getByRole("button", { name: "Kiểm tra Flow" }).click();
  await expect(page.getByRole("dialog", { name: "Xem trước biên dịch" })).toContainText("Valid");
  await page.getByRole("dialog", { name: "Xem trước biên dịch" }).getByRole("button", {
    name: "Đóng",
  }).click();
  await save.click();

  const run = page.getByRole("button", { name: "Chạy Flow" });
  await waitForEnabled(run);
  await run.click();
  const runDialog = page.getByRole("dialog", { name: "Chạy Flow" });
  await runDialog.getByRole("radio", { name: "Đã chọn" }).check();
  await expect(runDialog.getByText("2 selected")).toBeVisible();
  await runDialog.getByRole("button", { name: "Chạy trên thiết bị" }).click();

  await expect(page.getByRole("row", { name: /MOCK-IPHONE-01.*Wait.*Succeeded/ })).toBeVisible();
  await expect(page.getByRole("row", { name: /MOCK-IPHONE-02.*Wait.*Succeeded/ })).toBeVisible();
  await page.getByRole("button", { name: "Fixture screenshot 1" }).click();
  const artifact = page.getByRole("dialog", { name: "Tệp kết quả" });
  await expect(artifact).toContainText("Fixture screenshot");
  await expect.poll(async () => artifact.getByRole("img").evaluate((image) =>
    (image as HTMLImageElement).naturalWidth
  )).toBeGreaterThan(0);
  await artifact.getByRole("button", { name: "Đóng" }).click();
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
  await page.locator("[data-testid='nav-item']").getByText("Flow", { exact: true }).click();
  await expect(page.getByRole("region", { name: "Không gian Flow" })).toHaveAttribute(
    "data-loading",
    "false",
  );
  await expect(page.getByLabel("Tên Flow")).toHaveValue("E2E flow");
  await expect(page.locator(FLOW_NODE_TITLE).filter({ hasText: "Chờ" })).toHaveCount(1);
  await expect(page.locator(FLOW_NODE_TITLE).filter({ hasText: "Chạm" })).toHaveCount(1);
  await expect(page.locator(FLOW_NODE_TITLE).filter({ hasText: "Chụp màn hình" })).toHaveCount(1);
});

test("keeps uncertain Tap non-retryable and cancels a running Wait", async ({ page }) => {
  await openFlow(page, true);
  await setNextRunMode(page, "uncertainTap");
  await expect.poll(async () =>
    (await mockCommandCalls(page)).filter((call) => call.command === "flow_validate").length
  ).toBeGreaterThan(0);
  await page.getByRole("button", { name: "Kiểm tra Flow" }).click();
  const preview = page.getByRole("dialog", { name: "Xem trước biên dịch" });
  await expect(preview).toContainText("Valid");
  await preview.getByRole("button", { name: "Đóng" }).click();
  const run = page.getByRole("button", { name: "Chạy Flow" });
  await waitForEnabled(run);
  await run.click();
  await page.getByRole("dialog", { name: "Chạy Flow" })
    .getByRole("button", { name: "Chạy trên thiết bị" })
    .click();
  await expect(page.getByTestId("flow-monitor")).toContainText("Uncertain");
  await expect(page.getByRole("button", { name: /Retry Tap/ })).toHaveCount(0);

  await setNextRunMode(page, "runningWait");
  await run.click();
  await page.getByRole("dialog", { name: "Chạy Flow" })
    .getByRole("button", { name: "Chạy trên thiết bị" })
    .click();
  await expect(page.getByTestId("flow-monitor")).toContainText("Running");
  const runId = await page.locator("[data-testid='flow-run-history'] select").inputValue();
  await page.getByRole("button", { name: "Hủy", exact: true }).click();
  await emitRiviuEvent(page, { type: "flowRunUpdated", runId, revision: 2 });
  await expect(page.getByTestId("flow-monitor")).toContainText("Cancelled");
  await expect(page.getByRole("button", { name: "Hủy", exact: true })).toBeDisabled();
});

test("imports supported legacy JSON and preserves the draft on diagnostics", async ({ page }) => {
  await openFlow(page);
  await page.getByRole("button", { name: "Nhập Flow" }).click();
  let dialog = page.getByRole("dialog", { name: "Nhập Flow cũ" });
  await dialog.getByLabel("JSON script cũ").fill(JSON.stringify({
    version: 1,
    name: "supported",
    steps: [{ action: "wait", milliseconds: 250 }],
  }));
  await dialog.getByRole("button", { name: "Import", exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByLabel("Tên Flow")).toHaveValue("Imported legacy flow");
  const nodeCount = await page.locator("[data-testid='flow-node']").count();

  await page.getByRole("button", { name: "Nhập Flow" }).click();
  dialog = page.getByRole("dialog", { name: "Nhập Flow cũ" });
  await dialog.getByLabel("JSON script cũ").fill(JSON.stringify({
    version: 1,
    name: "unsupported",
    steps: [{ action: "wait", milliseconds: 60_001 }],
  }));
  await dialog.getByRole("button", { name: "Import", exact: true }).click();
  await expect(dialog.getByText("WaitOutOfRange")).toBeVisible();
  await expect(page.locator("[data-testid='flow-node']")).toHaveCount(nodeCount);
});

test("legacy scripts and jobs remain reachable", async ({ page }) => {
  await openFlow(page);
  await page.getByRole("tab", { name: "Legacy" }).click();
  await expect(page.getByRole("heading", { name: "Kịch bản" })).toBeVisible();
  await page.getByRole("button", { name: "Dùng ở Tác vụ" }).first().click();
  await expect(page.locator("[data-testid='page-title']", { hasText: "Tác vụ" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Chạy kịch bản" })).toBeVisible();
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
    await page.locator(FLOW_NODE_TITLE).filter({ hasText: "Chạm" }).click();
    if (viewport.width <= 1100) {
      await page.getByRole("button", { name: "Bật/tắt bảng hành động" }).click();
      await expect(page.getByTestId("flow-palette")).toHaveAttribute("data-open", "false");
      await expect(page.getByRole("button", { name: "Chạy Flow" })).toBeInViewport();
      await expect(page.getByRole("button", { name: "Bật/tắt bảng thuộc tính" })).toBeInViewport();
    }

    const canvas = await page.getByTestId("flow-canvas").boundingBox();
    expect(canvas).not.toBeNull();
    expect(canvas?.width).toBeGreaterThanOrEqual(420);
    expect(await page.locator("[data-testid='flow-node']").count()).toBeGreaterThan(0);
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
    expect(await page.locator("button:visible, [data-testid='flow-node-title']:visible").evaluateAll((elements) =>
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
