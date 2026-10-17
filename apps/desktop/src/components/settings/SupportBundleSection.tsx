import { useState } from "react";
import { supportBundleExport } from "../../api";
import { describeError } from "../../describeError";

/** One zip with everything support needs: logs, a database snapshot, recent evidence and the fleet. */
export function SupportBundleSection() {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  return (
    <section className="settings-section" aria-label="Trích xuất log">
      <h3>Trích xuất log</h3>
      <p className="hint">
        Gom log của app, bản chụp database (chiến dịch, từng lượt đăng, xác minh, Sheet, lịch), danh sách máy và ảnh bằng chứng 48 giờ gần nhất vào một file .zip trong thư mục Riviu-log trên Desktop. Mật khẩu, cookie và token không bao giờ có trong file. Có thể mất vài phút; điện thoại vẫn chạy bình thường.
      </p>
      <div className="settings-actions">
        <button
          type="button"
          className="primary"
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            setMessage(null);
            try {
              setMessage(await supportBundleExport());
            } catch (error) {
              setMessage(describeError(error));
            } finally {
              setBusy(false);
            }
          }}
        >
          {busy ? "Đang xuất..." : "Trích xuất log"}
        </button>
      </div>
      {message && <p role="status">{message}</p>}
    </section>
  );
}
