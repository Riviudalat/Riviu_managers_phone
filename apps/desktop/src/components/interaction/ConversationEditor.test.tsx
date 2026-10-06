import { act, fireEvent, render, screen, waitFor, cleanup } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ConversationEditor } from "./ConversationEditor";
import { DEFAULT_DRAFT, buildRequest, validateDraft } from "../../interactionPlan";
import { interactionDraftConversation, interactionParseConversation } from "../../api";
import type { DeviceInfo, ResolvedTikTokTarget, ScriptedConversation } from "../../types";
vi.mock("../../api",()=>({interactionParseConversation:vi.fn(),interactionDraftConversation:vi.fn()}));
afterEach(cleanup);
const target=(id:string)=>({targetKey:id,kind:"video",author:id,contentId:id,normalizedUrl:`https://www.tiktok.com/@${id}/video/123`,originalUrl:`https://www.tiktok.com/@${id}/video/123`}) as ResolvedTikTokTarget;
const script:ScriptedConversation={schemaVersion:1,durationMinutes:120,seed:1,roleBindings:[{roleId:"a",udid:"phone-a",username:"actual_a"},{roleId:"b",udid:"phone-b",username:"actual_b"}],targetScripts:[{targetKey:"one",steps:[{id:"s1",topic:"Vibe",speakerId:"a",text:"Ở đâu vậy?",parentStepId:null,mentionRoleIds:[]},{id:"s2",topic:"Vibe",speakerId:"b",text:"Có địa chỉ trong bài",parentStepId:"s1",mentionRoleIds:["a"]}]}]};
describe("scripted conversation",()=>{
 it("keeps the selected target and gives generated and saved steps the same preview/run contract",async()=>{
  const onChange=vi.fn();
  vi.mocked(interactionDraftConversation).mockResolvedValueOnce(script.targetScripts[0].steps);
  render(<ConversationEditor draft={{...DEFAULT_DRAFT,textSource:"script",conversationJson:JSON.stringify(script)}} onChange={onChange} targets={[target("one"),target("two")]} devices={[]} handles={{}}/>);
  fireEvent.change(screen.getByLabelText("Kịch bản của bài"),{target:{value:"two"}});
  fireEvent.change(screen.getByLabelText("Mô tả hoặc caption của bài đang chọn"),{target:{value:"Caption của bài hai"}});
  fireEvent.change(screen.getByLabelText("Vai cho AI"),{target:{value:"a, b"}});
  fireEvent.change(screen.getByLabelText("Số câu AI soạn"),{target:{value:"2"}});
  fireEvent.click(screen.getByRole("button",{name:"Soạn để duyệt"}));
  await waitFor(()=>expect(onChange).toHaveBeenCalledOnce());
  expect(vi.mocked(interactionDraftConversation).mock.lastCall).toEqual(["Caption của bài hai","Nói tự nhiên, nội dung nối đúng câu trước",["a","b"],2]);
  const generated=JSON.parse(onChange.mock.calls[0][0]);
  expect(generated).toEqual({...script,targetScripts:[...script.targetScripts,{targetKey:"two",steps:script.targetScripts[0].steps}]});
  const restored={...DEFAULT_DRAFT,textSource:"script" as const,conversationJson:JSON.stringify(generated)};
  const context={requestId:"r",targets:[target("one"),target("two")],actorUdids:["phone-a","phone-b"],mentions:[],largestCohort:2};
  expect(buildRequest(restored,{...context,purpose:"preview"}).scriptedConversation).toEqual(generated);
  expect(buildRequest(restored,{...context,purpose:"run"}).scriptedConversation).toEqual(generated);
  expect((screen.getByLabelText("Kịch bản của bài") as HTMLSelectElement).value).toBe("two");
 });
 it.each(["Giọng điệu và yêu cầu", "Vai cho AI", "Số câu AI soạn", "target", "draft", "unmount", "device telemetry", "device membership"])("applies a pending AI draft only to unchanged inputs after %s changes",async(field)=>{
  let resolve!: (steps: typeof script.targetScripts[0]["steps"]) => void;
  vi.mocked(interactionDraftConversation).mockReturnValueOnce(new Promise(done=>{resolve=done;}));
  const onChange=vi.fn();
  const props={draft:{...DEFAULT_DRAFT,textSource:"script" as const,conversationJson:JSON.stringify(script)},onChange,targets:[target("one")],devices:[{udid:"phone-a",platform:"android",battery:50,status:"ready",tileStreamState:"live",streamUrl:"http://localhost/stream-a"} as DeviceInfo],handles:{}};
  const view=render(<ConversationEditor {...props}/>);
  fireEvent.change(screen.getByLabelText("Mô tả hoặc caption của bài đang chọn"),{target:{value:"Caption của bài một"}});
  fireEvent.click(screen.getByRole("button",{name:"Soạn để duyệt"}));
  if(field==="target")view.rerender(<ConversationEditor {...props} targets={[target("two")]}/>);
  else if(field==="draft")view.rerender(<ConversationEditor {...props} draft={{...props.draft,conversationJson:JSON.stringify({...script,seed:2})}}/>);
  else if(field==="unmount")view.unmount();
  else if(field==="device telemetry")view.rerender(<ConversationEditor {...props} devices={[{...props.devices[0],battery:51,status:"busy",tileStreamState:"parked",streamUrl:"http://localhost/stream-b"}]}/>);
  else if(field==="device membership")view.rerender(<ConversationEditor {...props} devices={[{...props.devices[0],udid:"phone-b"}]}/>);
  else fireEvent.change(screen.getByLabelText(field),{target:{value:field==="Số câu AI soạn"?"4":"changed"}});
  await act(async()=>resolve(script.targetScripts[0].steps));
  if(field==="device telemetry"){
   expect(onChange).toHaveBeenCalledOnce();
   expect(JSON.parse(onChange.mock.calls[0][0]).targetScripts[0].steps).toEqual(script.targetScripts[0].steps);
  }else expect(onChange).not.toHaveBeenCalled();
 });
 it("does not show an old failure against a newly selected target",async()=>{
  let reject!: (error: Error) => void;
  vi.mocked(interactionParseConversation).mockReturnValueOnce(new Promise((_done,fail)=>{reject=fail;}));
  render(<ConversationEditor draft={{...DEFAULT_DRAFT,textSource:"script",conversationJson:JSON.stringify(script)}} onChange={vi.fn()} targets={[target("one"),target("two")]} devices={[]} handles={{}}/>);
  fireEvent.change(screen.getByLabelText("Dán hội thoại: @vai: nội dung"),{target:{value:"@a: hello"}});
  fireEvent.click(screen.getByRole("button",{name:"Phân tích kịch bản"}));
  fireEvent.change(screen.getByLabelText("Kịch bản của bài"),{target:{value:"two"}});
  await act(async()=>reject(new Error("old target failure")));
  expect(screen.queryByRole("alert")).toBeNull();
  expect((screen.getByLabelText("Kịch bản của bài") as HTMLSelectElement).value).toBe("two");
 });
 it.each([["Vai cho AI","a, a"],["Vai cho AI","a"],["Số câu AI soạn","2.5"],["Số câu AI soạn","65"]])("blocks invalid generation input %s=%s before IPC",(label,value)=>{
  render(<ConversationEditor draft={{...DEFAULT_DRAFT,textSource:"script"}} onChange={vi.fn()} targets={[target("one")]} devices={[]} handles={{}}/>);
  fireEvent.change(screen.getByLabelText("Mô tả hoặc caption của bài đang chọn"),{target:{value:"Caption"}});
  fireEvent.change(screen.getByLabelText(label),{target:{value}});
  expect((screen.getByRole("button",{name:"Soạn để duyệt"}) as HTMLButtonElement).disabled).toBe(true);
 });
 it("uses saved usernames and operator numbers, and refreshes a changed binding",async()=>{
  const onChange=vi.fn();
  const props={draft:{...DEFAULT_DRAFT,textSource:"script" as const,conversationJson:JSON.stringify(script)},onChange,targets:[target("one")],devices:[{udid:"phone-b",name:"Display name",platform:"android"} as DeviceInfo],deviceNumber:new Map([["phone-b",7]]),deviceLabel:new Map([["phone-b","Máy phụ"]])};
  const {rerender}=render(<ConversationEditor {...props} handles={{"phone-b":"actual_b"}}/>);
  expect(screen.getAllByRole("option",{name:"Máy 7 · Máy phụ · @actual_b"})).toHaveLength(2);
  expect(onChange).not.toHaveBeenCalled();
  rerender(<ConversationEditor {...props} handles={{"phone-b":"updated_b"}}/>);
  await waitFor(()=>expect(onChange).toHaveBeenCalled());
  expect(JSON.parse(onChange.mock.calls[0][0]).roleBindings[1].username).toBe("updated_b");
 });
 it("opens the existing account editor for a numbered device missing its username",()=>{
  const onAssignAccount=vi.fn();
  render(<ConversationEditor draft={{...DEFAULT_DRAFT,textSource:"script",conversationJson:JSON.stringify({...script,roleBindings:[{roleId:"a",udid:"phone-a",username:"actual_a"},{roleId:"b",udid:"phone-b",username:""}]})}} onChange={vi.fn()} onAssignAccount={onAssignAccount} targets={[target("one")]} devices={[]} handles={{"phone-a":"actual_a","phone-b":""}} deviceNumber={new Map([["phone-b",7]])}/>);
  expect(screen.getByText("Máy 7 chưa gán username TikTok")).toBeTruthy();
  fireEvent.click(screen.getByRole("button",{name:"Gán tài khoản"}));
  expect(onAssignAccount).toHaveBeenCalledWith("phone-b");
 });
 it("preserves per-post text and fixed role bindings in preview and execution",()=>{
  const draft={...DEFAULT_DRAFT,textSource:"script" as const,conversationJson:JSON.stringify(script)};
  const context={requestId:"r",targets:[target("one")],actorUdids:["phone-a","phone-b"],mentions:[],largestCohort:2};
  expect(buildRequest(draft,{...context,purpose:"preview"}).scriptedConversation).toEqual(script);
  expect(buildRequest(draft,{...context,purpose:"run"}).scriptedConversation).toEqual(script);
 });
 it("retains the draft but does not send a script when comments are disabled",()=>{
  const draft={...DEFAULT_DRAFT,textSource:"script" as const,conversationJson:JSON.stringify(script),actions:{like:true,save:false,comment:false}};
  const request=buildRequest(draft,{requestId:"r",targets:[target("one")],actorUdids:["phone-a"],mentions:[],largestCohort:1});
  expect(request.scriptedConversation).toBeUndefined();expect(request.mode).toBe("standalone");expect(draft.conversationJson).toBe(JSON.stringify(script));
 });
 it("refuses a newly added link without its own script or insufficient window",()=>{
  const context={targets:[target("one"),target("two")],actorUdids:["phone-a","phone-b"],largestCohort:2,badLineCount:0,mixedThread:false};
  const draft={...DEFAULT_DRAFT,textSource:"script" as const,conversationJson:JSON.stringify({...script,durationMinutes:1})};
  expect(validateDraft(draft,context).map(i=>i.message).join(" ")).toContain("Mỗi link cần kịch bản riêng");
  expect(validateDraft(draft,context).map(i=>i.message).join(" ")).toContain("tối thiểu");
 });
 it("pasting for a second post preserves the first post script and fixed roles",async()=>{
  const onChange=vi.fn();vi.mocked(interactionParseConversation).mockResolvedValue(script.targetScripts[0].steps);
  render(<ConversationEditor draft={{...DEFAULT_DRAFT,textSource:"script",conversationJson:JSON.stringify(script)}} onChange={onChange} targets={[target("one"),target("two")]} devices={[]} handles={{}}/>);
  fireEvent.change(screen.getByLabelText("Kịch bản của bài"),{target:{value:"two"}});
  fireEvent.change(screen.getByLabelText("Dán hội thoại: @vai: nội dung"),{target:{value:"@a: Ở đâu vậy?\n@b: @a Có địa chỉ trong bài"}});
  fireEvent.click(screen.getByRole("button",{name:"Phân tích kịch bản"}));
  await waitFor(()=>expect(onChange).toHaveBeenCalled());
  const result=JSON.parse(onChange.mock.calls[0][0]);expect(result.targetScripts).toHaveLength(2);expect(result.targetScripts[0]).toEqual(script.targetScripts[0]);expect(result.roleBindings).toEqual(script.roleBindings);
 });
});
