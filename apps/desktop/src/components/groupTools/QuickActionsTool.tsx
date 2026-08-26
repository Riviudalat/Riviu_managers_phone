import { useState } from "react";
import type { HardwareKey } from "../../types";
import { groupInput, setScreenLocked, setWallpaper, setWallpaperBytes } from "../../api";
import { pickFiles } from "../../pickFile";
import { groupInputOutcome } from "../../groupInput";
import { getGroupSync } from "../../groupSync";
import { pushToast, toastError } from "../../toastStore";
import { fanOutReached, fanOutReasons } from "../../fanout";

/** Render a tall PNG with a big number centred, for "set number as wallpaper" (A3). */
async function numberWallpaperPng(label: string): Promise<Uint8Array> {
  const canvas = document.createElement("canvas");
  canvas.width = 1080;
  canvas.height = 1920;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("no 2d context");
  ctx.fillStyle = "#0b0b0f";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.fillStyle = "#ff6a00";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.font = "bold 620px system-ui, sans-serif";
  ctx.fillText(label, canvas.width / 2, canvas.height / 2);
  const blob: Blob = await new Promise((resolve, reject) =>
    canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("toBlob failed"))), "image/png"),
  );
  return new Uint8Array(await blob.arrayBuffer());
}

export function QuickActionsTool({ targets, scopeLabel }: { targets: string[]; scopeLabel: string }) {
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
    const results = await Promise.allSettled(
      targets.map(async (udid, i) => {
        const png = await numberWallpaperPng(String(i + 1));
        await setWallpaperBytes(udid, Array.from(png));
      }),
    );
    setBusy(null);
    const ok = fanOutReached(results);
    if (ok === targets.length) pushToast("ok", "Đã đặt số làm hình nền", `${ok} máy`);
    else
      pushToast(
        "warn",
        `Đặt hình nền ${ok}/${targets.length} máy`,
        // Was a fixed guess about the helper. Often right, and when wrong it hid the sentence
        // the phone actually returned.
        fanOutReasons(targets, results) ?? "Máy còn lại cần Riviu helper.",
      );
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
    const results = await Promise.allSettled(targets.map((udid) => setWallpaper(udid, path)));
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
