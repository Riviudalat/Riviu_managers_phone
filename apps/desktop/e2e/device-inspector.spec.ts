import {test,expect} from '@playwright/test';
import {installTauriMock,mockCommandCalls} from './fixtures/tauriMock';
test('inspector selects without tapping, records outcomes and saves a semantic Flow',async({page})=>{
 await installTauriMock(page,{androidRoster:true,fleetSize:2});
 await page.addInitScript(()=>{
  const w=window as unknown as {__TAURI_INTERNALS__:{invoke:(c:string,a?:Record<string,unknown>)=>Promise<unknown>};__INSPECTOR_SAVED__:unknown};const original=w.__TAURI_INTERNALS__.invoke;let recording:Record<string,unknown>|null=null;
  const selector={package:'app.test',description:'Profile'};
  const snapshot={id:'before',udid:'MOCK-FLEET-1',package:'app.test',version:'1.0',locale:'en',width:360,height:720,pngBase64:'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jXioAAAAASUVORK5CYII=',treeSha256:'one',elements:[{index:1,parent:null,text:'Profile',description:'Profile',resourceId:'app.test:id/profile',className:'Button',x:0,y:600,width:100,height:60,enabled:true,clickable:true,selector}]};
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
 await inspector.getByRole('button',{name:'Profile Có thể bấm',exact:true}).click();
 await expect(inspector.getByText('Tìm đúng 1 phần tử bằng thuộc tính')).toBeVisible();
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
 await page.screenshot({path:test.info().outputPath('inspector.png')});
});
