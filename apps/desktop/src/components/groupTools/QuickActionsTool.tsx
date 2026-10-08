import { setWallpaperSync, retryWallpaperSync, useWallpaperSyncState, applyNumberWallpaper, applyCustomWallpaper } from "../../wallpaperSync";
import { useState } from "react";
import type { HardwareKey } from "../../types";
import { groupInput, listDeviceMetas, setScreenLocked } from "../../api";
import { pickFiles } from "../../pickFile";
import { groupInputOutcome } from "../../groupInput";
import { getGroupSync } from "../../groupSync";
import { pushToast, toastError } from "../../toastStore";
import { fanOutReached, fanOutReasons } from "../../fanout";

export function QuickActionsTool({ targets, scopeLabel }: { targets: string[]; scopeLabel: string }) {
  const wallpaperSync = useWallpaperSyncState();
  const [busy, setBusy] = useState<string | null>(null);

  const KEYS: { label: string; key: HardwareKey }[] = [
    { label: "Home", key: "home" },
    { label: "Back", key: "back" },
    { label: "Đa nhiệm", key: "recents" },
    { label: "Nguồn (khoá/mở)", key: "power" },
    { label: "Âm lượng +", key: "volumeUp" },
    { label: "Âm lượng −", key: "volumeDown" },
    { label: "Thông báo", key: "notification" },
  ];

  const fire = async (label: string, key: HardwareKey) => {
    if (!targets.length) {
      pushToast("warn", "Chưa có máy", "Chọn máy rồi thao tác.");
      return;
    }
    setBusy(key);
    try {
      const report = await groupInput({ udids: targets, kind: "key", key, sync: getGroupSync() });
      const outcome = groupInputOutcome(report);
      if (outcome.kind === "ok") pushToast("ok", label, `${targets.length} máy`);
      else if (outcome.kind === "partial") pushToast("warn", outcome.title, outcome.detail);
      else pushToast("error", outcome.title, outcome.detail);
    } catch (e) {
      toastError(`${label} thất bại`, e);
    } finally {
      setBusy(null);
    }
  };

  const numberWallpapers = async () => {
    if (!targets.length) {
      pushToast("warn", "Chưa có máy", "Chọn máy rồi đặt hình nền.");
      return;
    }
    setBusy("wall-num");
    try {
      // Read committed numbers once for the operation, including devices outside this selection.
      const metas = new Map((await listDeviceMetas()).map(meta => [meta.udid, meta]));
      const results = await Promise.allSettled(targets.map(async udid => {
        const number = metas.get(udid)?.number;
        if (number == null) throw new Error("Máy chưa có số đã lưu; cập nhật danh sách rồi thử lại.");
        await applyNumberWallpaper(udid, number);
      }));
      const ok = fanOutReached(results);
      pushToast(ok === targets.length ? "ok" : "warn", `Đã đặt hình nền ${ok}/${targets.length} máy`, fanOutReasons(targets, results) ?? undefined);
    } catch (error) {
      toastError("Chưa đặt được hình nền theo số máy", error);
    } finally {
      setBusy(null);
    }
  };

  const lock = async (locked: boolean) => {
    if (!targets.length) {
      pushToast("warn", "Chưa có máy", "Chọn máy rồi thao tác.");
      return;
    }
    setBusy(locked ? "lock" : "unlock");
    const results = await Promise.allSettled(targets.map((u) => setScreenLocked(u, locked)));
    setBusy(null);
    const ok = fanOutReached(results);
    const label = locked ? "Đã khoá màn hình" : "Đã mở khoá";
    if (ok === targets.length) pushToast("ok", label, `${ok} máy`);
    else
      pushToast(
        "warn",
        `${label} ${ok}/${targets.length} máy`,
        fanOutReasons(targets, results) ?? "Máy còn lại không hỗ trợ hoặc bận.",
      );
  };

  const customWallpaper = async () => {
    if (!targets.length) {
      pushToast("warn", "Chưa có máy", "Chọn máy rồi đặt hình nền.");
      return;
    }
    const picked = await pickFiles({
      title: "Chọn ảnh nền chung",
      filters: [{ name: "Ảnh", extensions: ["jpg", "jpeg", "png", "webp"] }],
    });
    if (!picked.length) return;
    const path = picked[0];
    setBusy("wall-img");
    const results = await Promise.allSettled(targets.map((udid) => applyCustomWallpaper(udid, path)));
    setBusy(null);
    const ok = fanOutReached(results);
    if (ok === targets.length) pushToast("ok", "Đã đặt ảnh nền", `${ok} máy`);
    else
      pushToast(
        "warn",
        `Đặt ảnh nền ${ok}/${targets.length} máy`,
        fanOutReasons(targets, results) ?? "Máy còn lại cần Riviu helper.",
      );
  };

  return (
    <>
      <p className="hint">
        Bấm một phím phần cứng cho {scopeLabel} cùng lúc (áp cả độ trễ/so le nếu đã bật ở Cài
        đặt). "Nguồn" bật/tắt màn hình luân phiên.
      </p>
      <div className="group-tools-keys">
        {KEYS.map((k) => (
          <button
            type="button"
            key={k.key}
            className="tb-btn"
            disabled={busy !== null}
            onClick={() => void fire(k.label, k.key)}
          >
            {busy === k.key ? "…" : k.label}
          </button>
        ))}
      </div>
      <p className="hint" style={{ marginTop: "0.7rem" }}>
        Khoá / mở khoá màn hình đồng loạt (iOS qua WDA; Android tắt/bật màn hình). Máy đặt mã
        PIN sẽ dừng ở màn khoá của nó — đây là bật/tắt màn, không phải vượt khoá.
      </p>
      <div className="nurture-float-actions">
        <button type="button" className="ghost" disabled={busy !== null} onClick={() => void lock(true)}>
          {busy === "lock" ? "…" : "Khoá màn hình"}
        </button>
        <button type="button" className="ghost" disabled={busy !== null} onClick={() => void lock(false)}>
          {busy === "unlock" ? "…" : "Mở khoá"}
        </button>
      </div>
      <p className="hint" style={{ marginTop: "0.7rem" }}>
        Hình nền (Android, cần Riviu helper) — đánh số máy để nhận diện, hoặc đặt một ảnh
        chung.
      </p>
      <label><input type="checkbox" checked={targets.length > 0 && targets.every(udid => wallpaperSync[udid]?.enabled)}
        onChange={event => { try { setWallpaperSync(targets, event.target.checked); } catch (error) { toastError("Chưa lưu được đồng bộ hình nền", error); } }} /> Đồng bộ hình nền theo số máy</label>
      <p className="hint">Áp dụng khi máy rảnh; máy ngoại tuyến chờ kết nối lại. Logo R ở trên, số đã lưu ở dưới.</p>
      {targets.some(udid => ["pending", "needsReview"].includes(wallpaperSync[udid]?.state ?? "")) && <details>
        <summary>Hình nền cần kiểm tra</summary>
        <p>Lần đặt trước chưa xác nhận hoàn tất. Kiểm tra điện thoại trước khi thử lại.</p>
        <button type="button" onClick={() => { try { retryWallpaperSync(targets); } catch (error) { toastError("Chưa lưu được yêu cầu thử lại", error); } }}>Thử lại đồng bộ hình nền</button>
      </details>}
      <div className="nurture-float-actions">
        <button
          type="button"
          className="ghost"
          disabled={busy !== null}
          onClick={() => void numberWallpapers()}
        >
          {busy === "wall-num" ? "…" : "Đặt số làm hình nền"}
        </button>
        <button
          type="button"
          className="ghost"
          disabled={busy !== null}
          onClick={() => void customWallpaper()}
        >
          {busy === "wall-img" ? "…" : "Chọn ảnh nền chung…"}
        </button>
      </div>
    </>
  );
}
