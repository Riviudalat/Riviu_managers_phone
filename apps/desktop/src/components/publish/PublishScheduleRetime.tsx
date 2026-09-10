import { useState } from "react";
import type { PublishCampaignRecord } from "../../types";
import { publishScheduleReschedule } from "../../api";
import { describeError } from "../../describeError";
import { localDateTime, scheduleDateIssue } from "./publishScheduleTimes";

export function PublishScheduleRetime({ campaign, onSaved }: { campaign: PublishCampaignRecord; onSaved: () => void }) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(campaign.runAt?.slice(0,16) ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const save = async () => {
    if (busy) return;
    const reason = scheduleDateIssue(value.slice(0,10),value.slice(11),Date.now());
    if (reason) { setError(reason); return; }
    setBusy(true); setError("");
    try { await publishScheduleReschedule(campaign.id,campaign.updatedAt,value); setEditing(false); onSaved(); }
    catch (e) { setError(describeError(e)); }
    finally { setBusy(false); }
  };
  if (campaign.state !== "scheduled") return null;
  return <div className="publish-schedule-retime">
    {!editing ? <button type="button" onClick={() => { setValue(campaign.runAt?.slice(0,16) ?? ""); setEditing(true); }}>Đổi giờ</button> : <>
      <label>Giờ bắt đầu mới<input type="datetime-local" aria-label="Giờ bắt đầu mới" min={localDateTime(new Date())} value={value} disabled={busy} onChange={event=>setValue(event.target.value)}/></label>
      <button type="button" disabled={busy || !value} onClick={()=>void save()}>{busy ? "Đang lưu…" : "Lưu giờ mới"}</button>
      <button type="button" disabled={busy} onClick={()=>setEditing(false)}>Bỏ thay đổi</button>
      {error && <p role="alert">{error}</p>}
    </>}
  </div>;
}
