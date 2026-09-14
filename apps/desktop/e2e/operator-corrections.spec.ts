import {test,expect} from "@playwright/test";
import {installTauriMock} from "./fixtures/tauriMock";

for(const width of [1440,820])test(`orange operator layout and run table at ${width}`,async({page})=>{
 await page.setViewportSize({width,height:900});await installTauriMock(page,{androidRoster:true,fleetSize:20});
 await page.goto('/');await expect(page.getByTestId('device-tile')).toHaveCount(20);
 const nav=page.getByRole('navigation',{name:'Điều hướng chính'});
 await expect(nav.getByRole('button',{name:'Dữ liệu',exact:true})).toHaveCount(0);
 await expect(nav.getByRole('button',{name:'Mạng & Router',exact:true})).toHaveCount(0);
 await expect(page.locator('.device-automation-bar')).toHaveCount(0);
 expect(await page.evaluate(()=>getComputedStyle(document.documentElement).getPropertyValue('--primary').trim())).toBe('#c2410c');
 await page.screenshot({path:test.info().outputPath(`control-orange-${width}.png`)});
 await nav.getByRole('button',{name:'Lượt chạy',exact:true}).click();
 await expect(page.getByRole('heading',{name:'Lượt chạy',exact:true})).toBeVisible();
 await expect(page.getByRole('table',{name:'Danh sách lượt chạy'})).toBeVisible();
 await expect(page.locator('.operations-summary')).toHaveCount(0);
 await expect(page.getByRole('dialog',{name:'Chi tiết lượt chạy'})).toHaveCount(0);
 await expect(page.getByText('Chưa có tác vụ',{exact:true})).toBeVisible();
 await page.screenshot({path:test.info().outputPath(`runs-orange-${width}.png`)});
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
});

test('runs remain a table until a row is chosen and retain exact source navigation',async({page})=>{
 await installTauriMock(page);
 await page.addInitScript(()=>{
  const w=window as unknown as {__TAURI_INTERNALS__:{invoke:(c:string,a?:Record<string,unknown>)=>Promise<unknown>}};const invoke=w.__TAURI_INTERNALS__.invoke;
  const summary={id:'nurture:fixture',sourceId:'fixture',kind:'nurture',title:'Lượt kiểm tra',state:'succeeded',targetCount:2,totalItems:2,completedItems:2,issueCount:0,retryableCount:0,retryScope:null,createdAt:'2026-09-14T01:00:00Z',updatedAt:'2026-09-14T01:05:00Z'};
  w.__TAURI_INTERNALS__.invoke=async(c,a={})=>c==='operation_query_runs'?{runs:[summary],total:1,counts:{active:0,succeeded:1,attention:0},hasMore:false}:c==='operation_get_run'?{summary,items:[]}:invoke(c,a);
 });
 await page.goto('/');await page.getByRole('button',{name:'Lượt chạy',exact:true}).click();
 await expect(page.getByRole('table',{name:'Danh sách lượt chạy'}).getByText('Lượt kiểm tra',{exact:true})).toBeVisible();
 await expect(page.getByRole('dialog',{name:'Chi tiết lượt chạy'})).toHaveCount(0);
 await page.getByRole('button',{name:'Xem lượt chạy Lượt kiểm tra',exact:true}).click();
 const drawer=page.getByRole('dialog',{name:'Chi tiết lượt chạy'});await expect(drawer.getByRole('button',{name:'Mở tại Nuôi TikTok'})).toBeVisible();
 await page.keyboard.press('Escape');await expect(drawer).toHaveCount(0);
});
