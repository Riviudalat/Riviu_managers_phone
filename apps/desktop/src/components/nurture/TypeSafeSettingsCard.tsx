import { useEffect, useState } from "react";
import { typesafeCheckComment, typesafeGetSettings, typesafeUpdateCredential, typesafeUpdateSettings } from "../../api";
import type { TypeSafeSettings } from "../../generated-ipc";
import { describeError } from "../../describeError";

export function TypeSafeSettingsCard() {
  const [settings, setSettings] = useState<TypeSafeSettings | null>(null);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  useEffect(() => {
    let active = true;
    void typesafeGetSettings().then(value => { if (active) setSettings(value); })
      .catch(error => { if (active) setMessage(describeError(error)); });
    return () => { active = false; };
  }, []);
  async function act(action: () => Promise<void>) {
    setBusy(true); setMessage("");
    try { await action(); }
    catch (error) {
      setMessage(describeError(error));
      // Recover the current revision after a competing settings writer.
      try { setSettings(await typesafeGetSettings()); } catch { /* Keep the original error. */ }
    } finally { setBusy(false); }
  }
  return <fieldset disabled={busy} className="nurture-sect">
    <legend>TypeSafe — kiểm bằng chứng chữ</legend>
    <p>Đối chiếu bình luận AI với caption và lời thoại trước khi gửi. Kiểm ảnh và đúng tài khoản vẫn chạy riêng. Khi bật, lỗi dịch vụ sẽ dừng lượt bình luận đó.</p>
    <label style={{ flexDirection: "row", alignItems: "center", gap: "0.5rem" }}>
      <input type="checkbox" style={{ width: 16, height: 16, margin: 0 }} checked={settings?.enabled ?? false} disabled={!settings}
        onChange={event => { const enabled = event.target.checked; if (settings) void act(async () => { setSettings(await typesafeUpdateSettings(enabled, settings.revision)); }); }} />
      Bật kiểm TypeSafe cho bình luận AI
    </label>
    <label>Khóa TypeSafe
      <input type="password" autoComplete="off" value={key} onChange={event => setKey(event.target.value)}
        placeholder={settings?.hasApiKey ? "Đã lưu khóa trong hệ điều hành" : "Chưa có khóa"} />
    </label>
    <div className="nurture-row">
      <button type="button" disabled={!key.trim()} onClick={() => void act(async () => { setSettings(await typesafeUpdateCredential(key)); setKey(""); setMessage("Đã lưu khóa TypeSafe."); })}>Lưu khóa TypeSafe</button>
      <button type="button" disabled={!settings?.hasApiKey} onClick={() => void act(async () => { setSettings(await typesafeUpdateCredential("")); setKey(""); setMessage("Đã xóa khóa TypeSafe."); })}>Xóa khóa</button>
      <button type="button" disabled={!settings?.hasApiKey} onClick={() => void act(async () => {
        const result = await typesafeCheckComment("Quán phở này ở đâu vậy?", "Hôm nay ghé quán phở bò ở Đà Lạt.");
        setMessage(`TypeSafe: ${result.support === "supported" ? "mẫu kiểm có bằng chứng phù hợp" : "mẫu kiểm chưa đạt"} · ${result.elapsedMs} ms · ${result.inputTokens + result.outputTokens} token.`);
      })}>Kiểm tra bằng mẫu chữ</button>
    </div>
    <small>Chỉ gửi câu bình luận, caption và lời thoại cần đối chiếu đến TypeSafe. Phép kiểm mẫu gọi API, không thao tác điện thoại. Chi phí TypeSafe chưa có số tiền đối soát.</small>
    {message && <p role="status">{message}</p>}
  </fieldset>;
}
