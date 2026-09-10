import { useEffect, useRef, useState } from "react";
import { automationList, automationScheduleList, automationScheduleFromSettings, automationScheduleUpdate } from "../api";
import type { AutomationKind, AutomationSchedule, AutomationScheduleV1, JsonValue, TargetRef } from "../types";
import { describeError } from "../describeError";

export function AutomationSettingsSchedule({ kind, target, config, disabled }: { kind: AutomationKind; target: TargetRef; config: JsonValue; disabled: boolean }) {
  const [rows, setRows] = useState<AutomationSchedule[]>([]);
  const [name, setName] = useState("");
  const [minutes, setMinutes] = useState(60);
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);
  const [message, setMessage] = useState("");
  const [editing, setEditing] = useState<Record<string, { name: string; minutes: number }>>({});
  useEffect(() => {
    let active = true;
    void Promise.all([automationList(true), automationScheduleList()]).then(([definitions, schedules]) => {
      const ids = new Set(definitions.filter(d => d.kind === kind).map(d => d.id));
      if (active) setRows(schedules.filter(s => ids.has(s.definitionId)));
    }).catch(e => { if (active) setMessage(describeError(e)); });
    return () => { active = false; };
  }, [kind]);
  const create = async () => {
    if (disabled || inFlight.current || !name.trim() || !Number.isInteger(minutes) || minutes < 15 || minutes > 1440) return;
    inFlight.current=true;setBusy(true);setMessage("");
    try { const row = await automationScheduleFromSettings(name.trim(), kind, target, config, { schemaVersion: 1, kind: "interval", everyMinutes: minutes }); setRows(r => [...r, row]); setName(""); setMessage("Đã lưu lịch với thiết lập hiện tại."); }
    catch(e) {setMessage(describeError(e));} finally {inFlight.current=false;setBusy(false);}
  };
  const toggle = async (row: AutomationSchedule) => {
    if(inFlight.current)return;inFlight.current=true;setBusy(true);setMessage("");
    try { const next = await automationScheduleUpdate(row.id,row.revision,row.name,row.definitionId,row.definitionRevision,!row.enabled,row.schedule as unknown as AutomationScheduleV1);setRows(r => r.map(s => s.id === next.id ? next : s)); }
    catch(e){setMessage(describeError(e));} finally {inFlight.current=false;setBusy(false);}
  };
  const revise = async (row: AutomationSchedule) => {
    const value=editing[row.id];
    if(!value||inFlight.current||!value.name.trim()||!Number.isInteger(value.minutes)||value.minutes<15||value.minutes>1440)return;
    inFlight.current=true;setBusy(true);setMessage("");
    try { const next=await automationScheduleUpdate(row.id,row.revision,value.name.trim(),row.definitionId,row.definitionRevision,row.enabled,{schemaVersion:1,kind:"interval",everyMinutes:value.minutes});setRows(r=>r.map(s=>s.id===next.id?next:s));setMessage("Đã lưu lịch; giữ nguyên thiết lập đã chụp."); }
    catch(e){setMessage(describeError(e));}finally{inFlight.current=false;setBusy(false);}
  };
  return <section className="automation-schedule-control" aria-label="Hẹn giờ từ thiết lập">
    <h3>Lịch tự động</h3><p>Lưu bài, hành động và máy đang chọn vào lịch. Sửa thiết lập sau đó không đổi lịch đã lưu.</p>
    <div className="automation-schedule-create"><label>Tên lịch<input aria-label="Tên lịch mới" value={name} disabled={busy} onChange={e=>setName(e.target.value)}/></label><label>Cách nhau (phút)<input aria-label="Chu kỳ lịch mới (phút)" type="number" min={15} max={1440} value={minutes} disabled={busy} onChange={e=>setMinutes(Number(e.target.value))}/></label><button type="button" disabled={busy||disabled||!name.trim()||!Number.isInteger(minutes)||minutes<15||minutes>1440} onClick={()=>void create()}>Lưu lịch từ thiết lập</button></div>
    {rows.map(row=>{const current=editing[row.id]??{name:row.name,minutes:Number((row.schedule as unknown as {everyMinutes:number}).everyMinutes)};return <div className="automation-schedule-row" key={row.id}><label>Tên lịch<input aria-label={`Tên lịch ${row.name}`} value={current.name} disabled={busy} onChange={e=>setEditing(v=>({...v,[row.id]:{...current,name:e.target.value}}))}/></label><label>Chu kỳ<input aria-label={`Chu kỳ ${row.name}`} type="number" min={15} max={1440} value={current.minutes} disabled={busy} onChange={e=>setEditing(v=>({...v,[row.id]:{...current,minutes:Number(e.target.value)}}))}/></label><span>{row.enabled?"Đang bật":"Đang tắt"}</span><button type="button" disabled={busy||!editing[row.id]} onClick={()=>void revise(row)}>Lưu thay đổi</button><button type="button" disabled={busy} onClick={()=>void toggle(row)}>{row.enabled?"Tắt lịch":"Bật lịch"}</button><small>{row.nextDueAt ? new Date(row.nextDueAt).toLocaleString("vi-VN") : "Chưa có lần chạy tiếp"}</small></div>;})}
    {message&&<p role="status">{message}</p>}
  </section>;
}
