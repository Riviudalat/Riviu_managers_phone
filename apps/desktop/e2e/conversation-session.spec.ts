import { expect, test } from "@playwright/test";
import { installTauriMock } from "./fixtures/tauriMock";

for (const width of [1440,820]) {
  test(`per-link conversation keeps roles, raw draft and shared duration at ${width}`,async({page})=>{
    await page.setViewportSize({width,height:900});await installTauriMock(page,{androidRoster:true,fleetSize:2});
    await page.addInitScript(()=>{
      const urls=["https://www.tiktok.com/@one/video/123","https://www.tiktok.com/@two/video/456"];
      const steps=[{id:"s1",topic:"Vibe",speakerId:"a",text:"Cho mình hỏi địa chỉ?",parentStepId:null,mentionRoleIds:[]},{id:"s2",topic:"Vibe",speakerId:"b",text:"Thông tin có trong bài nhé",parentStepId:"s1",mentionRoleIds:["a"]}];
      if (!localStorage.getItem("riviu.form-draft.v1.interaction")) localStorage.setItem("riviu.form-draft.v1.interaction",JSON.stringify({schemaVersion:1,value:{rawLinks:urls.join("\n"),messageCount:2,maxWords:12,threadKind:"chain",textSource:"script",instruction:"",manualText:"",actions:{like:false,save:false,comment:true},mentionParent:true,mentionText:"",actors:["MOCK-FLEET-1","MOCK-FLEET-2"],conversationRawJson:"{}",conversationJson:JSON.stringify({schemaVersion:1,durationMinutes:120,seed:1,roleBindings:[{roleId:"a",udid:"MOCK-FLEET-1",username:"a"},{roleId:"b",udid:"MOCK-FLEET-2",username:"b"}],targetScripts:[{targetKey:"content:123",steps},{targetKey:"content:456",steps:steps.map(step=>({...step,text:step.text+" Bài hai"}))}]})}}));
      const w=window as unknown as {__TAURI_INTERNALS__:{invoke:(cmd:string,args:Record<string,unknown>)=>Promise<unknown>};__effects:string[]};
      const invoke=w.__TAURI_INTERNALS__.invoke;w.__effects=[];
      w.__TAURI_INTERNALS__.invoke=async(command,args)=>{
        if(command==="startup_error")return null;
        if(command==="interaction_parse_links")return String(args.rawText).split("\n").filter(Boolean).map((url,index)=>{const match=url.match(/@([^/]+)\/(video|photo)\/(\d+)/)!;return {lineNo:index+1,original:url,error:null,target:{originalUrl:url,normalizedUrl:url,author:match[1],kind:match[2],contentId:match[3],targetKey:`content:${match[3]}`}};});
        if(command==="interaction_preview_thread")return {lines:[],plan:null,validTargetCount:2,cohortCount:1,streamCapacity:2};
        if(["interaction_start_thread","interaction_retry","interaction_read_account"].includes(command)){w.__effects.push(command);throw Error("unexpected public action");}
        return invoke(command,args);
      };
    });
    const errors:string[]=[];page.on("pageerror",e=>errors.push(e.message));
    await page.goto("/");await page.getByRole("button",{name:"Tương tác",exact:true}).click();
    const workspace=page.getByRole("region",{name:"Không gian Tương tác"});
    await workspace.getByRole("button",{name:"Chọn hành động & máy →"}).click();
    await expect(page.getByLabel("Thời lượng phiên (phút)")).toHaveValue("120");
    await expect(page.getByLabel("Nội dung câu 1")).toHaveValue("Cho mình hỏi địa chỉ?");
    await page.getByLabel("Kịch bản của bài").selectOption("content:456");
    await expect(page.getByLabel("Nội dung câu 1")).toHaveValue("Cho mình hỏi địa chỉ? Bài hai");
    await page.getByLabel("Dán hội thoại: @vai: nội dung").fill("@a: bản nháp chưa phân tích");
    await page.getByLabel("Thời lượng phiên (phút)").fill("180");
    await page.screenshot({path:test.info().outputPath(`conversation-${width}.png`)});
    await expect.poll(()=>page.evaluate(()=>JSON.parse(JSON.parse(localStorage.getItem("riviu.form-draft.v1.interaction")!).value.conversationJson).durationMinutes)).toBe(180);
    await page.reload();await page.getByRole("button",{name:"Tương tác",exact:true}).click();
    await page.getByRole("region",{name:"Không gian Tương tác"}).getByRole("button",{name:"Chọn hành động & máy →"}).click();
    await expect(page.getByLabel("Thời lượng phiên (phút)")).toHaveValue("180");
    await page.getByLabel("Kịch bản của bài").selectOption("content:456");
    await expect(page.getByLabel("Dán hội thoại: @vai: nội dung")).toHaveValue("@a: bản nháp chưa phân tích");
    expect(await page.evaluate(()=>(window as unknown as {__effects:string[]}).__effects)).toEqual([]);
    expect(errors).toEqual([]);
  });
}
