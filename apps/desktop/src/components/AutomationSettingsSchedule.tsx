import { useEffect, useMemo, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { automationList, automationScheduleList, automationScheduleFromSettings, automationScheduleUpdate } from "../api";
import type { AutomationKind, AutomationSchedule, AutomationScheduleV1, DeviceInfo, DeviceMeta, JsonValue, TargetRef } from "../types";
import { describeError } from "../describeError";
import { scheduleDeviceLabel, scheduleErrorLabel, scheduleScope } from "../scheduleScope";
import { NextRunsPreview, ScheduleScopeNotice } from "./ScheduleReview";
import { confirmScheduleApproval, needsGroups, useDeviceGroups, useScheduleTargets } from "../useScheduleScope";

const validMinutes = (minutes: number) => Number.isInteger(minutes) && minutes >= 15 && minutes <= 1440;
const interval = (everyMinutes: number): AutomationScheduleV1 => ({ schemaVersion: 1, kind: "interval", everyMinutes });

export function AutomationSettingsSchedule({ kind, target, config, disabled, devices = [], metas }: {
  kind: AutomationKind; target: TargetRef; config: JsonValue; disabled: boolean;
  /** The roster the approved device list is resolved against. */
  devices?: DeviceInfo[]; metas?: Map<string, DeviceMeta>;
}) {
  const [rows, setRows] = useState<AutomationSchedule[]>([]);
  const [name, setName] = useState("");
  const [minutes, setMinutes] = useState(60);
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);
  const [message, setMessage] = useState("");
  const [editing, setEditing] = useState<Record<string, { name: string; minutes: number }>>({});
  // Bumped by "Tải lại lịch": the rows and the group membership are read again together.
  const [refreshCount, setRefreshCount] = useState(0);
  const targets = useScheduleTargets(rows);
  const { groups, error: groupsError, reload: reloadGroups } = useDeviceGroups(target.type === "group" || needsGroups(targets.values()), refreshCount);
  const label = useMemo(() => (udid: string) => scheduleDeviceLabel(udid, devices, metas), [devices, metas]);
  // Saving approves exactly the phones the current setup names.
  const newScope = scheduleScope(null, target, devices, groups);
  const approvable = newScope.kind === "unapproved" ? newScope.designated : [];
  useEffect(() => {
    let active = true;
    void Promise.all([automationList(true), automationScheduleList()]).then(([definitions, schedules]) => {
      const ids = new Set(definitions.filter(d => d.kind === kind).map(d => d.id));
      if (active) setRows(schedules.filter(s => ids.has(s.definitionId)));
    }).catch(e => { if (active) setMessage(describeError(e)); });
    return () => { active = false; };
  }, [kind, refreshCount]);
  const create = async (enabled: boolean) => {
    if (disabled || inFlight.current || !name.trim() || !validMinutes(minutes)) return;
    if (newScope.kind === "unknown") { setMessage(groupsError ? `Chưa đọc được nhóm máy: ${groupsError}. Bấm Tải lại lịch tự động.` : "Đang đọc phạm vi máy; thử lại sau giây lát."); return; }
    if (!approvable.length) { setMessage("Phạm vi hiện không có máy nào để duyệt cho lịch."); return; }
    inFlight.current=true;setBusy(true);setMessage("");
    try {
      const row = await automationScheduleFromSettings(name.trim(), kind, target, config, interval(minutes), enabled, approvable);
      setRows(r => [...r, row]); setName("");
      setMessage(`${enabled ? "Đã lưu và bật lịch" : "Đã lưu lịch, đang tắt"} với thiết lập hiện tại · duyệt ${approvable.length} máy.`);
    }
    catch(e) {setMessage(describeError(e));} finally {inFlight.current=false;setBusy(false);}
  };
  const write = async (row: AutomationSchedule, values: { name: string; enabled: boolean; schedule: AutomationScheduleV1; approvedUdids: string[] | null }, done: string) => {
    if(inFlight.current)return;inFlight.current=true;setBusy(true);setMessage("");
    try { const next=await automationScheduleUpdate(row.id,row.revision,values.name,row.definitionId,row.definitionRevision,values.enabled,values.schedule,values.approvedUdids);setRows(r=>r.map(s=>s.id===next.id?next:s));if(done)setMessage(done); }
    catch(e){setMessage(describeError(e));}finally{inFlight.current=false;setBusy(false);}
  };
  const toggle = (row: AutomationSchedule) => write(row, { name: row.name, enabled: !row.enabled, schedule: row.schedule as unknown as AutomationScheduleV1, approvedUdids: null }, "");
  const revise = async (row: AutomationSchedule) => {
    const value=editing[row.id];
    if(!value||!value.name.trim()||!validMinutes(value.minutes))return;
    await write(row, { name: value.name.trim(), enabled: row.enabled, schedule: interval(value.minutes), approvedUdids: null }, "Đã lưu lịch; giữ nguyên thiết lập đã chụp.");
  };
  const approve = async (row: AutomationSchedule, udids: string[]) => {
    if (!(await confirmScheduleApproval(row.name, udids, label))) return;
    await write(row, { name: row.name, enabled: row.enabled, schedule: row.schedule as unknown as AutomationScheduleV1, approvedUdids: udids }, `Đã duyệt ${udids.length} máy cho lịch ${row.name}.`);
  };
  return <section className="automation-schedule-control" aria-label="Hẹn giờ từ thiết lập">
    <div className="automation-schedule-heading"><h3>Lịch tự động</h3><button type="button" className="icon-btn" aria-label="Tải lại lịch tự động" title="Đọc lại lịch và nhóm máy" disabled={busy} onClick={() => setRefreshCount(value => value + 1)}><RefreshCw size={15} /></button></div><p>Lưu bài, hành động và máy đang chọn vào lịch. Sửa thiết lập sau đó không đổi lịch đã lưu.</p>
    <div className="automation-schedule-create" role="group" aria-label="Tạo lịch mới">
      <label>Tên lịch<input aria-label="Tên lịch mới" value={name} disabled={busy} onChange={e=>setName(e.target.value)}/></label>
      <label>Cách nhau (phút)<input aria-label="Chu kỳ lịch mới (phút)" type="number" min={15} max={1440} value={minutes} disabled={busy} onChange={e=>setMinutes(Number(e.target.value))}/></label>
      <div className="schedule-save-actions">
        <button type="button" aria-label="Lưu lịch, chưa bật" disabled={busy||disabled||!name.trim()||!validMinutes(minutes)} onClick={()=>void create(false)}>Lưu</button>
        <button type="button" className="primary" aria-label="Lưu & bật lịch" disabled={busy||disabled||!name.trim()||!validMinutes(minutes)} onClick={()=>void create(true)}>Lưu & bật</button>
      </div>
    </div>
    {newScope.kind === "unapproved" && <p className="schedule-scope-line">Lưu sẽ duyệt {approvable.length} máy: {approvable.map(label).join(", ") || "không có máy nào"}. Nhóm đổi sau đó sẽ dừng lịch tới khi duyệt lại.</p>}
    {newScope.kind === "missingGroup" && <p className="schedule-scope-line" role="alert">Nhóm máy đang chọn không còn; chọn phạm vi khác trước khi lưu lịch.</p>}
    <NextRunsPreview nextDueAt={null} everyMinutes={minutes} enabled={false} />
    {rows.map(row=>{
      const storedMinutes=Number((row.schedule as unknown as {everyMinutes:number}).everyMinutes);
      const current=editing[row.id]??{name:row.name,minutes:storedMinutes};
      const scope=scheduleScope(row.approvedUdids, targets.get(`${row.definitionId}:${row.definitionRevision}`), devices, groups);
      // Switching on waits for a scope that matches the approval; switching off never does.
      const enableBlocked=!row.enabled&&scope.kind!=="approved";
      const lastError=scheduleErrorLabel(row.lastErrorCode);
      return <div className="automation-schedule-row" key={row.id} role="group" aria-label={`Lịch ${row.name}`}>
        <div className="automation-schedule-row-main">
          <label>Tên lịch<input aria-label={`Tên lịch ${row.name}`} value={current.name} disabled={busy} onChange={e=>setEditing(v=>({...v,[row.id]:{...current,name:e.target.value}}))}/></label>
          <label>Chu kỳ<input aria-label={`Chu kỳ ${row.name}`} type="number" min={15} max={1440} value={current.minutes} disabled={busy} onChange={e=>setEditing(v=>({...v,[row.id]:{...current,minutes:Number(e.target.value)}}))}/></label>
          <span className={`automation-schedule-state ${row.enabled ? "enabled" : "disabled"}`}>{row.enabled?"Đang bật":"Đang tắt"}</span>
          <button type="button" disabled={busy||!editing[row.id]} onClick={()=>void revise(row)}>Lưu thay đổi</button>
          <button type="button" disabled={busy||enableBlocked} title={enableBlocked?"Duyệt danh sách máy trước khi bật":undefined} onClick={()=>void toggle(row)}>{row.enabled?"Tắt lịch":"Bật lịch"}</button>
        </div>
        <ScheduleScopeNotice scope={scope} approved={row.approvedUdids} label={label} busy={busy} onApprove={udids=>void approve(row, udids)} error={groupsError} onRetry={reloadGroups}/>
        {lastError&&<p className="hint">Lần gần nhất: {lastError}</p>}
        <NextRunsPreview nextDueAt={row.nextDueAt} everyMinutes={storedMinutes} enabled={row.enabled} compact/>
      </div>;
    })}
    {message&&<p role="status">{message}</p>}
  </section>;
}
