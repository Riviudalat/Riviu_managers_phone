import { useRef, useState } from "react";
import { interactionDraftConversation, interactionParseConversation } from "../../api";
import { conversationOf, type InteractionDraft } from "../../interactionPlan";
import type { ConversationStep, DeviceInfo, ResolvedTikTokTarget, ScriptedConversation } from "../../types";
import { describeError } from "../../describeError";

const empty = (): ScriptedConversation => ({schemaVersion:1,durationMinutes:120,seed:1,targetScripts:[],roleBindings:[]});

export function ConversationEditor({draft, onChange, onRawChange, targets, devices, handles}: {
  draft: InteractionDraft; onChange:(json:string)=>void; onRawChange?:(json:string)=>void; targets:ResolvedTikTokTarget[];
  devices:DeviceInfo[]; handles:Record<string,string>;
}) {
  const current=conversationOf(draft) ?? empty();
  const currentRef=useRef(current);currentRef.current=current;
  const [selected,setSelected]=useState("");const targetKey=targets.some(t=>t.targetKey===selected)?selected:targets[0]?.targetKey ?? "";
  const [raw,setRaw]=useState<Record<string,string>>(()=>{try { const data=JSON.parse(draft.conversationRawJson||"{}"); return data && typeof data==="object" && !Array.isArray(data) ? Object.fromEntries(Object.entries(data).filter((entry):entry is [string,string]=>typeof entry[1]==="string")) : {}; } catch { return {}; }});const [contexts,setContexts]=useState<Record<string,string>>({});const context=contexts[targetKey]??"";const setContext=(value:string)=>{generation.current+=1;setContexts({...contexts,[targetKey]:value});};
  const [direction,setDirection]=useState("Nói tự nhiên, nội dung nối đúng câu trước");
  const [aiRoles,setAiRoles]=useState("vai_a, vai_b");const [count,setCount]=useState(6);
  const [error,setError]=useState("");const [busy,setBusy]=useState(false);const generation=useRef(0);
  const steps=current.targetScripts.find(t=>t.targetKey===targetKey)?.steps ?? [];
  const roles=[...new Set(current.targetScripts.flatMap(t=>t.steps.flatMap(s=>[s.speakerId,...s.mentionRoleIds])))];
  const save=(value:ScriptedConversation)=>{generation.current+=1;onChange(JSON.stringify(value));};
  const saveSteps=(key:string,values:ConversationStep[])=>{
    const value=currentRef.current;
    const targetScripts=[...value.targetScripts.filter(t=>t.targetKey!==key),{targetKey:key,steps:values}];
    const used=[...new Set(targetScripts.flatMap(t=>t.steps.flatMap(s=>[s.speakerId,...s.mentionRoleIds])))];
    const roleBindings=value.roleBindings.filter(r=>used.includes(r.roleId));
    for(const role of used){if(roleBindings.some(r=>r.roleId===role))continue;
      const device=devices.find(d=>d.platform==="android" && !roleBindings.some(r=>r.udid===d.udid) && (handles[d.udid]??"").replace(/^@/,"").toLowerCase()===role.toLowerCase());
      roleBindings.push({roleId:role,udid:device?.udid??"",username:device?handles[device.udid].replace(/^@/,""):""});
    }
    save({...value,targetScripts,roleBindings});
  };
  const parse=async(ai:boolean)=>{
    if(!targetKey||busy)return;setBusy(true);setError("");const ticket=++generation.current;const key=targetKey;
    try {
      const values=ai?await interactionDraftConversation(context,direction,aiRoles.split(/[,\s]+/).filter(Boolean),count):await interactionParseConversation(raw[key]??"");
      if(ticket===generation.current)saveSteps(key,values);
    }catch(e){setError(describeError(e));}finally{setBusy(false);}
  };
  const edit=(index:number,change:Partial<ConversationStep>)=>saveSteps(targetKey,steps.map((s,i)=>i===index?{...s,...change}:s));
  const total=current.targetScripts.reduce((sum,t)=>sum+t.steps.length,0);
  const setWindow=(name:"startsAt"|"endsAt",value:string)=>{const at=value?new Date(value):null;if(at&&!Number.isFinite(at.getTime()))return;save({...current,[name]:at?at.toISOString():null});};
  const localTime=(value?:string|null)=>{if(!value)return "";const at=new Date(value);if(!Number.isFinite(at.getTime()))return "";return new Date(at.getTime()-at.getTimezoneOffset()*60000).toISOString().slice(0,16);};
  return <section aria-label="Kịch bản hội thoại" className="conversation-editor">
    <div className="iw-fields">
      <label className="iw-field"><span>Thời lượng phiên (phút)</span><input type="number" min={1} max={1440} value={current.durationMinutes} onChange={e=>save({...current,durationMinutes:Number(e.target.value),startsAt:null,endsAt:null})}/></label>
      <span>{total} câu · dự toán tối thiểu {Math.ceil(total*2/0.9)} phút</span>
    </div>
    <details><summary>Đặt khung giờ bắt đầu – kết thúc</summary><div className="iw-fields">
      <label className="iw-field"><span>Bắt đầu</span><input type="datetime-local" value={localTime(current.startsAt)} onChange={e=>setWindow("startsAt",e.target.value)}/></label>
      <label className="iw-field"><span>Kết thúc</span><input type="datetime-local" value={localTime(current.endsAt)} onChange={e=>setWindow("endsAt",e.target.value)}/></label>
    </div></details>
    <label className="iw-field"><span>Kịch bản của bài</span><select value={targetKey} onChange={e=>{generation.current+=1;setSelected(e.target.value);setError("");}}>
      {targets.map((t,i)=><option key={t.targetKey} value={t.targetKey}>Bài {i+1} · @{t.author} · {current.targetScripts.find(s=>s.targetKey===t.targetKey)?.steps.length??0} câu</option>)}
    </select></label>
    <label className="iw-field"><span>Dán hội thoại: @vai: nội dung</span><textarea rows={5} value={raw[targetKey]??""} onChange={e=>{generation.current+=1;const next={...raw,[targetKey]:e.target.value};setRaw(next);onRawChange?.(JSON.stringify(next));}} placeholder={"Nhánh 1\n@vai_a: Cho mình hỏi địa chỉ?\n@vai_b: @vai_a Địa chỉ theo thông tin bài nhé"}/></label>
    <button type="button" className="ghost" disabled={!targetKey||busy||!raw[targetKey]?.trim()} onClick={()=>void parse(false)}>Phân tích kịch bản</button>
    <details><summary>AI soạn bản để duyệt</summary>
      <label className="iw-field"><span>Mô tả hoặc caption của bài đang chọn</span><textarea value={context} onChange={e=>setContext(e.target.value)} rows={3}/></label>
      <label className="iw-field"><span>Giọng điệu và yêu cầu</span><input value={direction} onChange={e=>setDirection(e.target.value)}/></label>
      <div className="iw-fields"><label className="iw-field"><span>Vai cho AI</span><input value={aiRoles} onChange={e=>setAiRoles(e.target.value)}/></label><label className="iw-field"><span>Số câu AI soạn</span><input type="number" min={2} max={64} value={count} onChange={e=>setCount(Number(e.target.value))}/></label></div>
      <button type="button" disabled={busy||!context.trim()||!targetKey} onClick={()=>void parse(true)}>Soạn để duyệt</button>
    </details>
    {busy&&<p role="status">Đang chuẩn bị kịch bản…</p>}{error&&<p role="alert">{error}</p>}
    {steps.length>0&&<div className="iw-table-scroll" tabIndex={0}><table className="iw-table" aria-label="Các câu trong kịch bản"><thead><tr><th>Câu / chủ đề</th><th>Người nói</th><th>Trả lời</th><th>Nội dung</th><th>Tag vai</th></tr></thead><tbody>{steps.map((s,i)=><tr key={s.id}>
      <td>{i+1}<input aria-label={`Chủ đề câu ${i+1}`} value={s.topic} onChange={e=>edit(i,{topic:e.target.value})}/></td>
      <td><input aria-label={`Người nói câu ${i+1}`} value={s.speakerId} onChange={e=>edit(i,{speakerId:e.target.value})}/></td>
      <td><select aria-label={`Trả lời câu ${i+1}`} value={s.parentStepId??""} onChange={e=>edit(i,{parentStepId:e.target.value||null})}><option value="">Bình luận gốc</option>{steps.slice(0,i).filter(p=>p.topic===s.topic).map(p=><option key={p.id} value={p.id}>{steps.indexOf(p)+1} · {p.speakerId}</option>)}</select></td>
      <td><textarea aria-label={`Nội dung câu ${i+1}`} rows={3} value={s.text} onChange={e=>edit(i,{text:e.target.value})}/></td>
      <td><input aria-label={`Tag câu ${i+1}`} value={s.mentionRoleIds.join(", ")} onChange={e=>edit(i,{mentionRoleIds:e.target.value.split(/[,\s]+/).filter(Boolean).map(v=>v.replace(/^@/,""))})}/></td>
    </tr>)}</tbody></table></div>}
    {roles.length>0&&<><h4>Vai → máy → tài khoản</h4><div className="iw-fields">{roles.map(role=>{
      const binding=current.roleBindings.find(r=>r.roleId===role);return <label className="iw-field" key={role}><span>@{role}</span><select aria-label={`Máy cho vai ${role}`} value={binding?.udid??""} onChange={e=>save({...current,roleBindings:[...current.roleBindings.filter(r=>r.roleId!==role),{roleId:role,udid:e.target.value,username:(handles[e.target.value]??"").replace(/^@/,"")}]})}>
        <option value="">Chọn máy</option>{devices.filter(d=>d.platform==="android").map(d=><option key={d.udid} value={d.udid}>{d.name} · @{handles[d.udid]||"chưa có nick"}</option>)}
      </select></label>;
    })}</div></>}
    <p className="iw-help">Mỗi bài dùng nội dung riêng. Các câu chạy xen kẽ trong cùng khung giờ; reply chờ đúng câu cha và tag được TikTok xác nhận.</p>
  </section>;
}
