import { useCallback, useEffect, useState } from "react";
import { clearAppleId, getAppleId, setAppleId } from "../../api";
import { describeError } from "../../describeError";
import { LoadingState, StatusNotice, type NoticeTone } from "../States";

/** Apple ID for the legacy stock-agent path. */
export function LegacyAgentSection() {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [hasPassword, setHasPassword] = useState(false);
  const [legacyMessage, setLegacyMessage] = useState<{ tone: NoticeTone; text: string } | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  const load = useCallback(() => {
    setLoading(true);
    setLoadError(null);
    getAppleId()
      .then((config) => {
        setEmail(config.email);
        setHasPassword(config.hasPassword);
      })
      .catch((error) => setLoadError(describeError(error)))
      .finally(() => setLoading(false));
  }, []);

  useEffect(load, [load]);
  return (
    <section className="settings-section">
      <h3>Khôi phục agent iOS cũ</h3>
      <p className="hint">
        Thông tin này chỉ dùng khi cần quay lại agent iOS dự phòng; đường điều khiển chính không đọc nó.
      </p>
      {loading && <LoadingState label="Đang đọc cấu hình iOS…" />}
      {loadError && (
        <StatusNotice tone="error" action={<button type="button" className="ghost" onClick={load}>Thử lại</button>}>
          {loadError}
        </StatusNotice>
      )}
      <label>
        Email
        <input value={email} onChange={(event) => setEmail(event.target.value)} />
      </label>
      <label>
        Mật khẩu {hasPassword ? "(đã lưu)" : ""}
        <input
          type="password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          placeholder={hasPassword ? "••••••••" : ""}
        />
      </label>
      {legacyMessage && <StatusNotice tone={legacyMessage.tone}>{legacyMessage.text}</StatusNotice>}
      <div className="row">
        <button
          type="button"
          className="primary"
          onClick={async () => {
            try {
              await setAppleId(email, password);
              setHasPassword(true);
              setPassword("");
              setLegacyMessage({ tone: "success", text: "Đã lưu trong kho thông tin xác thực của Windows" });
            } catch (error) {
              setLegacyMessage({ tone: "error", text: describeError(error) });
            }
          }}
        >
          Lưu
        </button>
        <button
          type="button"
          className="ghost"
          onClick={async () => {
            try {
              await clearAppleId();
              setEmail("");
              setPassword("");
              setHasPassword(false);
              setLegacyMessage({ tone: "success", text: "Đã xóa" });
            } catch (error) {
              setLegacyMessage({ tone: "error", text: describeError(error) });
            }
          }}
        >
          Xóa
        </button>
      </div>
    </section>
  );
}
