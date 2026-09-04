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
  await page.getByRole("button", { name: "Flow", exact: true }).click();
  await expect(page.getByRole("region", { name: "Không gian Flow" })).toHaveAttribute(
    "data-loading",
    "false",
  );
  await expect(page.getByLabel("Tên Flow")).toHaveValue("Cuộn nội dung");
  await expect(page.getByTestId("flow-canvas")).toBeVisible();
}

async function insertActionOnFirstEdge(page: Page, action: string): Promise<Locator> {
  const actionKinds: Record<string, string> = {
    "Chụp màn hình": "screenshot",
    "Chạm": "tap",
    "Chờ": "wait",
    "Tự động vuốt": "autoSwipe",
  };
  const kind = actionKinds[action];
  if (!kind) throw new Error(`Unsupported E2E action: ${action}`);
  const before = await page.locator(FLOW_NODE_TITLE).filter({ hasText: action }).count();
  // Select the exact edge before dropping. This test is about structural insertion/deletion, not
  // viewport measurement; geometric nearest-edge behaviour has its own component regression.
  // Re-resolve and dispatch to the hit path inside `toPass` because React Flow can replace its
  // SVG once while the initial ResizeObserver measurement settles under parallel browser load.
  // A coordinate click can then land on the newly measured canvas even though the old path gave
  // us that coordinate; dispatching on the freshly resolved path still exercises React Flow's
  // click handler without carrying stale geometry across the replacement.
  const edgeInteraction = page.locator(".react-flow__edge-interaction").first();
  const selectedEdge = page.locator(".react-flow__edge.selected");
  await expect(async () => {
    if (await selectedEdge.count() === 0) {
      await edgeInteraction.dispatchEvent("click");
    }
    await expect(selectedEdge).toHaveCount(1, { timeout: 500 });
  }).toPass({ intervals: [50, 100, 250], timeout: 5_000 });
  const dropPoint = await page.getByTestId("flow-canvas").evaluate((canvas) => {
    const bounds = canvas.getBoundingClientRect();
    return { x: bounds.left + bounds.width / 2, y: bounds.top + bounds.height / 2 };
  });
  const source = page.getByTestId("flow-palette").getByRole("button", {
    name: action,
    exact: true,
  });
  await source.evaluate((element, payload) => {
    const target = document.querySelector<HTMLElement>("[data-testid='flow-canvas'] .react-flow");
    if (!target) throw new Error("Flow canvas drop target is missing");
    const transfer = new DataTransfer();
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
        clientX: payload.dropPoint.x,
        clientY: payload.dropPoint.y,
        dataTransfer: transfer,
      }));
    }
    element.dispatchEvent(new DragEvent("dragend", {
      bubbles: true,
      cancelable: true,
      composed: true,
      dataTransfer: transfer,
    }));
  }, { kind, dropPoint });
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

