import {expect,test} from "@playwright/test";
import {installTauriMock} from "./fixtures/tauriMock";

for(const width of [1440,1060,820]) test(`stop the chosen task and retain its history at ${width}`,async({page},info)=>{
 await page.setViewportSize({width,height:900});
 await installTauriMock(page,{androidRoster:true,fleetSize:2});
 await page.addInitScript(()=>{
  const w=window as unknown as {__TAURI_INTERNALS__:{invoke:(cmd:string,args:Record<string,unknown>)=>Promise<unknown>};__stopped:string[]};
  const original=w.__TAURI_INTERNALS__.invoke;w.__stopped=[];
  const run={id:"publish:fixture-stop",sourceId:"fixture-stop",kind:"publish",title:"Đăng bài",state:"running",targetCount:2,totalItems:2,completedItems:1,issueCount:0,retryableCount:0,retryScope:null,createdAt:new Date().toISOString(),updatedAt:new Date().toISOString()};
  let stopped=false;
  w.__TAURI_INTERNALS__.invoke=async(cmd,args)=>{
   if(cmd==="operation_query_runs")return {runs:[run],total:1,counts:{active:stopped?0:1,succeeded:0,attention:0},hasMore:false};
   if(cmd==="operation_get_run")return {summary:run,items:[{id:"a",udid:"MOCK-FLEET-1",label:"Máy 1",kind:"assignment",state:"succeeded",retryable:false},{id:"b",udid:"MOCK-FLEET-2",label:"Máy 2",kind:"assignment",state:stopped?"uncertain":"running",retryable:false}]};
   if(cmd==="operation_stop"){w.__stopped.push(String(args.operationId));stopped=true;run.state="cancelled";return {operationId:run.id,state:"stopping",devices:[]};}
   if(cmd==="operation_stop_status")return stopped?{operationId:run.id,state:"closed",devices:[{udid:"MOCK-FLEET-1",closed:true,message:"TikTok đã tắt"},{udid:"MOCK-FLEET-2",closed:true,message:"TikTok đã tắt"}]}:null;
   return original(cmd,args);
  };
 });
 const errors:string[]=[];page.on("pageerror",e=>errors.push(e.message));
 await page.goto("/");
 // The wall's geometry while the run is active; it must be the same after the run ends.
 const wall=async()=>page.evaluate(()=>{const header=document.querySelector(".shell > .main-col > .page-header")!.getBoundingClientRect();
  const tile=document.querySelector("[data-testid='device-tile']")!.getBoundingClientRect();return [header.height,tile.x,tile.y,tile.width,tile.height];});
 await expect(page.getByTestId("device-tile").first()).toBeVisible();
 await expect(page.locator(".queue-run-pill")).not.toHaveClass(/is-idle/);
 const running=await wall();
 await page.getByRole("button",{name:"Tiến trình công việc"}).click();
 const monitor=page.getByRole("complementary",{name:"Hàng đợi Cần xử lý"});
 await expect(monitor.getByRole("button",{name:"Dừng tác vụ và về màn hình chính"})).toBeVisible();
 await page.screenshot({path:info.outputPath(`monitor-stop-${width}.png`)});
 await monitor.getByRole("button",{name:"Dừng tác vụ và về màn hình chính"}).click();
 await expect(monitor.getByText("Đã dừng · TikTok đã tắt trên 2 máy")).toBeVisible();
 expect(await page.evaluate(()=>(window as unknown as {__stopped:string[]}).__stopped)).toEqual(["publish:fixture-stop"]);
 await expect(monitor.getByRole("combobox",{name:"Chọn tác vụ theo dõi"})).toHaveValue("publish:fixture-stop");
 // No run left: the pill goes idle in its reserved slot and nothing under it moves.
 await expect(page.locator(".queue-run-pill")).toHaveClass(/is-idle/);
 expect(await wall()).toEqual(running);
 expect(errors).toEqual([]);
});
