import { useEffect, useMemo, useRef, useState } from "react";
import { ImagePlus, RefreshCw, Send, Trash2 } from "lucide-react";

import {
  addMaterial,
  deleteMaterial,
  listMaterials,
  pushMaterialBatch,
} from "../api";
import { requestConfirm } from "../confirmStore";
import { SelectionStrip } from "../components/SelectionStrip";
import { EmptyState, LoadingState, StatusNotice } from "../components/States";
import { describeError } from "../describeError";
import { flash, flashError } from "../farmToast";
import { pickMaterial } from "../pickFile";
import { targetsOf } from "../selectionTargets";
import type {
  MaterialItem,
  MaterialPushBatchResult,
  TargetRef,
} from "../types";
import type { SelProps } from "./pageProps";
import "../styles/operations.css";

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Material library backed by the managed artifact store and a bounded fleet transfer. */
export function MaterialPage({ devices, selected, onSelectUdids }: SelProps) {
  const [items, setItems] = useState<MaterialItem[]>([]);
  const [path, setPath] = useState("");
  const [busyMaterialId, setBusyMaterialId] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [transferError, setTransferError] = useState<string | null>(null);
  const [lastBatch, setLastBatch] = useState<MaterialPushBatchResult | null>(null);
  const loadTicket = useRef(0);
  const targets = targetsOf(selected, devices);
  const deviceNames = useMemo(
    () => new Map(devices.map((device, index) => [device.udid, `Máy ${index + 1} · ${device.name}`])),
    [devices],
  );
  const batchDeviceNames = useMemo(() => {
    if (!lastBatch) return new Map<string, string>();
    return new Map(lastBatch.target.included.map((device, index) => {
      const alias = device.alias.trim();
      const stableName = alias
        || (device.number ? `Máy ${device.number}` : null)
        || deviceNames.get(device.udid)
        || `Máy ${index + 1}`;
      return [device.udid, stableName];
    }));
  }, [deviceNames, lastBatch]);

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

  const transfer = async (materialId: string, retryUdids?: string[]) => {
    const target: TargetRef = retryUdids
      ? { type: "explicit", udids: retryUdids }
      : selected.length
        ? { type: "explicit", udids: targets }
        : { type: "all" };
    setBusyMaterialId(materialId);
    setTransferError(null);
    try {
      const batch = await pushMaterialBatch({ materialId, target });
      // A retry is a new immutable batch with a freshly resolved target snapshot. Keeping
      // results from the old attempt beside the new batch id would also keep a stale roster
      // hash and stale exclusions, so the "latest" panel replaces the attempt atomically.
      setLastBatch(batch);
      const succeeded = batch.results.filter((result) => result.status === "succeeded").length;
      const failed = batch.results.length - succeeded;
      flash(
        failed
          ? `Đã chuyển ${succeeded}/${batch.results.length} máy; ${failed} máy cần xử lý`
          : `Đã chuyển tới ${succeeded} máy`,
      );
    } catch (error) {
      setTransferError(describeError(error));
    } finally {
      setBusyMaterialId(null);
    }
  };

  const failedUdids = lastBatch?.results
    .filter((result) => result.status === "failed")
    .map((result) => result.udid) ?? [];

  return (
    <div className="panel operations-page material-page">
      <SelectionStrip
        devices={devices}
        selected={selected}
        onSelectAll={() => onSelectUdids(devices.map((device) => device.udid))}
        onClear={() => onSelectUdids([])}
        onSelectUdids={onSelectUdids}
      />

      <section className="operations-toolbar" aria-label="Thêm nội dung">
        <label className="operations-file-field">
          <span>File nguồn</span>
          <input
            value={path}
            onChange={(event) => setPath(event.target.value)}
            placeholder="Chọn ảnh hoặc video"
          />
        </label>
        <button
          type="button"
          className="ghost"
          onClick={async () => {
            const picked = await pickMaterial();
            if (picked) setPath(picked);
          }}
        >
          <ImagePlus size={16} /> Chọn file
        </button>
        <button
          type="button"
          className="primary"
          disabled={!path.trim() || adding}
          onClick={async () => {
            setAdding(true);
            try {
              await addMaterial(path.trim());
              setPath("");
              await reload();
              flash("Đã thêm vào kho nội dung");
            } catch (error) {
              flashError(error);
            } finally {
              setAdding(false);
            }
          }}
        >
          Thêm vào kho
        </button>
      </section>

      {transferError && (
        <StatusNotice tone="error">
          Không thể bắt đầu chuyển nội dung: {transferError}
        </StatusNotice>
      )}

      {lastBatch && (
        <section className="operations-results" aria-label="Kết quả chuyển gần nhất">
          <header>
            <div>
              <strong>Kết quả chuyển gần nhất</strong>
              <span>{lastBatch.results.length} máy trong phạm vi đã chốt</span>
            </div>
            {failedUdids.length > 0 && (
              <button
                type="button"
                className="ghost"
                disabled={busyMaterialId !== null}
                onClick={() => void transfer(lastBatch.materialId, failedUdids)}
              >
                <RefreshCw size={15} /> Thử lại {failedUdids.length} máy lỗi
              </button>
            )}
          </header>
          {lastBatch.target.excluded.length > 0 && (
            <StatusNotice tone="warning">
              <div>
                {lastBatch.target.excluded.length} máy đã bị loại khỏi snapshot vì không còn kết nối
                hoặc bị lặp.
                <details>
                  <summary>Xem máy bị loại</summary>
                  {lastBatch.target.excluded.map(({ device, reason }) => (
                    <code key={`${device.udid}:${reason}`}>
                      {device.alias || (device.number ? `Máy ${device.number}` : device.udid)}
                      {" · "}
                      {reason === "not_in_roster" ? "không còn kết nối" : "bị lặp trong phạm vi"}
                    </code>
                  ))}
                </details>
              </div>
            </StatusNotice>
          )}
          <div className="operations-result-list">
            {lastBatch.results.map((result) => (
              <article key={result.udid}>
                <div>
                  <strong>{batchDeviceNames.get(result.udid) ?? "Máy trong snapshot"}</strong>
                  <span className={`pill ${result.status}`}>
                    {result.status === "succeeded" ? "Đã chuyển" : "Thất bại"}
                  </span>
                </div>
                <details>
                  <summary>Chi tiết</summary>
                  <code>{result.udid}</code>
                  {result.evidence && <p>Bằng chứng đọc lại: {result.evidence}</p>}
                  {result.errorCode && <code>{result.errorCode}</code>}
                  {result.error && <p className="error">{result.error}</p>}
                </details>
              </article>
            ))}
          </div>
        </section>
      )}

      <section className="operations-list-section" aria-label="Nội dung đã lưu">
        <header>
          <div>
            <strong>Nội dung đã lưu</strong>
            <span>{items.length} file</span>
          </div>
          <button type="button" className="icon-btn" onClick={() => void reload()} aria-label="Làm mới kho nội dung" title="Làm mới">
            <RefreshCw size={16} />
          </button>
        </header>
        {loadError && (
          <StatusNotice
            tone="error"
            action={<button type="button" className="ghost" onClick={() => void reload()}>Thử lại</button>}
          >
            Không tải được kho nội dung: {loadError}
          </StatusNotice>
        )}
        {loading && !items.length && <LoadingState label="Đang tải kho nội dung…" />}
        <div className="operations-card-grid">
          {items.map((material) => (
            <article key={material.id} className="operations-card">
              <div className="operations-card-title">
                <div>
                  <strong>{material.name}</strong>
                  <span>{formatBytes(material.size)}</span>
                </div>
                <span className="pill">{material.kind === "video" ? "Video" : material.kind === "image" ? "Ảnh" : "File"}</span>
              </div>
              <div className="operations-card-actions">
                <button
                  type="button"
                  className="primary"
                  disabled={!targets.length || busyMaterialId !== null}
                  onClick={() => void transfer(material.id)}
                >
                  <Send size={15} /> Chuyển tới {targets.length} máy
                </button>
                <button
                  type="button"
                  className="icon-btn"
                  aria-label={`Xóa ${material.name}`}
                  title="Xóa khỏi kho"
                  disabled={busyMaterialId !== null}
                  onClick={async () => {
                    const confirmed = await requestConfirm({
                      title: "Xóa nội dung khỏi kho?",
                      message: material.name,
                      confirmLabel: "Xóa",
                      cancelLabel: "Giữ lại",
                      danger: true,
                    });
                    if (!confirmed) return;
                    try {
                      await deleteMaterial(material.id);
                      if (lastBatch?.materialId === material.id) setLastBatch(null);
                      await reload();
                    } catch (error) {
                      flashError(error);
                    }
                  }}
                >
                  <Trash2 size={16} />
                </button>
              </div>
              <details>
                <summary>Chi tiết file</summary>
                <code>{material.path}</code>
              </details>
            </article>
          ))}
        </div>
        {!loading && !loadError && !items.length && (
          <EmptyState
            compact
            icon={<ImagePlus size={17} />}
            title="Chưa có nội dung"
            hint="Chọn ảnh hoặc video để thêm vào kho."
          />
        )}
      </section>
    </div>
  );
}
