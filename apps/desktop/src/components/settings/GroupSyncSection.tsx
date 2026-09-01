import { setGroupSync, useGroupSync } from "../../groupSync";
import type { DelayPolicy } from "../../types";

/** Group sync: the delay and offset applied when one phone leads others. */
export function GroupSyncSection() {
  const groupSync = useGroupSync();
  // Normalised locals so the union narrows cleanly in JSX (the store always stores concrete
  // values; the type keeps the fields optional for forward-compat).
  const gsDelay: DelayPolicy = groupSync.delay ?? { mode: "none" };
  const gsMaxPx = groupSync.offset?.maxPx ?? 0;
  return (
    <section className="settings-section">
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
                setGroupSync({
                  ...groupSync,
                  delay: { mode: "random", minMs: 200, maxMs: 800 },
                });
              } else if (mode === "staggered") {
                setGroupSync({ ...groupSync, delay: { mode: "staggered", stepMs: 150 } });
              } else {
                setGroupSync({ ...groupSync, delay: { mode: "none" } });
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
                  setGroupSync({
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
                  setGroupSync({
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
                setGroupSync({ ...groupSync, delay: { mode: "staggered", stepMs: v } });
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
              setGroupSync({ ...groupSync, offset: { maxPx: v } });
            }}
          />
        </label>
      </div>
    </section>
  );
}
