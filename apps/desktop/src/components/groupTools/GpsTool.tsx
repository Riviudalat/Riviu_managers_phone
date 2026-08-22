import { useState } from "react";
import { setMockLocation, stopMockLocation } from "../../api";
import { pushToast } from "../../toastStore";

export function GpsTool({ targets, scopeLabel }: { targets: string[]; scopeLabel: string }) {
  const [coords, setCoords] = useState("");
  const [busy, setBusy] = useState(false);

  const parse = (): { lat: number; lng: number } | null => {
    const nums = coords
      .split(/[,\s]+/)
      .map(Number)
      .filter((n) => Number.isFinite(n));
    if (nums.length < 2) return null;
    const [lat, lng] = nums;
    if (Math.abs(lat) > 90 || Math.abs(lng) > 180) return null;
    return { lat, lng };
  };

  const apply = async () => {
    const c = parse();
    if (!c) {
      pushToast("warn", "Toạ độ không hợp lệ", "Nhập dạng: 21.028511, 105.804817");
      return;
    }
    if (!targets.length) {
      pushToast("warn", "Chưa có máy", "Chọn máy rồi đặt.");
      return;
    }
    setBusy(true);
    const results = await Promise.allSettled(targets.map((u) => setMockLocation(u, c.lat, c.lng)));
    setBusy(false);
    const ok = results.filter((r) => r.status === "fulfilled").length;
    if (ok === targets.length) pushToast("ok", "Đã đặt vị trí", `${ok} máy`);
    else
      pushToast(
        "warn",
        `Đặt vị trí ${ok}/${targets.length} máy`,
        "Máy còn lại cần Riviu helper + quyền mock-location.",
      );
  };

  const stop = async () => {
    if (!targets.length) return;
    setBusy(true);
    await Promise.allSettled(targets.map((u) => stopMockLocation(u)));
    setBusy(false);
    pushToast("ok", "Đã tắt giả lập vị trí", `${targets.length} máy`);
  };

  return (
    <>
      <p className="hint">
        Giả lập vị trí GPS cho {scopeLabel} (Android, cần Riviu helper — cấp quyền
        mock-location tự động). Copy toạ độ từ Google Maps (chuột phải → bấm vào toạ độ để
        chép) rồi dán vào đây.
      </p>
      <div className="row">
        <label style={{ flex: 1 }}>
          Toạ độ (vĩ độ, kinh độ)
          <input
            type="text"
            placeholder="21.028511, 105.804817"
            value={coords}
            onChange={(e) => setCoords(e.target.value)}
          />
        </label>
      </div>
      <div className="nurture-float-actions">
        <button type="button" className="primary" disabled={busy} onClick={() => void apply()}>
          {busy ? "Đang đặt…" : "Đặt vị trí"}
        </button>
        <button type="button" className="ghost" disabled={busy} onClick={() => void stop()}>
          Tắt giả lập
        </button>
      </div>
    </>
  );
}
