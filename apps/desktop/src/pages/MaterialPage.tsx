import { useEffect, useMemo, useRef, useState } from "react";
import { ImagePlus, RefreshCw, Send, Trash2 } from "lucide-react";

import {
  addMaterial,
  deleteMaterial,
  listMaterials,
  listGroups,
  pushMaterialBatch,
} from "../api";
import { requestConfirm } from "../confirmStore";
import { TargetSelector } from "../components/TargetSelector";
import { LibraryBatchMonitor } from "../components/LibraryBatchMonitor";
import { useLibraryBatch } from "../useLibraryBatch";
import { EmptyState, LoadingState, StatusNotice } from "../components/States";
import { DetailDrawer, ResponsiveTable, StatusChip } from "../components/WorkspacePrimitives";
import { describeError } from "../describeError";
import { flash, flashError } from "../farmToast";
import { pickMaterial } from "../pickFile";
import { resolveAutomationTarget } from "../automationTargets";
import type { OperationSourceRef } from "../operationSource";
import type {
  MaterialItem,
  MaterialPushBatchResult,
  TargetRef,
  DeviceGroup,
} from "../types";
import type { SelProps } from "./pageProps";
import "../styles/operations.css";

function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Material library backed by the managed artifact store and a bounded fleet transfer. */
export function MaterialPage({ devices, selected, operationSource }: SelProps & { operationSource?: OperationSourceRef }) {
  const batch = useLibraryBatch("materialTransfer", operationSource?.kind === "materialTransfer" ? operationSource.operationId : undefined);
  const [targetRef, setTargetRef] = useState<TargetRef>(() => ({ type: "explicit", udids: [...selected] }));
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  const [groupError, setGroupError] = useState<string | null>(null);
  const [groupRetry, setGroupRetry] = useState(0);
  const [importOpen, setImportOpen] = useState(false);
  const [items, setItems] = useState<MaterialItem[]>([]);
  const [path, setPath] = useState("");
  const [busyMaterialId, setBusyMaterialId] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [transferError, setTransferError] = useState<string | null>(null);
  const [lastBatch, setLastBatch] = useState<MaterialPushBatchResult | null>(null);
  const loadTicket = useRef(0);
  const targets = resolveAutomationTarget(targetRef, devices, groups);
  useEffect(() => {
    let active = true;
    void listGroups().then((next) => { if (active) { setGroups(next); setGroupError(null); } })
      .catch((error) => { if (active) setGroupError(describeError(error)); });
    return () => { active = false; };
  }, [groupRetry]);
  const deviceNames = useMemo(
    () => new Map(devices.map((device, index) => [device.udid, `Máy ${index + 1} · ${device.name}`])),
    [devices],
  );
  const batchDeviceNames = useMemo(() => {
    if (!lastBatch) return new Map<string, string>();
    return new Map(lastBatch.target.included.map((device, index) => {
      const alias = device.alias.trim();
      const number = device.number ?? index + 1;
      const stableName = alias ? `Máy ${number} · ${alias}`
        : device.number ? `Máy ${number}` : deviceNames.get(device.udid) || `Máy ${number}`;
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
      : targetRef;
    if (!(retryUdids ?? targets).length) return;
    setBusyMaterialId(materialId);
    setTransferError(null);
    try {
      const batch = await pushMaterialBatch({ materialId, target });
      // A retry is a new immutable batch with a freshly resolved target snapshot. Keeping
      // results from the old attempt beside the new batch id would also keep a stale roster
      // hash and stale exclusions, so the "latest" panel replaces the attempt atomically.
      setLastBatch(batch);
      if (operationSource) followBatch(`materialTransfer:${batch.batchId}`);
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
      void batch.reload();
    }
  };

  const failedUdids = lastBatch?.results
    .filter((result) => result.status === "failed")
    .map((result) => result.udid) ?? [];
  const followBatch = batch.follow;

  const importForm = (
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
              setImportOpen(false);
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
  );
  const monitor = <>
      {transferError && (
        <StatusNotice tone="error">
          Không thể bắt đầu chuyển nội dung: {transferError}
        </StatusNotice>
      )}

      <LibraryBatchMonitor batch={batch} retryDisabled={busyMaterialId !== null} onRetry={(artifactId,udids) => void transfer(artifactId,udids)} />
      {!batch.detail && lastBatch && (
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
                    {result.status === "succeeded" ? "Đã chuyển" : result.status === "uncertain" ? "Cần kiểm lại" : result.status === "cancelledBeforeDispatch" ? "Đã dừng trước khi chạy" : "Thất bại"}
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
    </>;
  return (
    <div className="operations-page material-page">
      <section className="operations-list-section" aria-label="Nội dung đã lưu">
        <header>
          <div>
            <strong>Nội dung đã lưu</strong>
            <span>{items.length} file</span>
          </div>
          <div className="admin-actions">
          <button type="button" className="primary" onClick={() => setImportOpen(true)}><ImagePlus size={16} /> Thêm nội dung</button>
          <button type="button" className="icon-btn" onClick={() => void reload()} aria-label="Làm mới kho nội dung" title="Làm mới">
            <RefreshCw size={16} />
          </button>
          </div>
        </header>
        <TargetSelector devices={devices} groups={groups} selected={[]} onChange={() => undefined}
          targetRef={targetRef} onTargetRefChange={setTargetRef} requireChoice label="Phạm vi chuyển nội dung" />
        {groupError && <StatusNotice tone="error" action={<button type="button" onClick={() => setGroupRetry((value) => value + 1)}>Thử lại nhóm</button>}>Không tải được nhóm: {groupError}</StatusNotice>}
        {loadError && (
          <StatusNotice
            tone="error"
            action={<button type="button" className="ghost" onClick={() => void reload()}>Thử lại</button>}
          >
            Không tải được kho nội dung: {loadError}
          </StatusNotice>
        )}
        {loading && !items.length && <LoadingState label="Đang tải kho nội dung…" />}
        <ResponsiveTable label="Nội dung đã lưu" viewKey="material" searchText={(material) => `${material.name} ${material.kind}`} rows={items} keyForRow={(material) => material.id}
          empty={!loading && !loadError ? <EmptyState compact icon={<ImagePlus size={17} />} title="Chưa có nội dung" /> : null} columns={[
          { id:"name",label:"Tên file",sortValue:(material) => material.name,render:(material) => <strong>{material.name}</strong> },
          { id:"kind",label:"Loại",sortValue:(material) => material.kind,render:(material) => <StatusChip>{material.kind === "video" ? "Video" : material.kind === "image" ? "Ảnh" : "File"}</StatusChip> },
          { id:"size",label:"Dung lượng",sortValue:(material) => material.size,render:(material) => formatBytes(material.size) },
          { id:"actions",label:"Thao tác",required:true,render:(material) => (
              <div className="admin-actions">
                <button
                  type="button"
                  className="ghost"
                  disabled={!targets.length || busyMaterialId !== null || batch.loading || batch.active || !!batch.error}
                  onClick={() => void transfer(material.id)}
                >
                  <Send size={15} /> Chuyển tới {targets.length} máy
                </button>
                <button
                  type="button"
                  className="icon-btn"
                  aria-label={`Xóa ${material.name}`}
                  title="Xóa khỏi kho"
                  disabled={busyMaterialId !== null || batch.active}
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
              <details>
                <summary>Chi tiết file</summary>
                <code>{material.path}</code>
              </details>
              </div>
          ) },
        ]} />
      </section>
      {monitor}
      <DetailDrawer open={importOpen} title="Thêm nội dung" onClose={() => { if (!adding) setImportOpen(false); }}>{importForm}</DetailDrawer>
    </div>
  );
}
