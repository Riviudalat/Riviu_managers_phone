import { expect, test } from "@playwright/test";
import { installTauriMock, mockCommandCalls } from "./fixtures/tauriMock";

for (const width of [1440, 820]) {
  test(`non-modal phone windows and direct stream settings at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await installTauriMock(page, { androidRoster: true, fleetSize: 4 });
    await page.addInitScript(() => {
      const w=window as unknown as {__TAURI_INTERNALS__:{invoke:(command:string,args?:Record<string,unknown>)=>Promise<unknown>}};
      const invoke=w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke=(command,args)=>command==='device_control_begin'||command==='device_control_end'?Promise.resolve(null):invoke(command,args);
    });
    await page.goto('/');
    await expect(page.getByTestId('device-tile')).toHaveCount(4);
    await page.getByRole('button',{name:'Hiện bảng Hiển thị',exact:true}).hover();
    await page.getByRole('slider',{name:'FPS màn hình Android'}).fill('18');
    await page.getByRole('slider',{name:'FPS màn hình Android'}).press('ArrowRight');
    await expect.poll(async()=> (await mockCommandCalls(page)).some(call=>call.command==='set_stream_settings'&&(call.args.settings as {fps:number}).fps===19)).toBe(true);
    await page.keyboard.press('Escape');
    await page.mouse.move(width-10, 10);
    await page.getByTestId('device-tile').first().dblclick();
    const first=page.getByRole('dialog',{name:'Điều khiển Máy thử 1',exact:true});
    await expect(first).toHaveAttribute('aria-modal','false');
    await page.getByRole('button',{name:'My Apps',exact:true}).click();
    await expect(first).toBeVisible();
    await page.getByRole('button',{name:'Control Center',exact:true}).click();
    // Changing device reuses the existing window, including when it covers the tile.
    await page.getByTestId('device-tile').nth(1).dispatchEvent('dblclick');
    await expect(page.locator('.device-floating-window')).toHaveCount(1);
    await expect(first).toHaveCount(0);
    await expect(page.getByRole('dialog',{name:'Điều khiển Máy thử 2',exact:true})).toBeVisible();
    await page.screenshot({path:test.info().outputPath(`control-${width}.png`)});
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
  });
}

test('account records persist, edit with revisions, and do not execute device actions on save',async({page})=>{
 await installTauriMock(page);
 await page.addInitScript(()=>{
   const w=window as unknown as {__TAURI_INTERNALS__:{invoke:(command:string,args?:Record<string,unknown>)=>Promise<unknown>}};const invoke=w.__TAURI_INTERNALS__.invoke;let rows:Record<string,unknown>[]=[];
   w.__TAURI_INTERNALS__.invoke=async(command,args={})=>{
    if(command==='operator_list')return structuredClone(rows);
    if(command==='operator_save'){const input=args.input as Record<string,unknown>;const existing=rows.find(r=>r.id===input.id);if((existing?.revision??null)!==input.expectedRevision)throw new Error('revision conflict');const record={...input,revision:Number(existing?.revision??0)+1,archived:false,createdAt:'now',updatedAt:'now'};rows=[...rows.filter(r=>r.id!==input.id),record];return structuredClone(record);}
    return invoke(command,args);
   };
 });
 await page.goto('/');await page.getByRole('button',{name:'Quản lý tài khoản',exact:true}).click();
 await page.getByRole('button',{name:'Thêm tài khoản',exact:true}).click();
 const editor=page.getByRole('complementary',{name:'Chỉnh bản ghi'});
 await editor.getByLabel('Tên',{exact:true}).fill('Đội nội dung');
 await editor.getByLabel('Tài khoản',{exact:true}).fill('example.account');
 await editor.getByRole('button',{name:'Lưu bản ghi',exact:true}).click();
 await expect(page.getByRole('cell',{name:'example.account',exact:true})).toBeVisible();
 await page.getByRole('button',{name:'Chỉnh sửa',exact:true}).click();
 await editor.getByLabel('Tên',{exact:true}).fill('Đội đã chỉnh');
 await editor.getByRole('button',{name:'Lưu bản ghi',exact:true}).click();
 await expect(page.getByRole('cell',{name:'Đội đã chỉnh',exact:true})).toBeVisible();
 expect((await mockCommandCalls(page)).filter(call=>call.command==='interaction_read_account'||call.command==='device_shell')).toHaveLength(0);
});
