import { useEffect, useState } from "react";
import { addMaterial, deleteMaterial, listMaterials, pushMaterial } from "../api";
import { SelectionStrip } from "../components/SelectionStrip";
import { flash, flashError } from "../farmToast";
import { targetsOf } from "../selectionTargets";
import { EmptyState } from "../components/States";
import { IconImage } from "../components/Icons";
import { pickMaterial } from "../pickFile";
import type { MaterialItem } from "../types";
import type { SelProps } from "./pageProps";

/** Material library: what the fleet can be given, and pushing it. */
export function MaterialPage({ devices, selected, onSelectUdids }: SelProps) {
  const [items, setItems] = useState<MaterialItem[]>([]);
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);
  const targets = targetsOf(selected, devices);
  const target = targets[0];

  const reload = () => listMaterials().then(setItems).catch((e) => flashError(e));
  useEffect(() => {
    reload();
  }, []);

  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Kho nội dung</h2>
      </header>
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
        {!items.length && (
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
