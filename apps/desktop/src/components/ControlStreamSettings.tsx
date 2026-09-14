import { useCallback, useEffect, useRef, useState } from "react";
import { getStreamSettings, setStreamSettings } from "../api";
import { describeError } from "../describeError";
import type { StreamSettings } from "../types";
import { useWorkspaceDraft } from "../workspaceDraft";

const QUALITIES = ["low", "medium", "high", "extra"] as const;
const LABELS = ["Thấp", "Vừa", "Cao", "Rất cao"];
export function ControlStreamSettings() {
  const [saved, setSaved] = useState<StreamSettings | null>(null);
  const [draft, setDraft] = useState<StreamSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const latest = useRef(draft);
  latest.current = draft;
  const lock = useRef<Promise<boolean> | null>(null);
  useEffect(() => {
    let alive = true;
    void getStreamSettings()
      .then((value) => {
        if (alive) {
          setSaved(value);
          setDraft(value);
        }
      })
      .catch((cause) => {
        if (alive) setError(describeError(cause));
      });
    return () => {
      alive = false;
    };
  }, []);
  const dirty = !!draft && JSON.stringify(draft) !== JSON.stringify(saved);
  const save = useCallback((): Promise<boolean> => {
    if (lock.current) return lock.current;
    const value = latest.current;
    if (!value) return Promise.resolve(false);
    setSaving(true);
    setError(null);
    const task = setStreamSettings(value)
      .then((result) => {
        setSaved(result);
        if (latest.current === value) setDraft(result);
        return latest.current === value;
      })
      .catch((cause) => {
        setError(describeError(cause));
        return false;
      })
      .finally(() => {
        lock.current = null;
        setSaving(false);
      });
    lock.current = task;
    return task;
  }, []);
  useWorkspaceDraft({
    id: "control-stream",
    label: "Chất lượng hình",
    dirty,
    snapshotKey: JSON.stringify(draft),
    autoSave: save,
    save,
    discard: () => setDraft(saved),
  });
  if (!draft)
    return (
      <p className="control-stream-status">
        {error ?? "Đang đọc chất lượng hình…"}
      </p>
    );
  const commit = () => {
    if (dirty) void save();
  };
  return (
    <>
      <label>
        <span>
          Chất lượng{" "}
          <output>{LABELS[QUALITIES.indexOf(draft.focusQuality)]}</output>
        </span>
        <input
          aria-label="Chất lượng hình Android"
          type="range"
          min={0}
          max={3}
          step={1}
          value={QUALITIES.indexOf(draft.focusQuality)}
          onChange={(event) => {
            const quality = QUALITIES[Number(event.target.value)];
            setDraft({ ...draft, focusQuality: quality, gridQuality: quality });
          }}
          onPointerUp={commit}
          onKeyUp={commit}
        />
      </label>
      <label>
        <span>
          Tốc độ khung hình <output>{draft.fps} FPS</output>
        </span>
        <input
          aria-label="FPS màn hình Android"
          type="range"
          min={5}
          max={30}
          step={1}
          value={draft.fps}
          onChange={(event) =>
            setDraft({ ...draft, fps: Number(event.target.value) })
          }
          onPointerUp={commit}
          onKeyUp={commit}
        />
      </label>
      <p className="control-stream-status" role="status">
        {saving
          ? "Đang áp dụng…"
          : dirty
            ? "Đang chờ áp dụng"
            : "Android · Đã áp dụng"}
      </p>
      {error && (
        <p className="control-stream-error" role="alert">
          {error}{" "}
          <button type="button" onClick={() => void save()}>
            Thử lại
          </button>
        </p>
      )}
    </>
  );
}
