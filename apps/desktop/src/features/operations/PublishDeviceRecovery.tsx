import {useCallback,useEffect,useRef,useState} from "react";
import {operationDeviceLog,publishExcludeAssignment,publishRetryAssignment,publishCheckLinks,publishResumeVerification,publishRetrySheetAssignment,listDevices} from "../../api";
import {useMonitorRead} from "./useMonitorRead";
import {describeError} from "../../describeError";
import {logMessage,publishDeviceProgress,progressLabel} from "./operationProgress";
import {ProgressBar} from "../../components/ProgressBar";
import {PostOutcomeEvidence,PostOutcomeLayers} from "./PostOutcome";
import {postOutcome} from "./postOutcomeModel";
import {usePublishRunRead,type PublishRunCapability} from "./publishRunRead";
import type {PublishExcludeResult,PublishRecoveryCapability} from "../../types";

const stepLabel:Record<string,string>={device:"Chuẩn bị máy",transfer:"Chuyển nội dung",media:"Chọn ảnh/video",sound:"Chọn nhạc",caption:"Nhập caption"};
const accountGuidance: Record<string, string> = {
 account_login_required: "TikTok đang yêu cầu đăng nhập. Mở đúng tài khoản trên máy rồi kiểm tra lại; chưa bấm Đăng.",
 account_security_prompt: "Lời nhắc bảo mật đang che hồ sơ. Xử lý trên điện thoại rồi kiểm tra lại; chưa bấm Đăng.",
 account_unreadable: "Chưa đọc đủ thông tin hồ sơ. Kiểm tra màn hình điện thoại rồi thử lại; chưa kết luận tài khoản đã đổi.",
 account_mismatch: "Tài khoản đang mở khác tài khoản của lượt này. Mở lại đúng tài khoản; không tự đổi tài khoản đã gắn với bài.",
 account_read_failed: "Không đọc được hồ sơ do kết nối hoặc phiên điều khiển. Kiểm tra kết nối máy rồi thử lại.",
 account_draft_banner_persisted: "Banner \"Continue editing this post?\" vẫn còn trên Hồ sơ sau một lần bấm Save draft. Bấm Save draft trên máy (không bấm Edit) rồi thử lại; chưa bấm Đăng.",
 account_draft_banner_unconfirmed: "Đầu trang Hồ sơ bị che nhưng chưa xác nhận được banner nháp. Xem máy, nếu có banner thì bấm Save draft (không bấm Edit) rồi thử lại; chưa bấm Đăng.",
 publish_device_not_released: "Lúc bắt đầu chưa nhả được máy này nên chưa gửi gì lên máy; các máy khác vẫn đăng. Khi máy rảnh, bấm Thử lại.",
};
function object(value: unknown): Record<string, unknown> {
 return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}
