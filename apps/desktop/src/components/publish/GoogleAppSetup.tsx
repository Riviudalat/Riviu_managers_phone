import { useState } from "react";
import type { GoogleSheetsConfiguration } from "../../types";

type Props = {
  clientId: string;
  busy: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (config: GoogleSheetsConfiguration) => Promise<boolean>;
};

export function GoogleAppSetup({ clientId, busy, open, onOpenChange, onSave }: Props) {
  const [client, setClient] = useState(clientId);
  const [secret, setSecret] = useState("");
  const [key, setKey] = useState("");
  const [project, setProject] = useState("");
  const save = async () => {
    if (await onSave({ clientId: client.trim(), clientSecret: secret.trim() || undefined,
      pickerApiKey: key.trim(), projectNumber: project.trim() })) {
      setSecret(""); setKey("");
    }
  };
  return <details className="google-app-setup" open={open} onToggle={event => onOpenChange(event.currentTarget.open)}>
    <summary>Thiết lập Google</summary>
    <p>Bản app này chưa có đủ cấu hình Google. Nhập thông tin ứng dụng do quản trị viên cung cấp một lần trên máy này, sau đó đăng nhập Google.</p>
    <fieldset disabled={busy}>
      <label>OAuth Client ID (Desktop)<input aria-label="OAuth Client ID (Desktop)" autoComplete="off" value={client}
        onChange={event => setClient(event.target.value)} placeholder="…apps.googleusercontent.com" /></label>
      <label>Client secret<input aria-label="Client secret" type="password" autoComplete="new-password" value={secret}
        onChange={event => setSecret(event.target.value)} placeholder="Nếu ứng dụng có cung cấp" /></label>
      <label>Google Picker API key (tùy chọn)<input aria-label="Google Picker API key" type="password" autoComplete="new-password" value={key}
        onChange={event => setKey(event.target.value)} /></label>
      <label>Google Cloud project number (tùy chọn, đi cùng Picker key)<input aria-label="Google Cloud project number" inputMode="numeric" autoComplete="off" value={project}
        onChange={event => setProject(event.target.value)} /></label>
      <button type="button" disabled={busy || !client.trim() || (!!project.trim() && !/^\d{1,32}$/.test(project.trim())) || Boolean(key.trim()) !== Boolean(project.trim())} onClick={() => void save()}>
        {busy ? "Đang lưu…" : "Lưu cấu hình Google"}
      </button>
    </fieldset>
    <p>Đây là cấu hình ứng dụng, không phải mật khẩu tài khoản Google. Phiên đăng nhập được lưu riêng trên từng máy.</p>
  </details>;
}
