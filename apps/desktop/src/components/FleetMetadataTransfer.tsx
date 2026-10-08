import { pauseImportedWallpapers } from "../wallpaperSync";
import { useState } from "react";
import { exportDeviceMetadata, importDeviceMetadata, type DeviceMetadataTransfer, type DeviceMetadataImportPreview } from "../api";
import { pushToast, toastError } from "../toastStore";

export function FleetMetadataTransfer({ onChanged }: { onChanged: () => Promise<void> | void }) {
  const [input, setInput] = useState<DeviceMetadataTransfer | null>(null);
  const [preview, setPreview] = useState<DeviceMetadataImportPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<DeviceMetadataTransfer | null>(() => {
    try {
      const saved = JSON.parse(localStorage.getItem("riviu.pendingMetadataImport.v1") ?? "null");
      if (!saved) return null;
      const value = saved.input;
      if (!value || value.namespace !== "riviu.device-meta" || value.version !== 1
        || !Array.isArray(value.devices) || !Array.isArray(value.groups)) throw new Error("invalid pending import");
      return value;
    } catch {
      // Preserve a blocking record when provenance cannot be read; never silently resubmit.
      return { namespace: "riviu.device-meta", version: 1, highWater: Number.MAX_SAFE_INTEGER, devices: [], groups: [] };
    }
  });
  const verifyPending = async () => {
    if (!pending) return;
    setBusy(true);
    try {
      const saved = await exportDeviceMetadata();
      const canonical = (value: unknown): unknown => Array.isArray(value) ? value.map(canonical)
        : value && typeof value === "object" ? Object.fromEntries(Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([key, item]) => [key, canonical(item)])) : value;
      const same = (left: unknown, right: unknown) => JSON.stringify(canonical(left)) === JSON.stringify(canonical(right));
      const normalizeGroup = (group: DeviceMetadataTransfer["groups"][number]) => ({ ...group, udids: [...group.udids].sort() });
      const matches = saved.highWater >= pending.highWater
        && pending.devices.every(device => same(saved.devices.find(row => row.udid === device.udid), device))
        && pending.groups.every(group => {
          const current = saved.groups.find(row => row.id === group.id);
          return current && same(normalizeGroup(current), normalizeGroup(group));
        });
      if (!matches) { pushToast("warn", "Chưa xác nhận nhập danh sách", "Dữ liệu chưa khớp hoàn toàn; giữ yêu cầu để kiểm tra. Đồng bộ hình nền vẫn tắt."); return; }
      localStorage.removeItem("riviu.pendingMetadataImport.v1"); setPending(null); setInput(null); setPreview(null);
      pushToast("ok", "Đã xác minh danh sách đã nhập", "Đồng bộ hình nền vẫn tắt; bật lại khi cần.");
      await onChanged();
    } catch (error) { toastError("Chưa xác minh được danh sách đã nhập", error); }
    finally { setBusy(false); }
  };
  const exportFile = async () => {
    setBusy(true);
    try {
      const body = await exportDeviceMetadata();
      const url = URL.createObjectURL(new Blob([JSON.stringify(body, null, 2)], { type: "application/json" }));
      const link = document.createElement("a"); link.href = url; link.download = "riviu-device-metadata.json";
      document.body.append(link); link.click(); link.remove();
      window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
    } catch (error) { toastError("Chưa xuất được danh sách máy", error); }
    finally { setBusy(false); }
  };
  const previewFile = async (file: File) => {
    setBusy(true); setInput(null); setPreview(null);
    try {
      if (file.size > 2_000_000) throw new Error("Tệp danh sách máy quá lớn (tối đa 2 MB).");
      const body: DeviceMetadataTransfer = JSON.parse(await file.text());
      const result = await importDeviceMetadata(body, false);
      setInput(body); setPreview(result);
    } catch (error) { toastError("Chưa đọc được danh sách máy", error); }
    finally { setBusy(false); }
  };
  const apply = async () => {
    if (!input || !preview || preview.conflicts.length || pending) return;
    setBusy(true);
    try {
      pauseImportedWallpapers(input.devices.map(device => device.udid));
      localStorage.setItem("riviu.pendingMetadataImport.v1", JSON.stringify({ operationId: crypto.randomUUID(), input }));
      setPending(input);
      const result = await importDeviceMetadata(input, true);
      setPreview(result);
      if (result.applied) {
        localStorage.removeItem("riviu.pendingMetadataImport.v1"); setPending(null); setInput(null);
        pushToast("ok", "Đã nhập danh sách máy", "Đồng bộ hình nền của các máy nhập đã tắt; bật lại khi cần.");
        try { await onChanged(); } catch (error) { toastError("Đã nhập nhưng chưa làm mới danh sách", error); }
      }
    } catch (error) { setPreview(null); toastError("Chưa xác nhận nhập danh sách; kiểm tra lại trước khi thử lại", error); }
    finally { setBusy(false); }
  };
  return <details className="fleet-metadata-transfer">
    <summary>Chuyển danh sách máy sang PC khác</summary>
    <p>Giữ số, tên và nhóm theo serial. Xem xung đột trước khi nhập; không thay tài khoản trên điện thoại.</p>
    {pending && <div role="status"><p>Lượt nhập trước chưa xác nhận. Đã giữ yêu cầu; chưa gửi lại.</p><button type="button" disabled={busy} onClick={() => void verifyPending()}>Đọc lại kết quả nhập</button></div>}
    <button type="button" disabled={busy} onClick={() => void exportFile()}>Xuất danh sách</button>
    <label>Nhập tệp JSON <input type="file" accept=".json,application/json" disabled={busy || Boolean(pending)} onChange={event => {
      const file = event.target.files?.[0]; event.target.value = ""; if (file) void previewFile(file);
    }} /></label>
    {preview && <div role="status">
      {preview.conflicts.length ? <><strong>Chưa nhập: có xung đột</strong><ul>{preview.conflicts.map((conflict, index) => <li key={index}>{conflict}</li>)}</ul></>
        : <p>{preview.applied ? "Đã nhập." : "Đã kiểm tra tệp, không có xung đột."}</p>}
      {input && !preview.conflicts.length && <button type="button" disabled={busy || Boolean(pending)} onClick={() => void apply()}>Xác nhận nhập danh sách</button>}
    </div>}
  </details>;
}
