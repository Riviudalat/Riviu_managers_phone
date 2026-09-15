import { fireEvent, render, screen, waitFor, cleanup } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ConversationEditor } from "./ConversationEditor";
import { DEFAULT_DRAFT, buildRequest, validateDraft } from "../../interactionPlan";
import { interactionParseConversation } from "../../api";
import type { DeviceInfo, ResolvedTikTokTarget, ScriptedConversation } from "../../types";
vi.mock("../../api",()=>({interactionParseConversation:vi.fn(),interactionDraftConversation:vi.fn()}));
afterEach(cleanup);
const target=(id:string)=>({targetKey:id,kind:"video",author:id,contentId:id,normalizedUrl:`https://www.tiktok.com/@${id}/video/123`,originalUrl:`https://www.tiktok.com/@${id}/video/123`}) as ResolvedTikTokTarget;
const script:ScriptedConversation={schemaVersion:1,durationMinutes:120,seed:1,roleBindings:[{roleId:"a",udid:"phone-a",username:"actual_a"},{roleId:"b",udid:"phone-b",username:"actual_b"}],targetScripts:[{targetKey:"one",steps:[{id:"s1",topic:"Vibe",speakerId:"a",text:"Ở đâu vậy?",parentStepId:null,mentionRoleIds:[]},{id:"s2",topic:"Vibe",speakerId:"b",text:"Có địa chỉ trong bài",parentStepId:"s1",mentionRoleIds:["a"]}]}]};
describe("scripted conversation",()=>{
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
