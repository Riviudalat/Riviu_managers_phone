import { useEffect, useState } from "react";
import { clearAppleId, driverMode, getAppleId, setAppleId } from "../api";

export function SettingsPanel() {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [hasPassword, setHasPassword] = useState(false);
  const [mode, setMode] = useState("…");
  const [msg, setMsg] = useState<string | null>(null);

  useEffect(() => {
    getAppleId()
      .then((cfg) => {
        setEmail(cfg.email);
        setHasPassword(cfg.hasPassword);
      })
      .catch(() => undefined);
    driverMode().then(setMode).catch(() => setMode("unknown"));
  }, []);

  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Settings</h2>
      </header>
      <section className="settings-card">
        <h3>Driver</h3>
        <p>
          Active mode: <code>{mode}</code>
        </p>
        <p className="hint">
          Mặc định kết nối <strong>thiết bị thật</strong> qua{" "}
          <code>pymobiledevice3</code>. Chỉ dùng mock khi set{" "}
          <code>RIVIU_MOCK_DEVICES=1</code> (dev).
        </p>
      </section>
      <section className="settings-card">
        <h3>On-iPhone agent</h3>
        <p className="hint">
          App cài lên iPhone tên <strong>Riviumanagersphone</strong>, icon chữ R cam.
          Bundle id: <code>com.riviu.managersphone.agent</code>.
        </p>
        <p className="hint">
          <strong>Bắt buộc một lần:</strong> cài <strong>Xcode</strong> từ App Store → mở
          Xcode → Settings → Accounts → thêm Apple ID (free) → Manage Certificates → thêm{" "}
          <em>Apple Development</em>. Sau đó dùng nút{" "}
          <strong>Cài / Re-sign Riviumanagersphone</strong> trên Control Center.
        </p>
        <button
          type="button"
          className="primary"
          onClick={() => {
            // Best-effort open App Store; desktop shell can ignore failures.
            window.open(
              "https://apps.apple.com/app/xcode/id497799835",
              "_blank",
              "noopener,noreferrer",
            );
          }}
        >
          Mở trang Xcode trên App Store
        </button>
      </section>
      <section className="settings-card">
        <h3>Apple ID (free signing)</h3>
        <p className="hint">
          Lưu trong OS keychain. Dùng để ký agent Riviumanagersphone trước khi cài lên máy.
        </p>
        <label>
          Email
          <input value={email} onChange={(e) => setEmail(e.target.value)} />
        </label>
        <label>
          Password {hasPassword ? "(saved)" : ""}
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={hasPassword ? "••••••••" : ""}
          />
        </label>
        {msg && <p className="hint">{msg}</p>}
        <div className="row">
          <button
            type="button"
            className="primary"
            onClick={async () => {
              await setAppleId(email, password);
              setHasPassword(true);
              setPassword("");
              setMsg("Saved to keychain");
            }}
          >
            Save
          </button>
          <button
            type="button"
            className="ghost"
            onClick={async () => {
              await clearAppleId();
              setEmail("");
              setPassword("");
              setHasPassword(false);
              setMsg("Cleared");
            }}
          >
            Clear
          </button>
        </div>
      </section>
    </div>
  );
}
