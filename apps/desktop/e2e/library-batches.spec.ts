import { expect, test } from "@playwright/test";
import { installTauriMock, mockCommandCalls } from "./fixtures/tauriMock";

for (const surface of [
  { name:"Trung tâm ứng dụng", command:"install_library_app_batch", button:/Cài → 2 Android/ },
  { name:"Kho nội dung", command:"push_material_batch", button:/Chuyển tới 2 máy/ },
]) {
  test(`${surface.name} reattaches to a running batch across navigation and reload`, async ({ page }) => {
    await installTauriMock(page,{androidRoster:true,libraryBatchRunning:true});
    await page.goto("/");
    await expect(page.getByTestId("device-tile")).toHaveCount(2);
    await page.getByRole("button",{name:surface.name,exact:true}).click();
    await page.getByRole("button",{name:surface.button}).click();
    await page.getByRole("button",{name:"Dữ liệu",exact:true}).click();
    await expect(page.getByText("Tác vụ trong 24 giờ qua")).toBeVisible();
    await page.getByRole("button",{name:surface.name,exact:true}).click();
    await expect(page.getByRole("table",{name:"Tiến độ batch đã lưu"})).toBeVisible();
    await expect(page.getByText("Máy 2 · Kệ 2",{exact:true})).toBeVisible();
    await expect(page.getByRole("button",{name:surface.button})).toBeDisabled();
    expect((await mockCommandCalls(page)).filter((call) => call.command === surface.command)).toHaveLength(1);
    await page.reload();
    await expect(page.getByTestId("device-tile")).toHaveCount(2);
    await page.getByRole("button",{name:surface.name,exact:true}).click();
    await expect(page.getByText("Máy 1 · Kệ 1",{exact:true})).toBeVisible();
    await expect(page.getByRole("button",{name:surface.button})).toBeDisabled();
    await page.getByRole("button",{name:"Dừng máy đang chờ"}).click();
    await expect(page.getByText("Đã dừng trước khi chạy",{exact:true})).toBeVisible();
    await expect(page.getByText("Đang thực hiện",{exact:true})).toBeVisible();
    expect((await mockCommandCalls(page)).filter((call) => call.command === surface.command)).toHaveLength(0);
    await expect(page.getByText(/Unknown mock command/)).toHaveCount(0);
  });
}