function readEvidence(raw: string | null | undefined): Record<string, unknown> {
 // Evidence may include large artifacts; decline oversized/malformed envelopes.
 if (!raw || raw.length > 262144) return {};
 try { return object(JSON.parse(raw)); } catch { return {}; }
}
function budgetPair(used: unknown, limit: unknown): string | null {
 return typeof used === "number" && Number.isSafeInteger(used) && used >= 0
  && typeof limit === "number" && Number.isSafeInteger(limit) && limit > 0 ? `${used}/${limit}` : null;
}
// What holds a scheduled check and the operator's next step. The backend names a phone
// condition only for rows its background verifier observes; anything else needs an action.
function verificationWait(c: PublishProgressCapability | undefined, due: number, now: number): string {
 if(!c)return "chưa đọc được trạng thái kiểm tra";
 const wait=typeof c.verificationWait?.message === "string" && c.verificationWait.message.trim() ? c.verificationWait.message.trim().slice(0,256) : null;
 const retry=typeof c.verificationWait?.retryAt === "string" && c.verificationWait.retryAt.length <= 64 ? Date.parse(c.verificationWait.retryAt) : NaN;
 if(due<=now&&wait)return Number.isFinite(retry)?`${wait} · tự thử lại lúc ${new Date(retry).toLocaleTimeString("vi-VN")}`:wait;
 const overdue=Math.floor((now-due)/60000);
 if(c.automaticVerification)
  return due>now?"app tự kiểm tra khi đến lịch"
   :overdue>=10?`đã quá lịch ${overdue} phút; app vẫn tự thử lại khi máy rảnh`
   :"app sẽ tự kiểm tra trong ít phút";
 if(c.resumeVerification.allowed)return "tự kiểm tra đang dừng — bấm Tiếp tục kiểm tra link";
 if(c.checkLink.allowed)return "bài này không tự kiểm tra — bấm Kiểm tra liên kết";
 return "chưa có lượt tự kiểm tra cho bài này";
}
function verificationSummary(evidence: Record<string, unknown>, now: number, c?: PublishProgressCapability): {text: string; budget: string} {
 const status=object(evidence.verificationStatus), budget=object(evidence.verificationBudget);
 const reason=typeof status.reason === "string" && status.reason.trim() ? status.reason.trim().slice(0,512) : null;
 const due=typeof status.nextCheckAt === "string" && status.nextCheckAt.length <= 64 ? Date.parse(status.nextCheckAt) : NaN;
 const text=status.state === "needsReview" ? `Cần xem xét${reason?` · ${reason}`:" · Chưa có lý do được lưu."}`
  : status.state === "pending" && Number.isFinite(due)
   ? `${due>now?"Chờ lịch kiểm tra từ":"Đã đến lịch kiểm tra"} ${new Date(due).toLocaleString("vi-VN")} · ${verificationWait(c,due,now)}; không đăng lại.`
   : "Chưa rõ lịch kiểm tra tiếp theo.";
 const ordinary=budgetPair(budget.noProgressObservations,budget.limit), metadata=budgetPair(budget.metadataObservations,budget.metadataLimit);
 return {text,budget:[ordinary?`Không có tiến triển ${ordinary}`:null,metadata?`Metadata ${metadata}`:null].filter(Boolean).join(" · ")};
}
type PublishProgressCapability = PublishRunCapability & {
 exclusion?: PublishExcludeResult | null;
};
export function PublishDeviceRecovery({campaignId,udid}:{campaignId:string;udid:string}) {
 const state=usePublishRunRead(campaignId);
 const readLog=useCallback(()=>operationDeviceLog(`publish:${campaignId}`,udid),[campaignId,udid]);
 const log=useMonitorRead(readLog,2000,["deviceTimeline",`publish:${campaignId}`,udid]);
 const [now,setNow]=useState(() => Date.now());
 useEffect(()=>{const timer=window.setInterval(()=>setNow(Date.now()),1000);return()=>clearInterval(timer);},[]);
 const excludeRequest=useRef<{id:string;revision:number;requestId:string}|null>(null);
 const [exclusions,setExclusions]=useState<Record<string,PublishExcludeResult>>({});
 const exclude=async(c:PublishRecoveryCapability)=>{
  if(flight.current){setMessage("Đang xử lý yêu cầu hiện tại của máy này.");return;}
  flight.current=true;setBusy(true);setMessage(null);
  try {
   if(excludeRequest.current?.id!==c.assignmentId||excludeRequest.current.revision!==c.revision)
    excludeRequest.current={id:c.assignmentId,revision:c.revision,requestId:crypto.randomUUID()};
   const result=await publishExcludeAssignment(c.assignmentId,c.revision,excludeRequest.current.requestId);
   setExclusions(current=>({...current,[c.assignmentId]:result}));
   // Render the revisioned receipt beside its assignment; the poll can supersede it.
   state.retry();
  }catch(error){setMessage(describeError(error));}finally{flight.current=false;setBusy(false);}
 };
 const roster=useMonitorRead(listDevices,2000,["publishRecoveryRoster"]);
 const online=roster.value?.some(d=>d.udid===udid&&["ready","connected"].includes(d.status))===true;
 const flight=useRef(false),request=useRef<{id:string;revision:number;requestId:string}|null>(null);
 const [busy,setBusy]=useState(false),[message,setMessage]=useState<string|null>(null);
 const act=async(c:PublishRecoveryCapability,kind:"retry"|"link"|"resume"|"sheet")=>{
  if(flight.current){setMessage("Đang xử lý yêu cầu hiện tại của máy này.");return;}flight.current=true;setBusy(true);setMessage(null);
  try {
   if(kind==="retry") {
    if(request.current?.id!==c.assignmentId||request.current.revision!==c.revision)request.current={id:c.assignmentId,revision:c.revision,requestId:crypto.randomUUID()};
    await publishRetryAssignment(c.assignmentId,true,c.revision,request.current!.requestId);
   }else if(kind==="sheet")await publishRetrySheetAssignment(c.assignmentId,c.revision);
   else if(kind==="resume")await publishResumeVerification(c.assignmentId,true,c.revision);
   else await publishCheckLinks(campaignId,udid);
   setMessage(kind==="retry"?"Đã nhận thử lại đúng một lần cho máy này.":"Đã yêu cầu kiểm tra bài đã gửi.");state.retry();
  }catch(e){setMessage(describeError(e));}finally{flight.current=false;setBusy(false);}
 };
 if(state.error)return <small role="status">Chưa đọc được quyền thử lại: {state.error}</small>;
 return <div className="run-device-recovery" onClick={e=>e.stopPropagation()}>
  {state.value?.assignments.filter(a=>a.udid===udid).map(a=>{
   const c=state.value?.capabilities.find(c=>c.assignmentId===a.id) as PublishProgressCapability | undefined;
   const receipt=exclusions[a.id];
   const durable=c?.exclusion?.assignmentId===a.id?c.exclusion:null;
   const exclusion=durable&&(!receipt||durable.revision>=receipt.revision)?durable:receipt;
   const evidence=readEvidence(a.evidenceJson);
   const failureCode=object(evidence.recoveryFailure).code;
   const accountCode = c?.recovery?.lastErrorCode ?? (typeof failureCode === "string" ? failureCode : null) ?? a.errorCode;
   const guidance = accountCode ? accountGuidance[accountCode] : undefined;
   const r=c?.recovery;const waiting=r&&["retryWaiting","waitingDevice"].includes(r.state)&&!a.effectIntent;
   const text=r?.state==="waitingDevice"?`Mất kết nối · chờ máy, còn ${Math.max(0,Math.ceil(((r.reconnectDeadline??0)-now)/1000))} giây`:r?.state==="retryWaiting"?`Thử lại ${r.retriesUsed}/${r.maxRetries} · ${stepLabel[r.step]??r.step}`:r?.state==="exhausted"?`Đã hết lượt tự thử · ${stepLabel[r.step]??r.step}`:null;
   const events=log.value?.entries.filter(event=>event.at&&Number.isFinite(Date.parse(event.at)))??[];
   const latest=events.at(-1);
   const step=events.filter(event=>event.action==="publishStep").at(-1);
   const active=["queued","preparing","ready","transferring","imported","posting","verifying"].includes(a.state);
   const end=active?now:latest?.at?Date.parse(latest.at):null;
   const elapsed=step?.at&&end!==null?Math.max(0,Math.floor((end-Date.parse(step.at))/1000)):null;
   const total=state.value?.createdAt&&end!==null?Math.max(0,Math.floor((end-Date.parse(state.value.createdAt))/1000)):null;
   // `checkLink.reason === "alreadyVerified"` only means a link-shaped URL is stored; the
   // verified layer comes from the shared outcome, which needs the backend's verified state.
   const outcome=postOutcome(a,{sheetRequired:c?.sheetRequired});
   const progress=publishDeviceProgress(a,log.value?.entries??[],{
    sheetRequired:c?.sheetRequired,
    exclusionState:exclusion?.state,
    recoveryStep:r&&["retryWaiting","waitingDevice","failed","exhausted","interrupted"].includes(r.state)?r.step:undefined,
   });
   const verification=verificationSummary(evidence,now,c);
   return <div key={a.id}>
    <ProgressBar label={`Tiến độ đăng trên máy ${udid} · bài ${a.ordinal+1}`} fraction={log.error&&!progress.done?null:progress.fraction}
      tone={progress.done?"done":progress.stopped?"failed":"run"}/>
    <small>{progress.stage} · {progress.completed}/{progress.total} bước đã xác nhận · {progressLabel(log.error&&!progress.done?null:progress.fraction)}</small>
    {outcome.post!=="notSent"&&<PostOutcomeLayers outcome={outcome} reason={!outcome.verified&&a.effectIntent&&a.state!=="succeeded"?verification.text:undefined}/>}
    {outcome.verified&&c?.sheetRequired===undefined&&!a.sheetDelivery&&<small>Chưa đọc được yêu cầu Sheet; chưa thể xác nhận hoàn tất.</small>}
    {(outcome.post==="notSent"||(!outcome.verified&&!a.effectIntent))&&<small>{step?logMessage(step):stepLabel[r?.step??""]??"Chờ mốc tiến trình"}</small>}
    {!outcome.verified&&a.effectIntent&&verification.budget&&<small>{verification.budget}</small>}
    {outcome.post!=="notSent"&&!outcome.verified&&<PostOutcomeEvidence outcome={outcome}/>}
    <small>{elapsed===null?"Chưa có thời gian bước":`Bước ${elapsed} giây`} · {total===null?"Chưa có tổng thời gian":`Tổng ${total} giây`}</small>
    {log.error&&<small role="status">Chưa cập nhật được mốc tiến trình: {log.error}</small>}
    {exclusion&&<small role="status">{exclusion.state==="excluded"?"Đã loại máy khỏi lượt này.":exclusion.state==="stopping"?"Đang dừng riêng máy này; chờ nhả máy.":"Máy còn bài cần đối chiếu; giữ nguyên nghĩa vụ xác minh."}{exclusion.reason&&` ${exclusion.reason}`}</small>}
    {c&&!["succeeded","cancelled","missed"].includes(a.state)&&<button type="button" disabled={busy||!!exclusion} title="Chỉ dừng máy này trong lượt đăng; giữ nguyên bài đã gửi và nghĩa vụ xác minh" onClick={()=>void exclude(c)}>Loại khỏi lượt này</button>}

    {!outcome.verified&&text&&<small role="status">{text}</small>}
    {!outcome.verified&&guidance&&<small role="status">{guidance}</small>}
    {!outcome.verified&&!guidance&&r?.lastError&&["failed","exhausted","interrupted"].includes(r.state)&&<small>{describeError(r.lastError)}</small>}
    {!c&&<small>Chưa đọc được quyền thao tác cho máy này.</small>}
    {c&&!a.effectIntent&&(a.state==="failedBeforeDispatch"||waiting)&&<><button type="button" disabled={busy||!!exclusion||!!waiting||!online||!c.retryBeforePost.allowed} title={!online?"Máy chưa kết nối":c.retryBeforePost.allowed?"Chạy thêm đúng một lần; giữ nguyên bài và máy":c.retryBeforePost.reason??"Đang phục hồi"} onClick={()=>void act(c,"retry")}>Thử lại</button>{!online&&<small>Máy chưa kết nối</small>}</>}
    {a.effectIntent&&c?.resumeVerification.allowed&&<button type="button" disabled={busy} onClick={()=>void act(c,"resume")}>Tiếp tục kiểm tra link</button>}
    {a.effectIntent&&c?.checkLink.allowed&&!c.resumeVerification.allowed&&<button type="button" disabled={busy} onClick={()=>void act(c,"link")}>Kiểm tra liên kết</button>}
    {outcome.verified&&a.sheetDelivery?.state==="failed"&&c&&<button type="button" disabled={busy} onClick={()=>void act(c,"sheet")}>Ghi lại Sheet</button>}
   </div>;
  })}
  {message&&<small role="status">{message}</small>}
 </div>;
}
