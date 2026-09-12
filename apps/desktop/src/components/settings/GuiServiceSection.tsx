import { useEffect, useRef, useState } from "react";
import {
  guiServiceSave,
  guiServiceStatus,
  guiServiceCheck,
  guiCompatibilityImport,
  guiCompatibilityRollback,
  guiDiagnosticsExport,
  type GuiServiceConfig,
  type GuiServiceStatus,
} from "../../api";
import { describeError } from "../../describeError";
import { useWorkspaceDraft } from "../../workspaceDraft";

export function GuiServiceSection() {
  const [status, setStatus] = useState<GuiServiceStatus | null>(null);
  const [config, setConfig] = useState<GuiServiceConfig | null>(null);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  const pending = useRef(false);
  useEffect(() => {
    let active = true;
    guiServiceStatus()
      .then((value) => {
        if (active) {
          setStatus(value);
          setConfig(value.config);
        }
      })
      .catch((error) => {
        if (active) setMessage(describeError(error));
      });
    return () => {
      active = false;
    };
  }, []);
  const dirty = Boolean(
    config &&
      status &&
      JSON.stringify(config) !== JSON.stringify(status.config),
  );
  const save = async () => {
    if (!config || pending.current) return false;
    if (
      !Number.isInteger(config.maxRequests) ||
      config.maxRequests < 1 ||
      config.maxRequests > 100
    ) {
      setMessage("Giới hạn request phải từ 1 đến 100.");
      return false;
    }
    const epoch = generation.current;
    pending.current = true;
    setBusy(true);
    try {
      await guiServiceSave(config);
      const next = await guiServiceStatus();
      setStatus(next);
      if (epoch === generation.current) {
        setConfig(next.config);
        setMessage("Đã lưu cấu hình nhận diện.");
      }
      return epoch === generation.current;
    } catch (error) {
      setMessage(describeError(error));
      return false;
    } finally {
      pending.current = false;
      setBusy(false);
    }
  };
  const discard = () => {
    generation.current += 1;
    setConfig(status?.config ?? null);
  };
  useWorkspaceDraft({
    id: "settings-gui",
    label: "Nhận diện giao diện",
    dirty,
    snapshotKey: String(generation.current),
    save,
    autoSave: save,
    discard,
  });
  const patch = (value: Partial<GuiServiceConfig>) => {
    generation.current += 1;
    setConfig((old) => (old ? { ...old, ...value } : old));
    setMessage("");
  };
  const action = async (fn: () => Promise<string>) => {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    try {
      setMessage(await fn());
      setStatus(await guiServiceStatus());
    } catch (error) {
      setMessage(describeError(error));
    } finally {
      pending.current = false;
      setBusy(false);
    }
  };
  return (
    <section
      className="settings-section gui-service-section"
      aria-label="Nhận diện giao diện"
    >
      <h3>Nhận diện giao diện</h3>
      <p className="hint">
        Hỗ trợ tìm lại nút điều hướng khi giao diện thay đổi. Dùng khóa AI đã
        lưu; để trống model và địa chỉ để dùng cấu hình AI hiện tại.
      </p>
      {config && (
        <>
          <label className="gui-service-toggle">
            <input
              type="checkbox"
              checked={config.enabled}
              onChange={(e) => patch({ enabled: e.target.checked })}
            />{" "}
            Hỗ trợ nhận diện bằng AI khi cần
          </label>
          <label className="settings-field">
            Địa chỉ provider
            <input
              value={config.baseUrl}
              placeholder="Dùng provider AI hiện tại"
              onChange={(e) => patch({ baseUrl: e.target.value })}
            />
          </label>
          <label className="settings-field">
            Model nhận diện
            <input
              value={config.model}
              placeholder="Dùng model AI hiện tại"
              onChange={(e) => patch({ model: e.target.value })}
            />
          </label>
          <label className="settings-field">
            Giới hạn request mỗi phiên
            <input
              type="number"
              min={1}
              max={100}
              value={config.maxRequests}
              onChange={(e) => patch({ maxRequests: Number(e.target.value) })}
            />
          </label>
          <p>
            {status?.running
              ? "Dịch vụ đang chạy"
              : "Dịch vụ khởi động khi cần"}{" "}
            · {status?.providerReady ? "Đã có khóa AI" : "Chưa có khóa AI"}
          </p>
          <div className="settings-actions">
            <button disabled={busy || !dirty} onClick={() => void save()}>
              Lưu cấu hình nhận diện
            </button>
            <button
              disabled={busy || dirty}
              onClick={() => void action(guiServiceCheck)}
            >
              Kiểm tra dịch vụ
            </button>
            <button
              disabled={busy}
              onClick={() => void action(guiDiagnosticsExport)}
            >
              Xuất chẩn đoán nhận diện
            </button>
            <button
              disabled={busy}
              onClick={() => void action(guiCompatibilityRollback)}
            >
              Khôi phục gói tương thích trước
            </button>
          </div>
          <label className="gui-import-label">
            Nhập gói tương thích
            <input
              type="file"
              aria-label="Nhập gói tương thích"
              accept="application/json,.json"
              disabled={busy}
              onChange={(e) => {
                const file = e.target.files?.[0];
                if (file)
                  void action(async () =>
                    guiCompatibilityImport(await file.text()),
                  );
                e.target.value = "";
              }}
            />
          </label>
        </>
      )}
      {message && <p role="status">{message}</p>}
      {status?.lastError && (
        <details>
          <summary>Chi tiết lỗi dịch vụ</summary>
          <p>{status.lastError}</p>
        </details>
      )}
    </section>
  );
}
