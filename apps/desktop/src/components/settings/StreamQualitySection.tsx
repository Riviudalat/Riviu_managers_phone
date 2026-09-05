import { useCallback, useEffect, useRef, useState } from "react";
import { Save } from "lucide-react";
import { getStreamSettings, setStreamSettings } from "../../api";
import { describeError } from "../../describeError";
import type { StreamSettings } from "../../types";
import { useWorkspaceDraft } from "../../workspaceDraft";
import { LoadingState, StatusNotice } from "../States";

const MIN_STREAM_FPS = 5;
const MAX_STREAM_FPS = 30;
const TILE_FPS_CEILING = 10;

type StreamDraft = Omit<StreamSettings, "fps"> & { fps: string };
const draftOf = (settings: StreamSettings): StreamDraft => ({ ...settings, fps: String(settings.fps) });

export function StreamQualitySection() {
  const [saved, setSaved] = useState<StreamSettings | null>(null);
  const [draft, setDraft] = useState<StreamDraft | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const epoch = useRef(0);
  const request = useRef(0);
  const savingRef = useRef(false);
  const dirty = saved !== null && draft !== null && JSON.stringify(draft) !== JSON.stringify(draftOf(saved));

  const load = useCallback(() => {
    const ticket = ++request.current;
    const editEpoch = epoch.current;
    setError(null);
    void getStreamSettings().then((next) => {
      if (ticket !== request.current || editEpoch !== epoch.current) return;
      setSaved(next);
      setDraft(draftOf(next));
    }).catch((cause) => {
      if (ticket === request.current) setError(describeError(cause));
    });
  }, []);
  useEffect(load, [load]);

  const patch = (change: Partial<StreamDraft>) => {
    epoch.current += 1;
    setDraft((current) => current && { ...current, ...change });
    setNotice(null);
  };
  const discard = () => {
    epoch.current += 1;
    setDraft(saved && draftOf(saved));
    setError(null);
    setNotice(null);
  };
  const save = async () => {
    if (!draft || savingRef.current) return false;
    const fps = Number(draft.fps);
    if (!draft.fps.trim() || !Number.isInteger(fps) || fps < MIN_STREAM_FPS || fps > MAX_STREAM_FPS) {
      setError(`FPS phải là số nguyên từ ${MIN_STREAM_FPS} đến ${MAX_STREAM_FPS}.`);
      return false;
    }
    const editEpoch = epoch.current;
    savingRef.current = true;
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const next = await setStreamSettings({ ...draft, fps });
      setSaved(next);
      if (epoch.current === editEpoch) {
        setDraft(draftOf(next));
        setNotice("Đã áp dụng chất lượng hình.");
      }
      return epoch.current === editEpoch;
    } catch (cause) {
      setError(describeError(cause));
      return false;
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  };
  useWorkspaceDraft({ id: "settings-stream", label: "Chất lượng hình", dirty, snapshotKey: JSON.stringify(draft), save, discard });

  return (
    <section className="settings-section" aria-label="Chất lượng stream">
      <h3>Chất lượng stream</h3>
      <p className="hint">Áp dụng sẽ khởi động lại hình Android đang chạy.</p>
      <details className="settings-details" aria-label="Phạm vi chất lượng hình">
        <summary>Phạm vi chất lượng hình</summary>
        <p className="hint">Tile trong lưới bị chặn ở {TILE_FPS_CEILING} hình/giây. FPS bên dưới áp cho máy mở lớn; thiết lập không đổi stream iOS.</p>
      </details>
      {!draft && !error && <LoadingState label="Đang đọc chất lượng hình…" />}
      {draft && <div className="row">
        {(["gridQuality", "focusQuality"] as const).map((field) => <label key={field}>
          {field === "gridQuality" ? "Chất lượng lưới" : "Chất lượng overlay"}
          <select value={draft[field]} onChange={(event) => patch({ [field]: event.target.value as StreamSettings[typeof field] })}>
            <option value="low">Thấp</option><option value="medium">Vừa</option><option value="high">Cao</option><option value="extra">Rất cao</option>
          </select>
        </label>)}
        <label>FPS overlay<input type="number" min={MIN_STREAM_FPS} max={MAX_STREAM_FPS} value={draft.fps} onChange={(event) => patch({ fps: event.target.value })} /></label>
      </div>}
      {error && <StatusNotice tone="error" action={!draft ? <button type="button" className="ghost" onClick={load}>Đọc lại</button> : undefined}>{error}</StatusNotice>}
      {notice && <StatusNotice tone="success">{notice}</StatusNotice>}
      <div className="row">
        <button type="button" className="primary" disabled={!dirty || saving} onClick={() => void save()}><Save size={15} />{saving ? "Đang áp dụng…" : "Áp dụng chất lượng hình"}</button>
        {dirty && <button type="button" className="ghost" disabled={saving} onClick={discard}>Bỏ thay đổi</button>}
      </div>
    </section>
  );
}
