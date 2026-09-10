import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";

test("submitted posts show incomplete progress and processing evidence without a success claim", async ({ page }) => {
  await installTauriMock(page);
  await page.addInitScript(() => {
    const w = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> } };
    const original = w.__TAURI_INTERNALS__.invoke;
    const now = new Date().toISOString();
    const summary = { id: "publish:pending", sourceId: "pending", kind: "publish", title: "Đăng bài", state: "running", targetCount: 2, totalItems: 2, completedItems: 1, issueCount: 0, retryableCount: 1, retryScope: "linkAndSheet", createdAt: now, updatedAt: now };
    w.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "publish_list") return [{ id: "pending", requestId: "request", sourceRoot: "C:/fixture", state: "verifying", visibility: "public", cleanupPolicy: "keepImportedAssets", assignments: [{ bundleId: "pending", udid: "snapshot-phone-2", ordinal: 0 }], createdAt: now, updatedAt: now }];
      if (command === "operation_list_runs") return [summary];
      if (command === "operation_query_runs") return { runs: [summary], total: 1, counts: { active: 1, succeeded: 0, attention: 0 }, hasMore: false };
      if (command === "operation_get_run") return { summary, items: [
        { id: "confirmed", kind: "assignment", label: "Máy 1", state: "succeeded", udid: "snapshot-phone-1", errorCode: null, detail: null, evidence: null, retryable: false },
        { id: "pending", kind: "assignment", label: "Máy 2", state: "running", udid: "snapshot-phone-2", errorCode: "post_verification_pending", detail: null, evidence: null, retryable: false },
      ], batch: null };
      if (command === "operation_device_log") return { entries: [
        { id: "1", at: now, action: "publishStep", state: "post_submitted", text: "[12] Đã bấm Đăng; chờ TikTok xử lý", detail: null },
        { id: "2", at: now, action: "publish", state: "verifying", text: null, detail: null },
      ], truncated: false };
      return original(command, args);
    };
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Mở rộng tiến trình", exact: true }).click();
  const panel = page.getByRole("dialog", { name: "Cửa sổ tiến trình" });
  await expect(panel.getByRole("progressbar", { name: "Tiến độ công việc" })).toHaveAttribute("aria-valuenow", "50");
  await expect(panel.getByLabel("Kết quả từng máy")).toContainText("1 hoàn tất");
  await expect(panel.getByLabel("Kết quả từng máy")).toContainText("1 đang chờ/chạy");
  await panel.getByRole("button", { name: /Máy 2 Chờ xác minh bài đăng/ }).click();
  await expect(panel.getByRole("list", { name: "Nhật ký theo thời gian" })).toContainText("Đã bấm Đăng — chờ TikTok hoàn tất và xác minh liên kết bài");
  await expect(panel).not.toContainText("100%");
  await expect(panel).not.toContainText("Thành công");
  await panel.screenshot({ path: "../../target/stable-installer-20260909/frontend/publish-pending.png" });
  await panel.getByRole("button", { name: "Thu nhỏ tiến trình", exact: true }).click();
  await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
  await page.getByRole("tab", { name: "Theo dõi", exact: true }).click();
  await page.getByRole("button", { name: /^Chiến dịch 1 / }).click();
  await expect(page.getByRole("button", { name: "Kiểm tra liên kết", exact: true })).toBeEnabled();
  await expect(page.getByRole("button", { name: "Chạy lại từ đầu", exact: true })).toHaveCount(0);
});

