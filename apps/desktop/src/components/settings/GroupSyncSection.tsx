import { setGroupSync, useGroupSync } from "../../groupSync";
import { useState } from "react";
import { Save } from "lucide-react";
import { useWorkspaceDraft } from "../../workspaceDraft";
import { StatusNotice } from "../States";
import type { DelayPolicy } from "../../types";

/** Group sync: the delay and offset applied when one phone leads others. */
export function GroupSyncSection() {
  const saved = useGroupSync();
  const [groupSync, setDraft] = useState(saved);
  const [error, setError] = useState<string | null>(null);
  const dirty = JSON.stringify(groupSync) !== JSON.stringify(saved);
  const discard = () => { setDraft(saved); setError(null); };
  const save = async () => {
    const delay = groupSync.delay;
    const values = delay?.mode === "random" ? [delay.minMs, delay.maxMs] : delay?.mode === "staggered" ? [delay.stepMs] : [];
    values.push(groupSync.offset?.maxPx ?? 0);
    if (values.some((value) => !Number.isInteger(value) || value < 0) || (delay?.mode === "random" && delay.minMs > delay.maxMs)) {
      setError("Độ trễ và độ lệch phải là số nguyên không âm; tối đa không được nhỏ hơn tối thiểu.");
      return false;
    }
    setGroupSync(groupSync);
    setError(null);
    return true;
  };
  useWorkspaceDraft({ id: "settings-sync", label: "Đồng bộ nhóm", dirty, snapshotKey: JSON.stringify(groupSync), save, discard });
  // Normalised locals so the union narrows cleanly in JSX (the store always stores concrete
  // values; the type keeps the fields optional for forward-compat).
  const gsDelay: DelayPolicy = groupSync.delay ?? { mode: "none" };
  const gsMaxPx = groupSync.offset?.maxPx ?? 0;
  return (
    <section className="settings-section" aria-label="Đồng bộ nhóm">
      <h3>Đồng bộ nhóm</h3>
      <p className="hint">
        Độ trễ và lệch toạ độ chỉ áp khi điều khiển ít nhất hai máy; đặt cả hai về tắt để phát đồng loạt.
      </p>
      <details className="settings-details" aria-label="Cách đồng bộ nhóm">
        <summary>Cách đồng bộ nhóm</summary>
        <p className="hint">
          Chạm, vuốt, gõ và phím được phát tới từng máy theo chính sách đã chọn. Điều khiển một máy không dùng các giá trị này.
        </p>
      </details>
      <div className="row">
        <label>
          Độ trễ mỗi máy
          <select
            value={gsDelay.mode}
            onChange={(event) => {
              const mode = event.target.value;
              if (mode === "random") {
                setDraft({
                  ...groupSync,
                  delay: { mode: "random", minMs: 200, maxMs: 800 },
                });
              } else if (mode === "staggered") {
                setDraft({ ...groupSync, delay: { mode: "staggered", stepMs: 150 } });
              } else {
                setDraft({ ...groupSync, delay: { mode: "none" } });
              }
            }}
          >
            <option value="none">Tắt</option>
            <option value="random">Ngẫu nhiên</option>
            <option value="staggered">So le theo thứ tự</option>
          </select>
        </label>
        {gsDelay.mode === "random" && (
          <>
            <label>
              Tối thiểu (ms)
              <input
                type="number"
                min={0}
                value={gsDelay.minMs}
                onChange={(event) => {
                  const v = Math.max(0, Math.round(Number(event.target.value) || 0));
                  setDraft({
                    ...groupSync,
                    delay: { mode: "random", minMs: v, maxMs: gsDelay.maxMs },
                  });
                }}
              />
            </label>
            <label>
              Tối đa (ms)
              <input
                type="number"
                min={0}
                value={gsDelay.maxMs}
                onChange={(event) => {
                  const v = Math.max(0, Math.round(Number(event.target.value) || 0));
                  setDraft({
                    ...groupSync,
                    delay: { mode: "random", minMs: gsDelay.minMs, maxMs: v },
                  });
                }}
              />
            </label>
          </>
        )}
        {gsDelay.mode === "staggered" && (
          <label>
            Bước (ms mỗi máy)
            <input
              type="number"
              min={0}
              value={gsDelay.stepMs}
              onChange={(event) => {
                const v = Math.max(0, Math.round(Number(event.target.value) || 0));
                setDraft({ ...groupSync, delay: { mode: "staggered", stepMs: v } });
              }}
            />
          </label>
        )}
        <label>
          Lệch toạ độ (± px)
          <input
            type="number"
            min={0}
            value={gsMaxPx}
            onChange={(event) => {
              const v = Math.max(0, Math.round(Number(event.target.value) || 0));
              setDraft({ ...groupSync, offset: { maxPx: v } });
            }}
          />
        </label>
      </div>
      {error && <StatusNotice tone="error">{error}</StatusNotice>}
      <div className="row">
        <button type="button" className="primary" disabled={!dirty} onClick={() => void save()}><Save size={15} />Áp dụng đồng bộ nhóm</button>
        {dirty && <button type="button" className="ghost" onClick={discard}>Bỏ thay đổi</button>}
      </div>
    </section>
  );
}
