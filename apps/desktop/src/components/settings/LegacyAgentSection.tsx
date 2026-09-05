import { useCallback, useEffect, useRef, useState } from "react";
import { Save } from "lucide-react";
import { useWorkspaceDraft } from "../../workspaceDraft";
import { clearAppleId, getAppleId, setAppleId } from "../../api";
import { describeError } from "../../describeError";
import { LoadingState, StatusNotice, type NoticeTone } from "../States";

/** Apple ID for the legacy stock-agent path. */
export function LegacyAgentSection() {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [hasPassword, setHasPassword] = useState(false);
  const [savedEmail, setSavedEmail] = useState("");
  const [saving, setSaving] = useState(false);
  const epoch = useRef(0);
  const savingRef = useRef(false);
  const [legacyMessage, setLegacyMessage] = useState<{ tone: NoticeTone; text: string } | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  const load = useCallback(() => {
    setLoading(true);
    setLoadError(null);
    getAppleId()
      .then((config) => {
        setEmail(config.email);
        setSavedEmail(config.email);
        setHasPassword(config.hasPassword);
      })
      .catch((error) => setLoadError(describeError(error)))
      .finally(() => setLoading(false));
  }, []);

  useEffect(load, [load]);
  const dirty = !loading && (email !== savedEmail || password.length > 0);
  const discard = () => { epoch.current += 1; setEmail(savedEmail); setPassword(""); };
  const save = async () => {
    if (savingRef.current || loading || loadError) return false;
    if (!email.trim() || (!hasPassword && !password)) {
      setLegacyMessage({ tone: "error", text: "Nhập email và mật khẩu trước khi lưu." });
      return false;
    }
    const editEpoch = epoch.current;
    savingRef.current = true;
    setSaving(true);
    try {
      await setAppleId(email, password);
      setSavedEmail(email);
      setHasPassword(true);
      if (editEpoch === epoch.current) {
        setPassword("");
        setLegacyMessage({ tone: "success", text: "Đã lưu trong kho thông tin xác thực của Windows" });
      }
      return editEpoch === epoch.current;
    } catch (error) {
      setLegacyMessage({ tone: "error", text: describeError(error) });
      return false;
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  };
  useWorkspaceDraft({ id: "settings-ios", label: "Thông tin iOS dự phòng", dirty, snapshotKey: String(epoch.current), save, discard });
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
        <input value={email} disabled={loading || Boolean(loadError)} onChange={(event) => { epoch.current += 1; setEmail(event.target.value); }} />
      </label>
      <label>
        Mật khẩu {hasPassword ? "(đã lưu)" : ""}
        <input
          type="password"
          value={password}
          disabled={loading || Boolean(loadError)}
          onChange={(event) => { epoch.current += 1; setPassword(event.target.value); }}
          placeholder={hasPassword ? "••••••••" : ""}
        />
      </label>
      {legacyMessage && <StatusNotice tone={legacyMessage.tone}>{legacyMessage.text}</StatusNotice>}
      <div className="row">
        <button
          type="button"
          className="primary"
          disabled={!dirty || saving}
          onClick={() => void save()}
        >
          <Save size={15} />{saving ? "Đang lưu…" : "Lưu thông tin iOS"}
        </button>
        <button
          type="button"
          className="ghost"
          disabled={loading || saving || !hasPassword}
          onClick={async () => {
            const editEpoch = epoch.current;
            savingRef.current = true;
            setSaving(true);
            try {
              await clearAppleId();
              setSavedEmail("");
              setHasPassword(false);
              if (editEpoch === epoch.current) {
                setEmail("");
                setPassword("");
                setLegacyMessage({ tone: "success", text: "Đã xóa" });
              }
            } catch (error) {
              setLegacyMessage({ tone: "error", text: describeError(error) });
            } finally {
              savingRef.current = false;
              setSaving(false);
            }
          }}
        >
          Xóa
        </button>
        {dirty && <button type="button" className="ghost" disabled={saving} onClick={discard}>Bỏ thay đổi</button>}
      </div>
    </section>
  );
}
