import { useCallback, useEffect, useState } from "react";
import { inspectorObserve, inspectorTap, inspectorRecord, inspectorRecording } from "../inspectorApi";
import { Scan, RefreshCw, Circle, Square, Save, X } from "lucide-react";
import { createPortal } from "react-dom";
import { useModalFocus } from "./useModalFocus";
import { useClosingTransition } from "./useClosingTransition";
import { describeError } from "../describeError";
import { flowSaveRevision } from "../api";
import { newFlowDocument, createFlowNode } from "../flow/model";
import type { JsonObject } from "../types";
import "./device-inspector.css";
type Selector={package:string;text?:string|null;description?:string|null;resourceId?:string|null;className?:string|null};
type Element={index:number;parent:number|null;text:string;description:string;resourceId:string;className:string;x:number;y:number;width:number;height:number;clickable:boolean;enabled:boolean;selector:Selector|null};
type Snapshot={id:string;udid:string;package:string;version:string;locale:string;width:number;height:number;pngBase64:string;treeSha256:string;elements:Element[]};
type Recording={id:string;udid:string;name:string;active:boolean;steps:{selector:Selector;expected?:Selector|null;beforeId:string;afterId:string;verified:boolean;error:string|null}[]};
export function DeviceInspector({udid,onClose}:{udid:string;onClose:()=>void}){
 const {closing,close}=useClosingTransition(onClose);const ref=useModalFocus<HTMLDivElement>(close);
 const [snapshot,setSnapshot]=useState<Snapshot|null>(null);const [selected,setSelected]=useState<number|null>(null);const [hover,setHover]=useState<number|null>(null);const [record,setRecord]=useState<Recording|null>(null);const [busy,setBusy]=useState(false);const [error,setError]=useState('');const [name,setName]=useState('Quy trình mới');const [saved,setSaved]=useState('');
 const refreshRecord=useCallback(async()=>setRecord(await inspectorRecording(udid)),[udid]);
 const observe=useCallback(async()=>{setBusy(true);setError('');try{setSnapshot(await inspectorObserve(udid));setSelected(null);setHover(null);await refreshRecord();}catch(e){setError(describeError(e));}finally{setBusy(false);}},[udid,refreshRecord]);
 useEffect(()=>{void observe();},[observe]);
 const element=snapshot?.elements.find(e=>e.index===selected);const outlined=snapshot?.elements.find(e=>e.index===(hover??selected));
 const hit=(event:React.PointerEvent<SVGSVGElement>)=>{if(!snapshot)return null;const rect=event.currentTarget.getBoundingClientRect();const x=(event.clientX-rect.left)*snapshot.width/rect.width,y=(event.clientY-rect.top)*snapshot.height/rect.height;return snapshot.elements.filter(e=>x>=e.x&&x<=e.x+e.width&&y>=e.y&&y<=e.y+e.height).sort((a,b)=>(a.width*a.height)-(b.width*b.height))[0]?.index??null;};
 const toggleRecord=async()=>{setError('');try{setRecord(await inspectorRecord(udid,name,!record?.active));setSaved('');}catch(e){setError(describeError(e));}};
 const tap=async()=>{if(!element?.selector)return;setBusy(true);setError('');try{setSnapshot(await inspectorTap(udid,element.selector));setSelected(null);}catch(e){setError(describeError(e));}finally{await refreshRecord();setBusy(false);}};
 const save=async()=>{if(!record||record.active||!record.steps.length)return;setBusy(true);setError('');try{
  if(record.steps.some(s=>!s.verified||!s.expected))throw Error('Có bước chưa xác minh phần tử kết quả. Ghi lại quy trình sau khi kiểm tra bước đó.');
  const doc=newFlowDocument(record.name);const start=doc.nodes.find(n=>n.kind==='start')!;const end=doc.nodes.find(n=>n.kind==='end')!;
  const launch=createFlowNode('launchApp',{x:220,y:100});launch.config={bundleId:record.steps[0].selector.package};launch.postcondition={kind:'activeAppEquals',bundleId:record.steps[0].selector.package};
  const taps=record.steps.map((step,i)=>{const node=createFlowNode('tap',{x:440+i*220,y:100});node.config={selector:step.selector as unknown as JsonObject};node.postcondition={kind:'elementVisible',selector:step.expected!};return node;});
  end.position={x:440+taps.length*220,y:100};doc.nodes=[start,launch,...taps,end];doc.edges=doc.nodes.slice(0,-1).map((n,i)=>({id:crypto.randomUUID(),sourceNodeId:n.id,sourcePort:'flow',targetNodeId:doc.nodes[i+1].id,targetPort:'flow'}));
  const revision=await flowSaveRevision(doc,null);setSaved(`Đã lưu Flow “${revision.document.name}”. Mở Flow thiết bị để chỉnh và chạy.`);
 }catch(e){setError(describeError(e));}finally{setBusy(false);}};
 return createPortal(<div className={`modal-backdrop inspector-backdrop${closing?' is-closing':''}`}><div ref={ref} tabIndex={-1} className="modal device-inspector" role="dialog" aria-label="Bắt thuộc tính và ghi Flow" aria-modal="true">
  <header><div><h2><Scan size={18}/>Bắt thuộc tính & ghi Flow</h2><small>{snapshot?`${snapshot.package} · ${snapshot.version} · ${snapshot.locale}`:udid}</small></div><button className="icon-btn" aria-label="Đóng Inspector" onClick={close}><X size={18}/></button></header>
  <div className="inspector-toolbar"><input aria-label="Tên quy trình" value={name} disabled={record?.active} onChange={e=>setName(e.target.value)}/><button disabled={busy} onClick={()=>void observe()}><RefreshCw size={15}/>Đọc lại</button><button disabled={busy} onClick={()=>void toggleRecord()}>{record?.active?<Square size={14}/>:<Circle size={14}/>} {record?.active?'Dừng ghi':'Bắt đầu ghi'}</button><button disabled={busy||!record||record.active||!record.steps.length} onClick={()=>void save()}><Save size={15}/>Lưu thành Flow</button></div>
  {error&&<p role="alert">{error}</p>}{saved&&<p role="status">{saved}</p>}
  <div className="inspector-body"><div className="inspector-screen">{snapshot?<svg viewBox={`0 0 ${snapshot.width} ${snapshot.height}`} onPointerMove={e=>setHover(hit(e))} onPointerLeave={()=>setHover(null)} onClick={()=>setSelected(hover)} aria-label="Chọn phần tử trên ảnh">
   <image href={`data:image/png;base64,${snapshot.pngBase64}`} width={snapshot.width} height={snapshot.height}/>
   {outlined&&<rect x={outlined.x} y={outlined.y} width={outlined.width} height={outlined.height} fill="rgba(194,65,12,.15)" stroke="#c2410c" strokeWidth={4}/>}
  </svg>:<p>{busy?'Đang đọc màn hình…':'Chưa có ảnh'}</p>}</div>
  <div className="inspector-properties"><h3>Phần tử trên màn hình</h3><div className="inspector-elements">{snapshot?.elements.filter(e=>e.text||e.description||e.clickable).map(e=><button key={e.index} className={selected===e.index?'active':''} onClick={()=>setSelected(e.index)}>{e.text||e.description||e.resourceId||e.className}<small>{e.clickable?'Có thể bấm':'Nội dung'}</small></button>)}</div>
  {element&&<><dl>{[['Chữ',element.text],['Mô tả',element.description],['ID',element.resourceId],['Loại',element.className]].map(([label,value])=><div key={label}><dt>{label}</dt><dd>{value||'—'}</dd></div>)}</dl><p>{element.selector?'Tìm đúng 1 phần tử bằng thuộc tính':'Chưa có quy tắc tìm duy nhất'}</p><button className="primary" disabled={busy||!element.clickable||!element.selector} onClick={()=>void tap()}>Bấm và kiểm tra kết quả</button><details><summary>Selector</summary><pre>{JSON.stringify(element.selector,null,2)}</pre></details></>}
  </div></div><footer>{busy?'Đang thao tác…':`${record?.steps.length??0} bước đã ghi`} · {record?.active?'Đang ghi':'Đã dừng'}<span>Chọn phần tử chỉ xem thuộc tính; bấm thử là thao tác riêng.</span></footer>
 </div></div>,document.body);
}
