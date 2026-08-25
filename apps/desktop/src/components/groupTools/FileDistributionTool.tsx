import { useMemo, useState } from "react";
import type { DeviceInfo } from "../../types";
import { distributeFiles } from "../../api";
import { pickFiles } from "../../pickFile";
import { assign, leftover } from "../../textDistribution";
import { groupInputOutcome } from "../../groupInput";
import { pushToast, toastError } from "../../toastStore";

export function FileDistributionTool({
  devices,
  targets,
  targetDevices,
}: {
  devices: DeviceInfo[];
  targets: string[];
  targetDevices: DeviceInfo[];
}) {
  const [paths, setPaths] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  const pairs = useMemo(
    () => assign(paths, targets).map((a) => ({ udid: a.udid, path: a.text })),
    [paths, targets],
  );
  const spare = useMemo(() => leftover(paths, targets), [paths, targets]);

  const nameFor = (udid: string): string => {
    const d = devices.find((x) => x.udid === udid);
    return d?.name || d?.model || udid.slice(-6);
  };
  const baseName = (p: string): string => p.split(/[/\\]/).pop() || p;

  const pick = async () => {
    const picked = await pickFiles({
      title: "Chọn tệp phân phối (mỗi máy một tệp, theo thứ tự)",
      filters: [
        { name: "Media", extensions: ["jpg", "jpeg", "png", "gif", "webp", "heic", "mp4", "mov", "m4v"] },
        { name: "Tất cả", extensions: ["*"] },
      ],
    });
    if (picked.length) setPaths(picked);
  };

  const send = async () => {
    if (!pairs.length) {
      pushToast("warn", "Chưa có gì để gửi", "Chọn tệp và ít nhất một máy.");
      return;
    }
    setBusy(true);
    try {
      const report = await distributeFiles(pairs);
      const outcome = groupInputOutcome(report);
      if (outcome.kind === "ok") pushToast("ok", "Đã phân phối tệp", `${pairs.length} máy`);
      else if (outcome.kind === "partial") pushToast("warn", outcome.title, outcome.detail);
      else pushToast("error", outcome.title, outcome.detail);
    } catch (e) {
      toastError("Phân phối tệp thất bại", e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <p className="hint">
        Đưa mỗi tệp vào một máy (theo thứ tự máy đang hiển thị), lưu vào thư viện ảnh/video của
        máy. Chọn nhiều tệp một lần; tệp thứ i vào máy thứ i.
      </p>
      <div className="nurture-float-actions">
        <button type="button" className="ghost" onClick={() => void pick()}>
          Chọn tệp…
        </button>
      </div>
      <p className="hint">
        {paths.length} tệp → {pairs.length}/{targetDevices.length} máy nhận
        {spare.extraItems > 0 && ` · dư ${spare.extraItems} tệp`}
        {spare.extraDevices > 0 && ` · thiếu cho ${spare.extraDevices} máy`}
      </p>
      {pairs.length > 0 && (
        <div className="group-tools-preview">
          {pairs.map((p, i) => (
            <div className="row-item" key={p.udid}>
              <span className="who">
                #{i + 1} {nameFor(p.udid)}
              </span>
              <span className="what">{baseName(p.path)}</span>
            </div>
          ))}
        </div>
      )}
      <div className="nurture-float-actions" style={{ marginTop: "0.7rem" }}>
        <button type="button" className="primary" disabled={busy || !pairs.length} onClick={() => void send()}>
          {busy ? "Đang gửi…" : `Gửi tới ${pairs.length} máy`}
        </button>
      </div>
    </>
  );
}