for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
  test(`publish timeline keeps numbered steps and seconds at ${viewport.width}px`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await installTauriMock(page);
    await page.addInitScript(() => {
      const w = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> } };
      const original = w.__TAURI_INTERNALS__.invoke;
      const now = new Date().toISOString();
      const summary = { id: "publish:steps", sourceId: "steps", kind: "publish", title: "Đăng bài", state: "succeeded", targetCount: 1, totalItems: 1, completedItems: 1, issueCount: 0, retryableCount: 0, retryScope: "none", createdAt: now, updatedAt: now };
      const messages = ["Kiểm tra kết nối và khả năng đăng bài của máy 2", "Đã xác nhận máy 2 sẵn sàng", "Đang tải 6 ảnh vào điện thoại", "Đã tải và xác nhận nội dung trong thư viện điện thoại", "Đang mở TikTok và chờ màn hình sẵn sàng", "Bấm dấu + để tạo bài đăng", "Mở thư viện ảnh/video", "Đã xác nhận chọn đủ 6 ảnh", "Đã xác nhận nhạc: Đến Khi Nào", "Đã đọc lại và xác nhận nội dung chữ", "Bấm Đăng bài", "Thành công — máy 2 đã đăng bài"];
      const entries = messages.map((message, index) => ({ id: String(index + 1), at: `2026-09-08T07:03:${String(index).padStart(2, "0")}`, action: "publishStep", state: index === messages.length - 1 ? "finished" : "working", text: `[${index + 1}] ${message}`, detail: null }));
      w.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (command === "operation_query_runs") return { runs: [summary], total: 1, counts: { active: 0, succeeded: 1, attention: 0 }, hasMore: false };
        if (command === "operation_get_run") return { summary, items: [{ id: "assignment", kind: "assignment", label: "Máy 2", state: "succeeded", udid: "snapshot-phone-2", errorCode: null, detail: null, evidence: null, retryable: false }], batch: null };
        if (command === "operation_device_log") return { entries, truncated: false };
        return original(command, args);
      };
    });
    await page.goto("/");
    await page.getByRole("button", { name: "Mở rộng tiến trình", exact: true }).click();
    await page.getByRole("button", { name: /Máy 2 Hoàn tất/ }).click();
    const log = page.getByRole("list", { name: "Nhật ký theo thời gian" });
    await expect(log.getByRole("listitem").first()).toContainText("[12] Thành công — máy 2 đã đăng bài");
    await expect(log.getByRole("listitem").first()).toContainText("07:03:11");
    await page.getByRole("button", { name: "Mới nhất trước" }).click();
    await expect(log.getByRole("listitem").first()).toContainText("[1] Kiểm tra kết nối");
    await expect(log.getByRole("listitem").first()).toContainText("07:03:00");
    const panel = page.getByRole("dialog", { name: "Cửa sổ tiến trình" });
    const bounds = await panel.boundingBox();
    expect(bounds).not.toBeNull();
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(viewport.width + 1);
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(viewport.height + 1);
    await panel.screenshot({ path: `../../output/playwright/publish-log-${viewport.width}.png` });
  });
}


test("visible monitor refreshes assignment evidence without replaying a command", async ({ page }) => {
  await installTauriMock(page);
  await page.addInitScript(() => {
    const w = window as unknown as { __TAURI_INTERNALS__: {invoke:(command:string,args:Record<string,unknown>)=>Promise<unknown>}; changed:boolean; effects:number };
    const original = w.__TAURI_INTERNALS__.invoke;
    const campaign = { id:"polling",requestId:"r",sourceRoot:"C:/fixture",state:"verifying",assignments:[],createdAt:new Date().toISOString() };
    w.changed=false;w.effects=0;
    w.__TAURI_INTERNALS__.invoke = async(command,args) => {
      if(command==="publish_list")return [campaign];
      if(command==="publish_reconcile")return {campaignId:"polling",status:"partial",retryScope:"linkAndSheet",reportJson:{sheetEnabled:true}};
      if(command==="publish_get")return {campaign,bundles:[],events:[],assignments:[{id:"a",campaignId:"polling",bundleId:"b",ordinal:0,udid:"snapshot-phone-1",state:"verifying",errorCode:"post_verification_pending",evidenceJson:JSON.stringify({verificationStatus:{state:"pending",reason:w.changed?"Đã thấy bài, đang đọc liên kết":"Bài đang tải",checkedAt:"2026-09-10T00:00:00Z",nextCheckAt:"2026-09-10T00:00:30Z"}})}]};
      if(command==="publish_execute"||command==="publish_create_campaign"){w.effects++;throw new Error("poll cannot dispatch");}
      return original(command,args);
    };
  });
  await page.goto("/");
  await page.getByRole("button",{name:"Đăng bài",exact:true}).click();
  await page.getByRole("tab",{name:"Theo dõi",exact:true}).click();
  await page.getByRole("button",{name:"Chi tiết máy",exact:true}).click();
  await expect(page.getByText("Bài đang tải",{exact:false})).toBeVisible();
  await page.evaluate(()=>{(window as unknown as {changed:boolean}).changed=true;});
  await expect(page.getByText("Đã thấy bài, đang đọc liên kết",{exact:false})).toBeVisible({timeout:10000});
  expect(await page.evaluate(()=>(window as unknown as {effects:number}).effects)).toBe(0);
});
