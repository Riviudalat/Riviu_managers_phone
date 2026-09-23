import {useCallback,useRef,useState} from "react";
import {publishGet,publishRecoveryCapabilities,publishRetryAssignment,publishCheckLinks,publishResumeVerification,publishRetrySheetAssignment,listDevices} from "../../api";
import {useMonitorRead} from "./useMonitorRead";
import {describeError} from "../../describeError";
import type {PublishRecoveryCapability} from "../../types";

const stepLabel:Record<string,string>={device:"Chuẩn bị máy",transfer:"Chuyển nội dung",media:"Chọn ảnh/video",sound:"Chọn nhạc",caption:"Nhập caption"};
export function PublishDeviceRecovery({campaignId,udid}:{campaignId:string;udid:string}) {
 const read=useCallback(async()=>{const [detail,capabilities]=await Promise.all([publishGet(campaignId),publishRecoveryCapabilities(campaignId)]);return {assignments:detail?.assignments??[],capabilities};},[campaignId]);
 const state=useMonitorRead(read,2000,["publishRecovery",campaignId]);
 const roster=useMonitorRead(listDevices,2000,["publishRecoveryRoster"]);
 const online=roster.value?.some(d=>d.udid===udid&&["ready","connected"].includes(d.status))===true;
 const flight=useRef(false),request=useRef<{id:string;revision:number;requestId:string}|null>(null);
 const [busy,setBusy]=useState(false),[message,setMessage]=useState<string|null>(null);
 const act=async(c:PublishRecoveryCapability,kind:"retry"|"link"|"resume"|"sheet")=>{
  if(flight.current)return;flight.current=true;setBusy(true);setMessage(null);
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
   const c=state.value?.capabilities.find(c=>c.assignmentId===a.id);if(!c)return null;
   const r=c.recovery;const waiting=r&&["retryWaiting","waitingDevice"].includes(r.state)&&!a.effectIntent;
   const text=r?.state==="waitingDevice"?`Mất kết nối · chờ máy, còn ${Math.max(0,Math.ceil(((r.reconnectDeadline??0)-Date.now())/1000))} giây`:r?.state==="retryWaiting"?`Thử lại ${r.retriesUsed}/${r.maxRetries} · ${stepLabel[r.step]??r.step}`:r?.state==="exhausted"?`Đã hết lượt tự thử · ${stepLabel[r.step]??r.step}`:null;
   return <div key={a.id}>
    {text&&<small role="status">{text}</small>}
    {r?.lastError&&["failed","exhausted","interrupted"].includes(r.state)&&<small>{describeError(r.lastError)}</small>}
    {!a.effectIntent&&(a.state==="failedBeforeDispatch"||waiting)&&<><button type="button" disabled={busy||!!waiting||!online||!c.retryBeforePost.allowed} title={!online?"Máy chưa kết nối":c.retryBeforePost.allowed?"Chạy thêm đúng một lần; giữ nguyên bài và máy":c.retryBeforePost.reason??"Đang phục hồi"} onClick={()=>void act(c,"retry")}>Thử lại</button>{!online&&<small>Máy chưa kết nối</small>}</>}
    {a.effectIntent&&c.resumeVerification.allowed&&<button type="button" disabled={busy} onClick={()=>void act(c,"resume")}>Tiếp tục kiểm tra link</button>}
    {a.effectIntent&&c.checkLink.allowed&&!c.resumeVerification.allowed&&<button type="button" disabled={busy} onClick={()=>void act(c,"link")}>Kiểm tra liên kết</button>}
    {a.effectIntent&&a.sheetDelivery?.state==="failed"&&c.checkLink.reason==="alreadyVerified"&&<button type="button" disabled={busy} onClick={()=>void act(c,"sheet")}>Ghi lại Sheet</button>}
   </div>;
  })}
  {message&&<small role="status">{message}</small>}
 </div>;
}
