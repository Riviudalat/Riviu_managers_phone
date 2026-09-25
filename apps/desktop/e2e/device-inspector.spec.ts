import {test,expect} from '@playwright/test';
import {installTauriMock,mockCommandCalls} from './fixtures/tauriMock';
test('inspector selects without tapping, records outcomes and saves a semantic Flow',async({page})=>{
 await installTauriMock(page,{androidRoster:true,fleetSize:2});
 await page.addInitScript(()=>{
  const w=window as unknown as {__TAURI_INTERNALS__:{invoke:(c:string,a?:Record<string,unknown>)=>Promise<unknown>};__INSPECTOR_SAVED__:unknown};const original=w.__TAURI_INTERNALS__.invoke;let recording:Record<string,unknown>|null=null;
  const selector={package:'app.test',description:'Profile'};
  const snapshot={id:'before',udid:'MOCK-FLEET-1',package:'app.test',version:'1.0',locale:'en',width:360,height:720,pngBase64:'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jXioAAAAASUVORK5CYII=',treeSha256:'one',hierarchyXml:'<hierarchy><node text="Profile"/></hierarchy>',elements:[{index:0,parent:null,text:'',description:'',resourceId:'',className:'FrameLayout',x:0,y:0,width:360,height:720,enabled:true,clickable:false,selector:null},{index:1,parent:0,text:'Profile',description:'Profile',resourceId:'app.test:id/profile',className:'Button',x:0,y:600,width:100,height:60,enabled:true,clickable:true,checkable:false,checked:null,selector}]};
  w.__TAURI_INTERNALS__.invoke=async(c,a={})=>{
   if(['device_control_begin','device_control_end'].includes(c))return null;
   if(c==='inspector_observe')return snapshot;
   if(c==='inspector_recording')return recording;
   if(c==='inspector_record'){recording=a.active?{id:'record',udid:a.udid,name:a.name,active:true,steps:[]}:{...recording,active:false};return recording;}
   if(c==='inspector_tap'){recording={...recording,steps:[{selector,expected:{package:'app.test',text:'Edit profile'},beforeId:'before',afterId:'after',verified:true,error:null}]};return {...snapshot,id:'after',treeSha256:'two'};}
   if(c==='flow_save_revision'){w.__INSPECTOR_SAVED__=a.document;return {document:{...a.document as object,revision:1}};}
   return original(c,a);
  };
 });
 await page.goto('/');await page.getByTestId('device-tile').first().dblclick();
 await page.getByRole('button',{name:'Bắt thuộc tính & ghi Flow',exact:true}).click();
 const inspector=page.getByRole('dialog',{name:'Bắt thuộc tính và ghi Flow',exact:true});
 await expect(inspector.getByRole('img',{name:'Màn hình thiết bị'})).toBeVisible();
 await expect(inspector.getByRole('table',{name:'Thuộc tính phần tử'})).toBeVisible();
 await expect(inspector.getByRole('tree',{name:'Cây phần tử'})).toBeVisible();
 await inspector.getByRole('treeitem',{name:'Profile Có thể bấm',exact:true}).click();
 await expect(inspector.getByText('Tìm đúng một phần tử bằng thuộc tính')).toBeVisible();
 await expect(inspector.getByRole('row',{name:'checkable false'})).toBeVisible();
 await expect(inspector.getByRole('row',{name:'checked —'})).toBeVisible();
 expect((await mockCommandCalls(page)).filter(c=>c.command==='device_tap')).toHaveLength(0);
 await inspector.getByRole('button',{name:'Bắt đầu ghi',exact:true}).click();
 await inspector.getByRole('button',{name:'Bấm và kiểm tra kết quả',exact:true}).click();
 await expect(inspector.locator('footer')).toContainText('1 bước đã ghi');
 await inspector.getByRole('button',{name:'Dừng ghi',exact:true}).click();
 await inspector.getByRole('button',{name:'Lưu thành Flow',exact:true}).click();
 await expect(inspector.getByRole('status')).toContainText('Đã lưu Flow');
 const flow=await page.evaluate(()=>(window as unknown as {__INSPECTOR_SAVED__:{nodes:{kind:string;config:Record<string,unknown>}[]}}).__INSPECTOR_SAVED__);
 expect(flow.nodes.map(n=>n.kind)).toEqual(['start','launchApp','tap','end']);
 expect(flow.nodes[2].config).toEqual({selector:{package:'app.test',description:'Profile'}});
 await page.screenshot({path:test.info().outputPath('inspector.png'),animations:'disabled'});
});

for (const viewport of [{ width: 1440, height: 900 }, { width: 820, height: 560 }]) {
 test(`inspector panes fit ${viewport.width}x${viewport.height} without overlapping`, async ({ page }) => {
  await page.setViewportSize(viewport);
  await installTauriMock(page, { androidRoster: true, fleetSize: 2 });
  await page.addInitScript(() => {
   const w = window as unknown as { __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> } };
   const original = w.__TAURI_INTERNALS__.invoke;
   w.__TAURI_INTERNALS__.invoke = async (command, args) => {
    if (command === 'inspector_observe') return { id: 'pane-snapshot', udid: 'MOCK-FLEET-1', package: 'app.test', version: '1.0', locale: 'en', width: 360, height: 720, pngBase64: 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jXioAAAAASUVORK5CYII=', treeSha256: 'one', hierarchyXml: '<hierarchy><node text="Profile"/></hierarchy>', elements: [{ index: 0, parent: null, text: '', description: '', resourceId: '', className: 'FrameLayout', x: 0, y: 0, width: 360, height: 720, enabled: true, clickable: false, selector: null }, { index: 1, parent: 0, text: 'Profile', description: 'Profile', resourceId: 'app.test:id/profile', className: 'Button', x: 0, y: 600, width: 100, height: 60, enabled: true, clickable: true, selector: { package: 'app.test', description: 'Profile' } }] };
    if (command === 'inspector_recording') return null;
    return original(command, args);
   };
  });
  await page.goto('/');
  await page.getByTestId('device-tile').first().dblclick();
  await page.getByRole('button', { name: 'Bắt thuộc tính & ghi Flow', exact: true }).click();
  const inspector = page.getByRole('dialog', { name: 'Bắt thuộc tính và ghi Flow', exact: true });
  const image = inspector.getByRole('img', { name: 'Màn hình thiết bị' });
  const table = inspector.getByRole('table', { name: 'Thuộc tính phần tử' });
  const tree = inspector.getByRole('tree', { name: 'Cây phần tử' });
  await expect(image).toBeVisible();
  await expect(table).toBeVisible();
  await expect(tree.getByRole('treeitem', { name: /Profile/ })).toBeVisible();
  const bounds = await Promise.all([image.boundingBox(), table.boundingBox(), tree.boundingBox()]);
  expect(bounds.every(Boolean)).toBe(true);
  const [left, middle, right] = bounds as { x: number; y: number; width: number; height: number }[];
  expect(left.x + left.width).toBeLessThanOrEqual(middle.x + 2);
  if (viewport.width > 1020) expect(middle.x + middle.width).toBeLessThanOrEqual(right.x + 2);
  else expect(middle.y + middle.height).toBeLessThanOrEqual(right.y + 2);
  const child = await tree.getByRole('treeitem', { name: /Profile/ }).boundingBox();
  expect(child).not.toBeNull();
  expect(child!.y + child!.height).toBeLessThanOrEqual(right.y + right.height + 1);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: test.info().outputPath(`inspector-${viewport.width}.png`), animations: 'disabled' });
 });
}
