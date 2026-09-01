import { useEffect, useRef, useState } from "react";
import { addMaterial, deleteMaterial, listMaterials, pushMaterial } from "../api";
import { SelectionStrip } from "../components/SelectionStrip";
import { flash, flashError } from "../farmToast";
import { targetsOf } from "../selectionTargets";
import { EmptyState, LoadingState, StatusNotice } from "../components/States";
import { IconImage } from "../components/Icons";
import { pickMaterial } from "../pickFile";
import { describeError } from "../describeError";
import type { MaterialItem } from "../types";
import type { SelProps } from "./pageProps";

/** Material library: what the fleet can be given, and pushing it. */
export function MaterialPage({ devices, selected, onSelectUdids }: SelProps) {
  const [items, setItems] = useState<MaterialItem[]>([]);
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const loadTicket = useRef(0);
  const targets = targetsOf(selected, devices);
  const target = targets[0];

  const reload = async () => {
    const ticket = ++loadTicket.current;
    setLoading(true);
    setLoadError(null);
    try {
      const next = await listMaterials();
      if (ticket === loadTicket.current) setItems(next);
    } catch (error) {
      if (ticket === loadTicket.current) setLoadError(describeError(error));
    } finally {
      if (ticket === loadTicket.current) setLoading(false);
    }
  };
  useEffect(() => {
    void reload();
    return () => {
      loadTicket.current += 1;
    };
  }, []);

  return (
    <div className="panel">
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((d) => d.udid))}
        onClear={() => onSelectUdids([])}
        onSelectUdids={onSelectUdids}
      />
      <div className="row" style={{ marginTop: 8 }}>
        <input
          style={{ flex: 1 }}
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder="Đường dẫn file…"
        />
        <button
          type="button"
          className="ghost"
          onClick={async () => {
            const p = await pickMaterial();
            if (p) setPath(p);
          }}
        >
          Chọn file…
        </button>
        <button
          type="button"
          className="primary"
          disabled={!path.trim() || busy}
          onClick={async () => {
            setBusy(true);
            try {
              await addMaterial(path.trim());
              setPath("");
              await reload();
              flash("Đã thêm material");
            } catch (e) {
              flashError(e);
            } finally {
              setBusy(false);
            }
          }}
        >
          Thêm
        </button>
      </div>
      <div className="job-list" style={{ marginTop: 12 }}>
        {loadError && (
          <StatusNotice
            tone="error"
            action={(
              <button type="button" className="ghost" onClick={() => void reload()}>
                Thử lại
              </button>
            )}
          >
            Không tải được kho nội dung: {loadError}
          </StatusNotice>
        )}
        {loading && !items.length && <LoadingState label="Đang tải kho nội dung…" />}
        {items.map((m) => (
          <article key={m.id} className="job-card">
            <div>
              <strong>{m.name}</strong>
              <span className="pill">{m.kind}</span>
            </div>
            <p className="hint">
              {(m.size / 1024).toFixed(1)} KB · {m.path}
            </p>
            <div className="row">
              <button
                type="button"
                className="primary"
                disabled={!target || busy}
                onClick={async () => {
                  setBusy(true);
                  try {
                    flash(await pushMaterial(target!, m.id));
                  } catch (e) {
                    flashError(e);
                  } finally {
                    setBusy(false);
                  }
                }}
              >
                Push → {target ? target.slice(0, 8) : "?"}
              </button>
              <button
                type="button"
                className="ghost"
                onClick={async () => {
                  await deleteMaterial(m.id);
                  await reload();
                }}
              >
                Xóa
              </button>
            </div>
          </article>
        ))}
        {!loading && !loadError && !items.length && (
          <EmptyState
            compact
            icon={<IconImage size={15} />}
            title="Chưa có nội dung"
            hint="Bấm «Chọn file…» để thêm ảnh hoặc video vào kho."
          />
        )}
      </div>
    </div>
  );
}
