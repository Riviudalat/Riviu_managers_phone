import { useEffect, useRef, useState } from "react";
import { flowConnectorInfo, flowConnectorImportFile, flowConnectorSaveSecret, type FlowConnectorInfo } from "../../api";
import { describeError } from "../../describeError";
import "./flow-connector-tools.css";

/** Configuration only. A Flow run owns every connector dispatch and its ledger. */
export function FlowConnectorTools() {
  const [info, setInfo] = useState<FlowConnectorInfo | null>(null);
  const [name, setName] = useState("");
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const generation = useRef(0);
  const pending = useRef(false);
  useEffect(() => {
    const ticket = ++generation.current;
    flowConnectorInfo().then(result => { if (ticket === generation.current) setInfo(result); })
      .catch(cause => { if (ticket === generation.current) setError(describeError(cause)); });
    return () => { generation.current += 1; };
  }, []);

  const run = async (action: () => Promise<void>, message: string) => {
    if (pending.current) return;
    pending.current = true;
    const ticket = ++generation.current;
    setBusy(true); setError(null); setStatus(null);
    try {
      await action();
      const result = await flowConnectorInfo();
      if (ticket === generation.current) { setInfo(result); setStatus(message); }
    } catch (cause) { if (ticket === generation.current) setError(describeError(cause)); }
    finally { if (ticket === generation.current) { pending.current = false; setBusy(false); } }
  };

  return <details className="flow-connector-tools">
    <summary>Kết nối dữ liệu: tệp, HTTP và Google Sheet</summary>
    <p>Flow đọc và ghi tệp trong thư mục <code>{info?.root ?? "flow-data"}</code>. Dùng đường dẫn tương đối, ví dụ <code>inputs/posts.csv</code>.</p>
    <label className="flow-field">Nhập tệp UTF-8 (tối đa 4.096 ký tự)
      <input type="file" accept=".txt,.csv,.json" disabled={busy} onChange={event => {
        const file = event.currentTarget.files?.[0]; event.currentTarget.value = "";
        if (!file) return;
        void run(async () => {
          if (file.size > 16384) throw new Error("Tệp vượt quá 16 KiB.");
          const content = await file.text();
          if ([...content].length > 4096) throw new Error("Tệp vượt quá 4.096 ký tự.");
          await flowConnectorImportFile(file.name, content);
        }, `Đã nhập ${file.name} và đọc lại khớp nội dung.`);
      }} />
    </label>
    <p>Token HTTP lưu trong kho thông tin xác thực của hệ điều hành. Bước HTTP chỉ lưu tên tham chiếu và gửi token dạng Bearer.</p>
    <div className="flow-connector-credentials">
      <label className="flow-field">Tên tham chiếu
        <input value={name} maxLength={64} disabled={busy} onChange={event => setName(event.target.value)} autoComplete="off" placeholder="api_quan" />
      </label>
      <label className="flow-field">Token
        <input type="password" value={secret} disabled={busy} onChange={event => setSecret(event.target.value)} autoComplete="new-password" />
      </label>
      <button type="button" disabled={busy || !/^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(name) || !secret} onClick={() => {
        const value = secret; setSecret("");
        void run(() => flowConnectorSaveSecret(name, value), `Đã lưu tham chiếu ${name}.`);
      }}>Lưu token</button>
    </div>
    {!!info?.credentialNames.length && <ul>{info.credentialNames.map(reference => <li key={reference}>
      <code>{reference}</code>{" "}<button type="button" disabled={busy} onClick={() => void run(() => flowConnectorSaveSecret(reference, ""), `Đã xóa tham chiếu ${reference}.`)}>Xóa</button>
    </li>)}</ul>}
    <p>{info?.sheetConfigured ? "Google Sheet dùng kết nối đã cấu hình ở Đăng bài." : "Thiết lập kết nối Google Sheet ở Đăng bài trước khi chạy bước Sheet."} Mỗi bước chọn rõ link, tên tab và vùng ô. Cập nhật triển khai Apps Script bằng bản đi kèm để bật đọc/ghi vùng.</p>
    <p>Tệp CSV trả bảng JSON gồm các hàng chuỗi. Ghi CSV và Sheet nhận cùng định dạng, ví dụ <code>[["Máy 1","Sẵn sàng"]]</code>. Các bước sau dùng biến kết quả của bước trước.</p>
    {error && <p role="alert">{error}</p>}
    {status && <p role="status">{status}</p>}
  </details>;
}