test("moving a node with the keyboard reaches the document", async ({ page }) => {
  // React Flow moves a selected node with the arrow keys through `moveSelectedNodes`, which emits
  // a position change and **no drag-stop event**. Positions were committed only by
  // `onNodeDragStop`, so the node visibly moved and the document never heard: the draft stayed
  // clean, a reload restored the old coordinates, and a save that happened for another reason
  // wrote the old ones back so the node snapped.
  //
  // Only e2e can see this. `FlowCanvas` is mocked in the unit suite, and the behaviour lives
  // entirely in React Flow's key handling plus the change it emits.
  await openFlow(page);
  const node = page.locator("[data-testid='flow-node']").first();
  const before = await node.boundingBox();
  expect(before).not.toBeNull();

  await node.click();
  await expect(page.locator("[data-testid='flow-node'][data-selected='true']")).toHaveCount(1);
  // Dirty is what says the document heard. The toolbar's Save is the visible proof of it.
  await expect(page.getByRole("button", { name: "Xuất Flow" })).toBeEnabled();

  for (let press = 0; press < 4; press += 1) {
    await page.keyboard.press("ArrowRight");
  }

  const after = await node.boundingBox();
  expect(after).not.toBeNull();
  expect(after!.x).toBeGreaterThan(before!.x);

  // The document is dirty now, which it was not before, and Export is gated on a clean flow —
  // so its going disabled is the document saying it took the move.
  await expect(page.getByRole("button", { name: "Xuất Flow" })).toBeDisabled();

  // One history entry per press, not two: four presses, four Undos, back where it started.
  for (let undo = 0; undo < 4; undo += 1) {
    await page.getByRole("button", { name: "Hoàn tác" }).click();
  }
  const restored = await node.boundingBox();
  expect(restored).not.toBeNull();
  expect(Math.abs(restored!.x - before!.x)).toBeLessThan(1);
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
  await expect(page.getByRole("dialog", { name: "Xem trước biên dịch" })).toContainText("Hợp lệ");
  await page.getByRole("dialog", { name: "Xem trước biên dịch" }).getByRole("button", {
    name: "Đóng",
  }).click();
  await save.click();

  const run = page.getByRole("button", { name: "Chạy Flow" });
  await waitForEnabled(run);
  await run.click();
  const runDialog = page.getByRole("dialog", { name: "Chạy Flow" });
  await runDialog.getByRole("radio", { name: "Đã chọn" }).check();
  await expect(runDialog.getByText("2 máy đã chọn")).toBeVisible();
  await runDialog.getByRole("button", { name: "Chạy trên thiết bị" }).click();

  await expect(page.getByRole("row", { name: /Máy 1.*Chờ.*Thành công/ })).toBeVisible();
  await expect(page.getByRole("row", { name: /Máy 2.*Chờ.*Thành công/ })).toBeVisible();
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
  await expect(preview).toContainText("Hợp lệ");
  await preview.getByRole("button", { name: "Đóng" }).click();
  const run = page.getByRole("button", { name: "Chạy Flow" });
  await waitForEnabled(run);
  await run.click();
  await page.getByRole("dialog", { name: "Chạy Flow" })
    .getByRole("button", { name: "Chạy trên thiết bị" })
    .click();
  await expect(page.getByTestId("flow-monitor")).toContainText("Chưa chắc chắn");
  await expect(page.getByRole("button", { name: /Chạy lại Chạm/ })).toHaveCount(0);

  await setNextRunMode(page, "runningWait");
  await run.click();
  await page.getByRole("dialog", { name: "Chạy Flow" })
    .getByRole("button", { name: "Chạy trên thiết bị" })
    .click();
  await expect(page.getByTestId("flow-monitor")).toContainText("Đang chạy");
  const runId = await page.locator("[data-testid='flow-run-history'] select").inputValue();
  await page.getByRole("button", { name: "Hủy", exact: true }).click();
  await emitRiviuEvent(page, { type: "flowRunUpdated", runId, revision: 2 });
  await expect(page.getByTestId("flow-monitor")).toContainText("Đã hủy");
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
  await dialog.getByRole("button", { name: "Nhập", exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByLabel("Tên Flow")).toHaveValue("Imported legacy flow");
  const nodeCount = await page.locator("[data-testid='flow-node']").count();

  // The imported flow is unsaved work, and a second import would replace it — so this click now
  // asks first, the same way Flow mới and Nhân bản do. Confirming is what an operator does; the
  // point of the prompt is that it exists at all.
  await page.getByRole("button", { name: "Nhập Flow" }).click();
  await page.getByRole("button", { name: "Bỏ thay đổi" }).click();
  dialog = page.getByRole("dialog", { name: "Nhập Flow cũ" });
  await dialog.getByLabel("JSON script cũ").fill(JSON.stringify({
    version: 1,
    name: "unsupported",
    steps: [{ action: "wait", milliseconds: 60_001 }],
  }));
  await dialog.getByRole("button", { name: "Nhập", exact: true }).click();
  await expect(dialog.getByText("Bước 1: Không thể nhập hành động này.")).toBeVisible();
  await expect(dialog.getByText(/WaitOutOfRange/)).toBeHidden();
  await expect(page.locator("[data-testid='flow-node']")).toHaveCount(nodeCount);
});

test("keeps device Flow separate from fleet orchestration", async ({ page }) => {
  await openFlow(page);
  await expect(page.getByRole("tab", { name: "Flow thiết bị" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await page.getByRole("tab", { name: "Điều phối" }).click();
  await expect(page.getByRole("region", { name: "Không gian Điều phối" })).toBeVisible();
  await expect(page.getByText("Chưa có điều phối nào")).toBeVisible();
  await page.getByRole("button", { name: "Tạo điều phối", exact: true }).click();
  await expect(page.getByLabel("Tên điều phối")).toHaveValue("Điều phối mới");
});

test("authors a bounded TikTok AutoSwipe node without a script surface", async ({ page }) => {
  await openFlow(page);
  await insertActionOnFirstEdge(page, "Tự động vuốt");

  await expect(page.getByLabel("Mẫu thao tác")).toHaveValue("tiktokFeed");
  await expect(page.getByLabel("Số lần vuốt")).toHaveValue("10");
  await expect(page.getByLabel("Thời lượng mỗi lần vuốt (ms)")).toHaveValue("350");
  await expect(page.getByLabel("Nghỉ tối thiểu giữa các lần vuốt (ms)")).toHaveValue("1200");
  await expect(page.getByLabel("Nghỉ tối đa giữa các lần vuốt (ms)")).toHaveValue("2500");
  await expect(page.getByLabel("Độ lệch (%)")).toHaveValue("2");

  // Both modes are finite. Switching mode removes count rather than leaving two competing
  // limits in the document, and the browser exercises the same inspector users operate.
  await page.getByRole("button", { name: "Thời lượng" }).click();
  await page.getByLabel("Tổng thời lượng (ms)").fill("60000");
  await page.getByLabel("Loại bằng chứng").selectOption("frameDigestChanged");
  await page.getByLabel("Khoảng cách tối thiểu").fill("8");

  expect(await page.getByLabel(/script|shell/i).count()).toBe(0);
  await page.getByRole("button", { name: "Kiểm tra Flow" }).click();
  const preview = page.getByRole("dialog", { name: "Xem trước biên dịch" });
  await expect(preview).toContainText("Hợp lệ");

  const calls = await mockCommandCalls(page);
  const validation = [...calls].reverse().find((call) => call.command === "flow_validate");
  expect(validation).toBeDefined();
  const document = validation!.args.document as { nodes: Array<Record<string, unknown>> };
  expect(document.nodes).toContainEqual(expect.objectContaining({
    kind: "autoSwipe",
    config: expect.objectContaining({
      preset: "tiktokFeed",
      durationMs: 60_000,
      gestureDurationMs: 350,
      pauseMinMs: 1_200,
      pauseMaxMs: 2_500,
      jitterPercent: 2,
    }),
  }));
  expect(
    document.nodes.find((node) => node.kind === "autoSwipe")?.config,
  ).not.toHaveProperty("count");

  await preview.getByRole("button", { name: "Đóng" }).click();
  const save = page.getByRole("button", { name: "Lưu bản" });
  await waitForEnabled(save);
  await save.click();

  // The persisted revision, rather than the live React state, must carry the same bounded
  // config. Reloading also exercises the fixture's command catalog: a missing AutoSwipe wire
  // command used to surface only as an operator-facing `Unknown mock command` toast.
  await page.reload();
  await page.getByRole("button", { name: "Flow", exact: true }).click();
  await expect(page.getByRole("region", { name: "Không gian Flow" })).toHaveAttribute(
    "data-loading",
    "false",
  );
  await page.locator(FLOW_NODE_TITLE).filter({ hasText: "Tự động vuốt" }).evaluate((title) => {
    title.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, composed: true }));
  });
  await expect(page.getByRole("button", { name: "Thời lượng" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByLabel("Tổng thời lượng (ms)")).toHaveValue("60000");

  const run = page.getByRole("button", { name: "Chạy Flow" });
  await waitForEnabled(run);
  await run.click();
  const runDialog = page.getByRole("dialog", { name: "Chạy Flow" });
  await runDialog.getByText("Tất cả máy hợp lệ", { exact: true }).click();
  await expect(runDialog.getByRole("radio", { name: "Tất cả máy hợp lệ" })).toBeChecked();
  await runDialog.getByRole("button", { name: "Chạy trên thiết bị" }).click();
  await expect(page.getByRole("row", { name: /Máy 1.*Tự động vuốt.*Thành công/ }))
    .toBeVisible();
  await expect(page.getByRole("row", { name: /Máy 2.*Tự động vuốt.*Thành công/ }))
    .toBeVisible();
  await expect(page.getByText(/Unknown mock command/i)).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Thông báo" })).toHaveCount(0);
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
      await expect(page.getByTestId("flow-palette")).toHaveAttribute("data-open", "false");
      await expect(page.getByRole("button", { name: "Chạy Flow" })).toBeInViewport();
      await expect(page.getByRole("button", { name: "Bật/tắt bảng thuộc tính" })).toBeInViewport();
    }

    const canvas = await page.getByTestId("flow-canvas").boundingBox();
    expect(canvas).not.toBeNull();
    expect(canvas?.width).toBeGreaterThanOrEqual(420);
    expect(await page.locator("[data-testid='flow-node']").count()).toBeGreaterThan(0);
    const inspector = await page.getByTestId("flow-inspector").boundingBox();
    expect(inspector).not.toBeNull();
    expect((inspector?.x ?? 0) + (inspector?.width ?? 0)).toBeLessThanOrEqual(viewport.width);
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

test("keeps the Flow work surface usable on a scaled laptop viewport", async ({ page }) => {
  await page.setViewportSize({ width: 820, height: 600 });
  await openFlow(page);
  await page.locator(FLOW_NODE_TITLE).filter({ hasText: "Chạm" }).click();

  const sidebar = page.locator(".aside");
  await expect(sidebar).toHaveClass(/collapsed/);
  expect((await sidebar.boundingBox())?.width).toBeLessThanOrEqual(64);
  await expect(page.getByTestId("flow-palette")).toHaveAttribute("data-open", "false");

  const canvas = await page.getByTestId("flow-canvas").boundingBox();
  const inspector = await page.getByTestId("flow-inspector").boundingBox();
  expect(canvas?.width).toBeGreaterThanOrEqual(420);
  expect((inspector?.x ?? 0) + (inspector?.width ?? 0)).toBeLessThanOrEqual(820);
  await expect(page.getByRole("button", { name: "Chạy Flow" })).toBeInViewport();
  expect(await page.locator(".content-flow").evaluate((element) =>
    getComputedStyle(element).overflowY
  )).toBe("auto");
  await expect(page).toHaveScreenshot("fixture-only-flow-820x600.png", {
    fullPage: false,
    animations: "disabled",
    maxDiffPixelRatio: 0.002,
  });
});

for (const viewport of [
  { width: 1440, height: 900 },
  { width: 900, height: 700 },
]) {
  test(`contains the FIXTURE_ONLY orchestration workspace at ${viewport.width}x${viewport.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await openFlow(page);
    await page.getByRole("tab", { name: "Điều phối" }).click();

    const workspace = page.getByRole("region", { name: "Không gian Điều phối" });
    await expect(workspace).toBeVisible();
    await expect(page.getByText("Chưa có điều phối nào")).toBeVisible();
    await page.getByRole("button", { name: "Tạo điều phối", exact: true }).click();
    await expect(page.getByLabel("Tên điều phối")).toHaveValue("Điều phối mới");
    await page.getByRole("button", { name: "Thêm Chờ" }).click();
    await expect(page.getByRole("strong").filter({ hasText: "Chờ" })).toHaveCount(1);

    await expect(page.getByText(/Unknown mock command/i)).toHaveCount(0);
    await expect(page.getByRole("region", { name: "Thông báo" })).toHaveCount(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);

    const workspaceBox = await workspace.boundingBox();
    expect(workspaceBox).not.toBeNull();
    expect(workspaceBox?.x).toBeGreaterThanOrEqual(0);
    expect((workspaceBox?.x ?? 0) + (workspaceBox?.width ?? 0)).toBeLessThanOrEqual(viewport.width);

    const libraryBox = await page.getByRole("complementary", { name: "Danh sách điều phối" }).boundingBox();
    const editor = page.locator(".orchestration-editor");
    const editorBox = await editor.boundingBox();
    expect(libraryBox).not.toBeNull();
    expect(editorBox).not.toBeNull();
    expect(overlaps(libraryBox!, editorBox!)).toBe(false);
    expect(await editor.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    await expect(page.getByRole("button", { name: "Chạy điều phối" })).toBeInViewport();
    await expect(page.getByRole("button", { name: "Lưu bản" })).toBeInViewport();
    expect(await workspace.locator("button:visible, input:visible").evaluateAll((elements) =>
      elements.every((element) => element.scrollWidth <= element.clientWidth)
    )).toBe(true);

    await expect(page).toHaveScreenshot(
      `fixture-only-orchestration-${viewport.width}x${viewport.height}.png`,
      {
        fullPage: true,
        animations: "disabled",
        maxDiffPixelRatio: 0.002,
      },
    );
  });
}
