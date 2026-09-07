import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";

for (const viewport of [
  { width: 1440, height: 900 },
  { width: 820, height: 560 },
]) {
  test(`publish source keeps picked paths and ignores stale scans at ${viewport.width}`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.setViewportSize(viewport);
    await installTauriMock(page, { androidRoster: true, fleetSize: 20 });
    await page.addInitScript(() => {
      const w = window as unknown as {
        __TAURI_INTERNALS__: {
          invoke: (
            command: string,
            args: Record<string, unknown>,
          ) => Promise<unknown>;
        };
        __PUBLISH_SOURCE_TEST__: {
          release: (() => void) | null;
          calls: string[];
          unknown: string[];
          effects: string[];
        };
      };
      const invoke = w.__TAURI_INTERNALS__.invoke;
      w.__PUBLISH_SOURCE_TEST__ = {
        release: null,
        calls: [],
        unknown: [],
        effects: [],
      };
      const manifest = (root: string, count: number) => ({
        sourceRoot: root,
        scannedAt: new Date().toISOString(),
        notices: [],
        ignoredPartnerFiles: 0,
        ignoredHiddenFiles: 0,
        bundles: Array.from({ length: count }, (_, index) => ({
          id: `bundle-${index + 1}`,
          name: count === 1 ? "Bài riêng" : `Bộ ảnh ${index + 1}`,
          sourcePath: count === 1 ? root : `${root}/Bộ ảnh ${index + 1}`,
          mediaKind: "image",
          images: [],
          captionPath: `${root}/caption-${index + 1}.txt`,
          caption: `Chú thích bài ${index + 1}`,
          captionSha256: "aa".repeat(32),
          totalBytes: 1024,
        })),
      });
      w.__TAURI_INTERNALS__.invoke = async (command, args) => {
        if (command === "startup_error") return null;
        if (command === "android_tool_problems") return [];
        if (command === "plugin:dialog|open") return "C:/Nội dung/thư mục rỗng";
        if (command === "publish_scan_folder") {
          const root = String(args.sourceRoot);
          w.__PUBLISH_SOURCE_TEST__.calls.push(root);
          if (root.endsWith("thư mục rỗng"))
            throw {
              code: "OperationFailed",
              message: "publish folder has no bundle directories",
            };
          if (root.endsWith("quét chậm"))
            await new Promise<void>((resolve) => {
              w.__PUBLISH_SOURCE_TEST__.release = resolve;
            });
          return manifest(root, root.endsWith("21 bài") ? 21 : 1);
        }
        if (
          [
            "publish_create_campaign",
            "publish_execute",
            "publish_preflight",
          ].includes(command)
        ) {
          w.__PUBLISH_SOURCE_TEST__.effects.push(command);
          throw new Error("source fixture must not dispatch");
        }
        try {
          return await invoke(command, args);
        } catch (error) {
          if (String(error).includes("Unknown mock command"))
            w.__PUBLISH_SOURCE_TEST__.unknown.push(command);
          throw error;
        }
      };
    });
    await page.goto("/");
    await expect(page.getByTestId("device-tile")).toHaveCount(20);
    await page.getByRole("button", { name: "Đăng bài", exact: true }).click();
    await page
      .getByRole("button", { name: "Chọn thư mục", exact: true })
      .click();
    const source = page.getByRole("textbox", { name: "Thư mục nguồn" });
    await expect(source).toHaveValue("C:/Nội dung/thư mục rỗng");
    await expect(
      page.getByText("Chưa tìm thấy gói bài trong thư mục đã chọn", {
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      page.getByText("publish folder has no bundle directories", {
        exact: true,
      }),
    ).not.toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: test
        .info()
        .outputPath(`publish-source-error-${viewport.width}.png`),
    });

    await source.fill("C:/Nội dung/quét chậm");
    await page.getByRole("button", { name: "Quét nguồn", exact: true }).click();
    await expect(
      page.getByText("Đang quét nội dung…", { exact: true }),
    ).toBeVisible();
    await source.fill("C:/Nội dung/Bài riêng");
    await page.evaluate(() =>
      (
        window as unknown as {
          __PUBLISH_SOURCE_TEST__: { release: () => void };
        }
      ).__PUBLISH_SOURCE_TEST__.release(),
    );
    await expect(source).toHaveValue("C:/Nội dung/Bài riêng");
    await expect(
      page.getByRole("checkbox", { name: "Chọn Bài riêng", exact: true }),
    ).toHaveCount(0);
    await page.getByRole("button", { name: "Quét nguồn", exact: true }).click();
    await expect(
      page.getByRole("checkbox", { name: "Chọn Bài riêng", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("checkbox", { name: "Chọn Bài riêng", exact: true }),
    ).not.toBeChecked();
    await expect(
      page.getByRole("button", { name: "Chọn máy", exact: true }),
    ).toBeDisabled();

    await source.fill("C:/Nội dung/21 bài");
    await page.getByRole("button", { name: "Quét nguồn", exact: true }).click();
    const bundles = page.getByRole("region", { name: "Chọn bài đăng" });
    await expect(
      bundles.getByRole("checkbox", { name: "Chọn Bộ ảnh 1", exact: true }),
    ).toBeVisible();
    const seen = new Set<string>();
    do {
      const boxes = bundles.getByRole("checkbox", { name: /^Chọn Bộ ảnh/ });
      for (const box of await boxes.all()) {
        await expect(box).not.toBeChecked();
        seen.add((await box.getAttribute("aria-label"))!);
      }
      const next = page.getByRole("button", { name: "Bài đăng: trang tiếp" });
      if (await next.isDisabled()) break;
      await next.click();
    } while (seen.size <= 21);
    expect(seen.size).toBe(21);
    await expect(
      page.getByRole("button", { name: "Chọn máy", exact: true }),
    ).toBeDisabled();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: test.info().outputPath(`publish-source-21-${viewport.width}.png`),
    });
    await expect(page.getByRole("alert")).toHaveCount(0);
    await expect(page.getByText(/Unknown mock command/)).toHaveCount(0);
    expect(errors).toEqual([]);
    expect(
      await page.evaluate(() => {
        const { calls, unknown, effects } = (
          window as unknown as {
            __PUBLISH_SOURCE_TEST__: {
              calls: string[];
              unknown: string[];
              effects: string[];
            };
          }
        ).__PUBLISH_SOURCE_TEST__;
        return { calls, unknown, effects };
      }),
    ).toEqual({
      calls: [
        "C:/Nội dung/thư mục rỗng",
        "C:/Nội dung/quét chậm",
        "C:/Nội dung/Bài riêng",
        "C:/Nội dung/21 bài",
      ],
      unknown: [],
      effects: [],
    });
  });
}
