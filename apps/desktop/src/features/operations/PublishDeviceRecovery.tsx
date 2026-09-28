import {useCallback,useEffect,useRef,useState} from "react";
import {operationDeviceLog,publishExcludeAssignment,publishGet,publishRecoveryCapabilities,publishRetryAssignment,publishCheckLinks,publishResumeVerification,publishRetrySheetAssignment,listDevices} from "../../api";
import {useMonitorRead} from "./useMonitorRead";
import {describeError} from "../../describeError";
import {logMessage,publishDeviceProgress,progressLabel} from "./operationProgress";
import {ProgressBar} from "../../components/ProgressBar";
import type {PublishExcludeResult,PublishRecoveryCapability} from "../../types";

const stepLabel:Record<string,string>={device:"Chuẩn bị máy",transfer:"Chuyển nội dung",media:"Chọn ảnh/video",sound:"Chọn nhạc",caption:"Nhập caption"};
type PublishProgressCapability = PublishRecoveryCapability & {
 exclusion?: PublishExcludeResult | null;
 sheetRequired?: boolean;
};
export function PublishDeviceRecovery({campaignId,udid}:{campaignId:string;udid:string}) {
 const read=useCallback(async()=>{const [detail,capabilities]=await Promise.all([publishGet(campaignId),publishRecoveryCapabilities(campaignId)]);return {assignments:detail?.assignments??[],capabilities,createdAt:detail?.campaign.createdAt};},[campaignId]);
 const state=useMonitorRead(read,2000,["publishRecovery",campaignId]);
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
   const r=c?.recovery;const waiting=r&&["retryWaiting","waitingDevice"].includes(r.state)&&!a.effectIntent;
   const text=r?.state==="waitingDevice"?`Mất kết nối · chờ máy, còn ${Math.max(0,Math.ceil(((r.reconnectDeadline??0)-now)/1000))} giây`:r?.state==="retryWaiting"?`Thử lại ${r.retriesUsed}/${r.maxRetries} · ${stepLabel[r.step]??r.step}`:r?.state==="exhausted"?`Đã hết lượt tự thử · ${stepLabel[r.step]??r.step}`:null;
   const events=log.value?.entries.filter(event=>event.at&&Number.isFinite(Date.parse(event.at)))??[];
   const latest=events.at(-1);
   const step=events.filter(event=>event.action==="publishStep").at(-1);
   const active=["queued","preparing","ready","transferring","imported","posting","verifying"].includes(a.state);
   const end=active?now:latest?.at?Date.parse(latest.at):null;
   const elapsed=step?.at&&end!==null?Math.max(0,Math.floor((end-Date.parse(step.at))/1000)):null;
   const total=state.value?.createdAt&&end!==null?Math.max(0,Math.floor((end-Date.parse(state.value.createdAt))/1000)):null;
   const progress=publishDeviceProgress(a,log.value?.entries??[],{
    sheetRequired:c?.sheetRequired,
    publicationVerified:c?.checkLink.reason==="alreadyVerified",
    exclusionState:exclusion?.state,
    recoveryStep:r&&["retryWaiting","waitingDevice","failed","exhausted","interrupted"].includes(r.state)?r.step:undefined,
   });
   return <div key={a.id}>
    <ProgressBar label={`Tiến độ đăng trên máy ${udid} · bài ${a.ordinal+1}`} fraction={log.error&&!progress.done?null:progress.fraction}
      tone={progress.done?"done":progress.stopped?"failed":"run"}/>
    <small>{progress.stage} · {progress.completed}/{progress.total} bước đã xác nhận · {progressLabel(log.error&&!progress.done?null:progress.fraction)}</small>
    {progress.uncertain&&<small>Đã gửi hoặc đang xác minh bài; chưa đủ bằng chứng để tính hoàn tất.</small>}
    {progress.verified&&!progress.done&&!progress.stopped&&<small>{c?.sheetRequired===undefined&&!a.sheetDelivery?"Chưa đọc được yêu cầu Sheet; chưa thể xác nhận hoàn tất.":"Bài đã xác minh · chờ hoàn tất Sheet."}</small>}
    <small>{step?logMessage(step):stepLabel[r?.step??""]??"Chờ mốc tiến trình"}</small>
    <small>{elapsed===null?"Chưa có thời gian bước":`Bước ${elapsed} giây`} · {total===null?"Chưa có tổng thời gian":`Tổng ${total} giây`}</small>
    {log.error&&<small role="status">Chưa cập nhật được mốc tiến trình: {log.error}</small>}
    {exclusion&&<small role="status">{exclusion.state==="excluded"?"Đã loại máy khỏi lượt này.":exclusion.state==="stopping"?"Đang dừng riêng máy này; chờ nhả máy.":"Máy còn bài cần đối chiếu; giữ nguyên nghĩa vụ xác minh."}{exclusion.reason&&` ${exclusion.reason}`}</small>}
    {c&&!["succeeded","cancelled","missed"].includes(a.state)&&<button type="button" disabled={busy||!!exclusion} title="Chỉ dừng máy này trong lượt đăng; giữ nguyên bài đã gửi và nghĩa vụ xác minh" onClick={()=>void exclude(c)}>Loại khỏi lượt này</button>}

    {text&&<small role="status">{text}</small>}
    {r?.lastError&&["failed","exhausted","interrupted"].includes(r.state)&&<small>{describeError(r.lastError)}</small>}
    {!c&&<small>Chưa đọc được quyền thao tác cho máy này.</small>}
    {c&&!a.effectIntent&&(a.state==="failedBeforeDispatch"||waiting)&&<><button type="button" disabled={busy||!!exclusion||!!waiting||!online||!c.retryBeforePost.allowed} title={!online?"Máy chưa kết nối":c.retryBeforePost.allowed?"Chạy thêm đúng một lần; giữ nguyên bài và máy":c.retryBeforePost.reason??"Đang phục hồi"} onClick={()=>void act(c,"retry")}>Thử lại</button>{!online&&<small>Máy chưa kết nối</small>}</>}
    {a.effectIntent&&c?.resumeVerification.allowed&&<button type="button" disabled={busy} onClick={()=>void act(c,"resume")}>Tiếp tục kiểm tra link</button>}
    {a.effectIntent&&c?.checkLink.allowed&&!c.resumeVerification.allowed&&<button type="button" disabled={busy} onClick={()=>void act(c,"link")}>Kiểm tra liên kết</button>}
    {a.effectIntent&&a.sheetDelivery?.state==="failed"&&c?.checkLink.reason==="alreadyVerified"&&<button type="button" disabled={busy} onClick={()=>void act(c,"sheet")}>Ghi lại Sheet</button>}
   </div>;
  })}
  {message&&<small role="status">{message}</small>}
 </div>;
}
