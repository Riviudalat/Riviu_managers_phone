import { useEffect, useState } from "react";
import { localApiGetConfig, localApiSetConfig, type LocalApiConfig } from "../../api";
import { describeError } from "../../toastStore";

/** The loopback HTTP API: whether it listens, on what port, behind what token. */
export function LocalApiSection() {
  const [localApi, setLocalApi] = useState<LocalApiConfig | null>(null);
  const [savingApi, setSavingApi] = useState(false);
  const [apiMessage, setApiMessage] = useState<string | null>(null);

  useEffect(() => {
    localApiGetConfig()
      .then(setLocalApi)
      .catch((error) => setApiMessage(describeError(error)));
  }, []);
  return (
    <section className="settings-section">
      <h3>API tự động hoá cục bộ (openapi)</h3>
      <p className="hint">
        Máy chủ HTTP chỉ chạy trên loopback (127.0.0.1) để script bên ngoài điều khiển fleet
        — bật/tắt màn, chạm, vuốt, gõ, phím. Mặc định TẮT, luôn cần token Bearer. Đổi cấu
        hình có hiệu lực sau khi khởi động lại ứng dụng.
      </p>
      {localApi && (
        <>
          <label className="agent-toggle" style={{ marginBottom: "0.5rem" }}>
            <input
              type="checkbox"
              checked={localApi.enabled}
              onChange={(event) => setLocalApi({ ...localApi, enabled: event.target.checked })}
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
                  setLocalApi({ ...localApi, port: Number(event.target.value) || 0 })
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
              onClick={() => setLocalApi({ ...localApi, token: "" })}
              title="Xoá token hiện tại; lưu sẽ tạo token mới"
            >
              Tạo token mới
            </button>
          </div>
          <div className="row" style={{ marginTop: "0.5rem" }}>
            <button
              type="button"
              className="primary"
              disabled={savingApi}
              onClick={async () => {
                setSavingApi(true);
                setApiMessage(null);
                try {
                  const saved = await localApiSetConfig(localApi);
                  setLocalApi(saved);
                  setApiMessage(
                    saved.enabled
                      ? `Đã lưu. API sẽ chạy ở 127.0.0.1:${saved.port} sau khi khởi động lại ứng dụng.`
                      : "Đã lưu (API đang tắt).",
                  );
                } catch (error) {
                  setApiMessage(describeError(error));
                } finally {
                  setSavingApi(false);
                }
              }}
            >
              {savingApi ? "Đang lưu…" : "Lưu"}
            </button>
          </div>
          {localApi.token && (
            <pre className="group-tools-log" style={{ marginTop: "0.5rem" }}>
              {`# ví dụ: liệt kê máy\ncurl -H "Authorization: Bearer ${localApi.token}" http://127.0.0.1:${localApi.port}/v1/devices\n\n# chạm toạ độ trên một máy\ncurl -X POST -H "Authorization: Bearer ${localApi.token}" \\\n  -d '{"udid":"<udid>","x":540,"y":1200}' http://127.0.0.1:${localApi.port}/v1/tap`}
            </pre>
          )}
        </>
      )}
      {apiMessage && <p className="hint">{apiMessage}</p>}
    </section>
  );
}
