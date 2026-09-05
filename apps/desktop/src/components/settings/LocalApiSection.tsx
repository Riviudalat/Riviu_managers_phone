import { useCallback, useEffect, useRef, useState } from "react";
import { Save } from "lucide-react";
import { localApiGetConfig, localApiSetConfig, localApiStatus, type LocalApiConfig, type LocalApiStatus } from "../../api";
import { useWorkspaceDraft } from "../../workspaceDraft";
import { describeError } from "../../describeError";
import { LoadingState, StatusNotice, type NoticeTone } from "../States";

/** The loopback HTTP API: whether it listens, on what port, behind what token. */
export function LocalApiSection() {
  const [localApi, setLocalApi] = useState<LocalApiConfig | null>(null);
  const [saved, setSaved] = useState<LocalApiConfig | null>(null);
  const [status, setStatus] = useState<LocalApiStatus | null>(null);
  const editEpoch = useRef(0);
  const savingRef = useRef(false);
  const [savingApi, setSavingApi] = useState(false);
  const [apiMessage, setApiMessage] = useState<{ tone: NoticeTone; text: string } | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const load = useCallback(() => {
    setLocalApi(null);
    setLoadError(null);
    localApiGetConfig()
      .then((config) => { setLocalApi(config); setSaved(config); })
      .catch((error) => setLoadError(describeError(error)));
    void localApiStatus().then(setStatus).catch(() => setStatus(null));
  }, []);

  useEffect(load, [load]);
  const dirty = Boolean(localApi && saved && JSON.stringify(localApi) !== JSON.stringify(saved));
  const patch = (config: LocalApiConfig) => { editEpoch.current += 1; setLocalApi(config); setApiMessage(null); };
  const discard = () => { editEpoch.current += 1; setLocalApi(saved); setApiMessage(null); };
  const save = async () => {
    if (!localApi || savingRef.current) return false;
    if (!Number.isInteger(localApi.port) || localApi.port < 1 || localApi.port > 65535) {
      setApiMessage({ tone: "error", text: "Cổng phải là số nguyên từ 1 đến 65535." });
      return false;
    }
    const epoch = editEpoch.current;
    savingRef.current = true;
    setSavingApi(true);
    setApiMessage(null);
    try {
      const next = await localApiSetConfig(localApi);
      setSaved(next);
      if (epoch === editEpoch.current) setLocalApi(next);
      try { setStatus(await localApiStatus()); } catch { setStatus(null); }
      if (epoch === editEpoch.current) setApiMessage({ tone: "success", text: "Đã lưu cấu hình. Thay đổi có hiệu lực sau khi khởi động lại ứng dụng." });
      return epoch === editEpoch.current;
    } catch (error) {
      setApiMessage({ tone: "error", text: describeError(error) });
      return false;
    } finally {
      savingRef.current = false;
      setSavingApi(false);
    }
  };
  useWorkspaceDraft({ id: "settings-api", label: "API cục bộ", dirty, snapshotKey: String(editEpoch.current), save, discard });
  return (
    <section className="settings-section" aria-label="API tự động hoá cục bộ">
      <h3>API tự động hoá cục bộ</h3>
      <p className="hint">
        Khi bật, script trên chính máy tính có thể điều khiển fleet; thay đổi chỉ có hiệu lực sau khi khởi động lại app.
      </p>
      <p className="hint" role="status">
        {status?.running === true ? `Đang chạy tại 127.0.0.1:${status.activePort}` : status?.running === false ? "Hiện không chạy" : "Chưa đọc được trạng thái đang chạy"}
        {status?.restartRequired ? ". Cần khởi động lại để áp dụng cấu hình đã lưu." : "."}
      </p>
      {status?.lastError && <StatusNotice tone="error">{status.lastError}</StatusNotice>}
      <details className="settings-details" aria-label="Phạm vi API cục bộ">
        <summary>Phạm vi API cục bộ</summary>
        <p className="hint">
          Máy chủ chỉ nghe tại 127.0.0.1 và mọi lệnh đều cần token Bearer. Mặc định API tắt.
        </p>
      </details>
      {!localApi && !loadError && <LoadingState label="Đang đọc cấu hình API…" />}
      {loadError && (
        <StatusNotice tone="error" action={<button type="button" className="ghost" onClick={load}>Thử lại</button>}>
          {loadError}
        </StatusNotice>
      )}
      {localApi && (
        <>
          <label className="agent-toggle" style={{ marginBottom: "0.5rem" }}>
            <input
              type="checkbox"
              checked={localApi.enabled}
              onChange={(event) => patch({ ...localApi, enabled: event.target.checked })}
            />
            Bật API cục bộ
          </label>
          <div className="row">
            <label>
              Cổng
              <input
                type="number"
                min={1}
                max={65535}
                value={localApi.port}
                onChange={(event) =>
                  patch({ ...localApi, port: Number(event.target.value) || 0 })
                }
                style={{ width: "8rem" }}
              />
            </label>
            <label style={{ flex: 1 }}>
              Token (Bearer)
              <input type="text" readOnly value={localApi.token || "(tạo khi lưu)"} className="mono" />
            </label>
            <button
              type="button"
              className="ghost"
              onClick={() => patch({ ...localApi, token: "" })}
              title="Xoá token hiện tại; lưu sẽ tạo token mới"
            >
              Tạo token mới
            </button>
          </div>
          <div className="row" style={{ marginTop: "0.5rem" }}>
            <button
              type="button"
              className="primary"
              disabled={!dirty || savingApi}
              onClick={() => void save()}
            >
              <Save size={15} />{savingApi ? "Đang lưu…" : "Lưu API cục bộ"}
            </button>
            {dirty && <button type="button" className="ghost" disabled={savingApi} onClick={discard}>Bỏ thay đổi</button>}
          </div>
          {localApi.token && (
            <details className="settings-details" aria-label="Ví dụ gọi API">
              <summary>Ví dụ gọi API</summary>
              <pre className="group-tools-log">
              {`# ví dụ: liệt kê máy\ncurl -H "Authorization: Bearer ${localApi.token}" http://127.0.0.1:${localApi.port}/v1/devices\n\n# chạm toạ độ trên một máy\ncurl -X POST -H "Authorization: Bearer ${localApi.token}" \\\n  -d '{"udid":"<udid>","x":540,"y":1200}' http://127.0.0.1:${localApi.port}/v1/tap`}
              </pre>
            </details>
          )}
        </>
      )}
      {apiMessage && <StatusNotice tone={apiMessage.tone}>{apiMessage.text}</StatusNotice>}
    </section>
  );
}
