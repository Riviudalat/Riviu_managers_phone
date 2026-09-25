import {useCallback,useEffect,useRef,useState} from "react";
import {interactionDraftSeeding} from "../../api";
import {buildRequest,type InteractionDraft} from "../../interactionPlan";
import type {ResolvedTikTokTarget,SeedingConfig} from "../../types";
import {describeError} from "../../describeError";

export function SeedingControls({draft,actors,targets,onChange}:{draft:InteractionDraft;actors:string[];targets:ResolvedTikTokTarget[];onChange:(next:InteractionDraft)=>void}) {
 const [contexts,setContexts]=useState<Record<string,string>>({});
 const [busy,setBusy]=useState(false),[error,setError]=useState("");const ticket=useRef(0);
 const s=draft.seeding;
 const generationInput=JSON.stringify([draft,actors,targets,contexts]);
 const invalidate=useCallback(()=>{ticket.current++;},[]);
 useEffect(()=>{invalidate();return invalidate;},[generationInput,invalidate]);
 const update=(patch:Partial<SeedingConfig>)=>{if(!s)return;ticket.current++;const next={...s,...patch};onChange({...draft,seeding:next,actions:{...draft.actions,follow:false,like:next.likeCount>0,save:next.saveCount>0,share:next.shareCount>0}});};
 const enable=()=>onChange({...draft,textSource:"ai",messageCount:40,threadKind:"chain",seeding:{standaloneCount:20,likeCount:0,saveCount:0,shareCount:0,seed:Math.floor(Math.random()*0x7fffffff),preferredActors:[],watchSeconds:{min:5,max:15},commentGapSeconds:{min:20,max:45},comments:{}},actions:{...draft.actions,follow:false,like:false,save:false,share:false}});
 const generate=async()=>{if(!s||busy)return;const at=++ticket.current;setBusy(true);setError("");try{const request=buildRequest(draft,{requestId:crypto.randomUUID(),targets,actorUdids:actors,largestCohort:actors.length,mentions:[],purpose:"preview"});const comments=await interactionDraftSeeding(request,contexts);if(ticket.current===at)update({comments});}catch(e){setError(describeError(e));}finally{setBusy(false);}};
 return <section className="iw-advanced" aria-label="Phân bổ seeding">
  <label className="iw-checkbox"><input type="checkbox" checked={!!s} onChange={e=>{ticket.current++;if(e.target.checked)enable();else onChange({...draft,seeding:undefined,actions:{...draft.actions,share:false}});}}/>Phân bổ số lượt và cụm hội thoại</label>
  {s&&<>
   <div className="iw-fields">{([['likeCount','Tim'],['saveCount','Lưu'],['shareCount','Share cho bạn bè']] as const).map(([key,label])=><label className="iw-field" key={key}><span>{label}</span><input type="number" min={0} max={actors.length} value={s[key]} onChange={e=>update({[key]:Math.max(0,Math.trunc(Number(e.target.value)))})}/></label>)}</div>
   <label className="iw-field"><span>Máy thực hiện</span><select value={s.preferredActors.length?'specified':'random'} onChange={e=>update({preferredActors:e.target.value==='specified'?[...actors]:[]})}><option value="random">Ngẫu nhiên trong máy đã chọn</option><option value="specified">Ưu tiên máy chỉ định</option></select></label>
   {!!s.preferredActors.length&&<div aria-label="Máy ưu tiên seeding">{actors.map(actor=><label key={actor}><input type="checkbox" checked={s.preferredActors.includes(actor)} onChange={e=>update({preferredActors:e.target.checked?[...s.preferredActors,actor]:s.preferredActors.filter(a=>a!==actor)})}/>{actor}</label>)}</div>}
   <p className="iw-help">Tim/Lưu đã có thì bù bằng máy khác trong phạm vi. Share gửi một bạn TikTok ngẫu nhiên; không có bạn thì bỏ qua. Không gửi lại khi chưa rõ kết quả.</p>
   {draft.actions.comment&&<div className="iw-fields"><label className="iw-field"><span>Tổng bình luận mỗi bài</span><input type="number" min={1} max={64} value={draft.messageCount??40} onChange={e=>{ticket.current++;onChange({...draft,messageCount:Math.trunc(Number(e.target.value)),seeding:{...s,comments:{}}});}}/></label><label className="iw-field"><span>Bình luận đơn</span><input type="number" min={0} max={draft.messageCount??40} value={s.standaloneCount} onChange={e=>update({standaloneCount:Math.trunc(Number(e.target.value)),comments:{}})}/></label><p>{(draft.messageCount??40)-s.standaloneCount} câu hội thoại, gồm câu gốc · tối đa 4 tài khoản/cụm</p></div>}
   {([['watchSeconds','Xem trước hành động'],['commentGapSeconds','Nghỉ giữa bình luận']]as const).map(([key,label])=><div className="iw-fields" key={key}><label className="iw-field"><span>{label} · ít nhất (giây)</span><input type="number" min={0} max={600} value={s[key].min} onChange={e=>update({[key]:{...s[key],min:Number(e.target.value)}})}/></label><label className="iw-field"><span>Nhiều nhất (giây)</span><input type="number" min={0} max={600} value={s[key].max} onChange={e=>update({[key]:{...s[key],max:Number(e.target.value)}})}/></label></div>)}
   {draft.actions.comment&&<><p>Soạn theo caption/mô tả thật, duyệt đủ câu trước khi chạy. Giữ đúng thứ tự câu trong từng bài.</p>{targets.map(t=><div key={t.targetKey}><label className="iw-field"><span>Nội dung nguồn · @{t.author}</span><textarea value={contexts[t.targetKey]??''} onChange={e=>{ticket.current++;setContexts({...contexts,[t.targetKey]:e.target.value});}}/></label><label className="iw-field"><span>Bình luận đã duyệt · mỗi dòng một câu</span><textarea rows={6} value={(s.comments[t.targetKey]??[]).join('\n')} onChange={e=>update({comments:{...s.comments,[t.targetKey]:e.target.value.split('\n').filter(v=>v.trim())}})}/></label></div>)}<button type="button" disabled={busy||!actors.length||!targets.length} onClick={()=>void generate()}>{busy?'Đang soạn…':'AI soạn theo kế hoạch'}</button></>}
   {error&&<p role="alert">{error}</p>}
  </>}
 </section>;
}
