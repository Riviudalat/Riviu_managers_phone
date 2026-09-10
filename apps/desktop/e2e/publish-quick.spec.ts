import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";
test("approved quick desk scans, selects, assigns and checks without public dispatch", async ({page})=>{
  await installTauriMock(page,{androidRoster:true,fleetSize:20});
    await page.addInitScript(() => {
      const w = window as unknown as {
        __TAURI_INTERNALS__: {
          invoke: (
            command: string,
            args: Record<string, unknown>,
          ) => Promise<unknown>;
        };
        __PUBLISH_CALLS__: { command: string; args: Record<string, unknown> }[];
      };
      const invoke = w.__TAURI_INTERNALS__.invoke;
      w.__PUBLISH_CALLS__ = [];
      w.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (command === "startup_error") return null;
        if (command === "android_tool_problems") return [];
        if (command === "operation_query_runs") return {
          total: 1, offset: 0, limit: 200,
          counts: { active: 0, succeeded: 1, attention: 0 }, hasMore: false,
          runs: [{ id: "publish:finished", sourceId: "finished", kind: "publish", title: "Đăng bài", state: "succeeded", targetCount: 1, totalItems: 1, completedItems: 1, issueCount: 0, retryableCount: 0, retryScope: "none", createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() }],
        };
        if (command === "plugin:dialog|open") return "C:/Bài đăng";
        if (command === "publish_scan_folder")
          return {
            sourceRoot: args.sourceRoot,
            scannedAt: "2026-09-07T00:00:00Z",
            notices: [],
            ignoredPartnerFiles: 0,
            ignoredHiddenFiles: 0,
            bundles: Array.from({ length: 10 }, (_, i) => ({
              id: `bundle-${i + 1}`,
              name: `Bài Đà Lạt ${i + 1}`,
              sourcePath: `C:/Bài đăng/Bài ${i + 1}`,
              mediaKind: "image",
              images: Array.from({ length: 5 }, (_, j) => ({
                path: `C:/Bài đăng/Bài ${i + 1}/${j + 1}.png`,
                fileName: `${j + 1}.png`,
                order: j,
                sha256: "a".repeat(64),
                byteLen: 100,
                width: 100,
                height: 100,
              })),
              captionPath: "caption.txt",
              caption: `Nội dung bài ${i + 1}`,
              captionSha256: "b".repeat(64),
              totalBytes: 500,
            })),
          };
        if (command === "publish_image_preview")
          return (
            "data:image/svg+xml," +
            encodeURIComponent(
              '<svg xmlns="http://www.w3.org/2000/svg" width="120" height="160"><rect width="120" height="160" fill="#b9d8c1"/><path d="M0 130 40 45 65 90 87 55 120 130" fill="#608771"/><text x="14" y="22" font-size="14">Đà Lạt</text></svg>',
            )
          );
        if (command === "publish_preflight") {
          w.__PUBLISH_CALLS__.push({ command, args });
          const r = args.request as {
            bundleIds: string[];
            udids: string[];
            targetRef: unknown;
          };
          return {
            inputDigest: "digest",
            sheetEnabled: false,
            sheetConfigured: false,
            canExecute: true,
            targetSnapshot: {
              targetRef: r.targetRef,
              included: r.udids.map((udid) => ({ udid, alias: "" })),
              excluded: [],
              rosterSha256: "c".repeat(64),
            },
            assignments: r.bundleIds.map((id, i) => ({
              ordinal: i,
              bundleId: id,
              udid: r.udids[i],
              media: "pass",
              composer: "pass",
              soundPicker: "pass",
              storage: "pass",
              requiredBytes: 100,
              availableBytes: 99999,
              issues: [],
            })),
            issues: [],
          };
        }
        if (
          command === "publish_create_campaign" ||
          command === "publish_execute"
        ) {
          w.__PUBLISH_CALLS__.push({ command, args });
          throw new Error("No public action in layout gate");
        }
        return invoke(command, args);
      };
    });

  await page.goto("/");
  await page.getByRole("button",{name:"Đăng bài",exact:true}).click();
  await page.getByRole("combobox",{name:"Phạm vi thiết bị"}).selectOption("all");
  await page.getByRole("button",{name:"Chọn thư mục",exact:true}).click();
  await page.getByRole("button",{name:"Quét",exact:true}).click();
  await page.getByRole("button",{name:"Chọn nhanh",exact:true}).click();
  await expect(page.locator(".pq-footer")).toContainText("10/10 bài có máy");
  await page.getByRole("button", {name:"Hoàn tác gán nhanh"}).click();
  await expect(page.locator(".pq-footer")).toContainText("0 bài đã chọn");
  await page.getByRole("button",{name:"Chọn nhanh",exact:true}).click();
  await page.getByRole("button",{name:"Chọn nhanh",exact:true}).click();
  await expect(page.getByRole("button",{name:"Kiểm tra & đăng",exact:true})).toBeEnabled();
  for(const size of [{width:1440,height:900},{width:820,height:560}]){
    await page.setViewportSize(size);
    await page.screenshot({path:test.info().outputPath(`quick-${size.width}.png`)});
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  }
  await page.getByRole("button",{name:"Kiểm tra & đăng",exact:true}).click();
  await expect(page.getByRole("button",{name:"Xác nhận đăng 10 bài",exact:true})).toBeEnabled();
  await expect(page.getByRole("dialog", {name:"Kiểm tra đợt đăng"})).toContainText("Ghi Sheet đang tắt");
});
