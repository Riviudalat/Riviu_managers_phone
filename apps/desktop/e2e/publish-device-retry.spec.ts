import {test,expect} from "@playwright/test";
import {installTauriMock} from "./fixtures/tauriMock";

for (const width of [1440,820]) test(`monitor retries just one failed device at ${width}`,async({page})=>{
 await page.setViewportSize({width,height:width===820?560:900});await installTauriMock(page,{androidRoster:true,fleetSize:2});
 await page.addInitScript(()=>{
  const w=window as unknown as {__TAURI_INTERNALS__:{invoke:(command:string,args:Record<string,unknown>)=>Promise<unknown>};retryCalls:Record<string,unknown>[]};
  const original=w.__TAURI_INTERNALS__.invoke;w.retryCalls=[];
  const summary={id:"publish:retry",sourceId:"retry",kind:"publish",title:"Đăng bài",state:"partial",targetCount:2,totalItems:2,completedItems:1,issueCount:1,retryableCount:1,retryScope:null,createdAt:null,updatedAt:null};
  w.__TAURI_INTERNALS__.invoke=async(command,args)=>{
   if(command==="operation_query_runs")return {runs:[summary],total:1,counts:{active:0,succeeded:0,attention:1},hasMore:false};
   if(command==="operation_get_run")return {summary,items:[{id:"failed",udid:"retry-phone",kind:"assignment",label:"Máy 1",state:w.retryCalls.length?"queued":"failed",detail:null,errorCode:"sound_timeout",retryable:true},{id:"posted",udid:"posted-phone",kind:"assignment",label:"Máy 2",state:"succeeded",detail:null,errorCode:null,retryable:false}]};
   if(command==="list_devices")return [{udid:"retry-phone",name:"Phone1",status:"ready",platform:"android",connection:"usb"},{udid:"posted-phone",name:"Phone2",status:"ready",platform:"android",connection:"usb"}];
   if(command==="publish_get")return {campaign:{id:"retry"},assignments:[{id:"failed",udid:"retry-phone",state:w.retryCalls.length?"queued":"failedBeforeDispatch",effectIntent:null},{id:"posted",udid:"posted-phone",state:"succeeded",effectIntent:"post"}]};
   if(command==="publish_recovery_capabilities")return [{assignmentId:"failed",revision:8,retryBeforePost:{allowed:!w.retryCalls.length,reason:null},checkLink:{allowed:false},resumeVerification:{allowed:false},recovery:{step:"sound",state:"failed",retriesUsed:3,maxRetries:3}},{assignmentId:"posted",revision:9,retryBeforePost:{allowed:false},checkLink:{allowed:false},resumeVerification:{allowed:false}}];
   if(command==="publish_retry_assignment"){w.retryCalls.push(args);return null;}
   if(command==="operation_device_log")return {entries:[],truncated:false};
   if(command==="publish_execute"||command==="publish_create_campaign")throw Error("No campaign redispatch");
   return original(command,args);
  };
 });
 await page.goto("/");await page.getByRole("button",{name:"Mở rộng tiến trình",exact:true}).click();
 const monitor=page.getByRole("dialog",{name:"Cửa sổ tiến trình"});const retry=monitor.getByRole("button",{name:"Thử lại",exact:true});await expect(retry).toHaveCount(1);await retry.click();
 await expect.poll(()=>page.evaluate(()=>(window as unknown as {retryCalls:unknown[]}).retryCalls.length)).toBe(1);
 expect(await page.evaluate(()=>(window as unknown as {retryCalls:Record<string,unknown>[]}).retryCalls[0])).toMatchObject({assignmentId:"failed",confirmed:true,expectedRevision:8});
 await expect(monitor.getByRole("button",{name:"Thử lại",exact:true})).toHaveCount(0);
 await monitor.screenshot({path:test.info().outputPath(`retry-${width}.png`)});
});
