import { useEffect, useState } from "react";
import { clearAppleId, getAppleId, setAppleId } from "../../api";
import { describeError } from "../../describeError";

/** Apple ID for the legacy stock-agent path. */
export function LegacyAgentSection() {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [hasPassword, setHasPassword] = useState(false);
  const [legacyMessage, setLegacyMessage] = useState<string | null>(null);

  useEffect(() => {
    getAppleId()
      .then((config) => {
        setEmail(config.email);
        setHasPassword(config.hasPassword);
      })
      .catch(() => undefined);
  }, []);
  return (
    <section className="settings-section">
      <h3>Khôi phục agent iOS cũ</h3>
      <p className="hint">
        Thông tin này chỉ dùng khi cần quay lại agent iOS dự phòng; đường điều khiển chính không đọc nó.
      </p>
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
      {legacyMessage && <p className="hint">{legacyMessage}</p>}
      <div className="row">
        <button
          type="button"
          className="primary"
          onClick={async () => {
            try {
              await setAppleId(email, password);
              setHasPassword(true);
              setPassword("");
              setLegacyMessage("Đã lưu trong kho thông tin xác thực của Windows");
            } catch (error) {
              setLegacyMessage(describeError(error));
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
              setLegacyMessage("Đã xóa");
            } catch (error) {
              setLegacyMessage(describeError(error));
            }
          }}
        >
          Xóa
        </button>
      </div>
    </section>
  );
}
