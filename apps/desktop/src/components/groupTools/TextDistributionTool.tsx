import { useMemo, useState } from "react";
import type { DeviceInfo } from "../../types";
import { distributeText } from "../../api";
import { assign, leftover, splitText, type SplitMode } from "../../textDistribution";
import { groupInputOutcome } from "../../groupInput";
import { pushToast, toastError } from "../../toastStore";
import { describeError } from "../../describeError";

export function TextDistributionTool({
  devices,
  targets,
  targetDevices,
}: {
  devices: DeviceInfo[];
  targets: string[];
  targetDevices: DeviceInfo[];
}) {
  const [raw, setRaw] = useState("");
  const [modeKind, setModeKind] = useState<SplitMode["kind"]>("lines");
  const [separator, setSeparator] = useState(",");
  const [pattern, setPattern] = useState("\\s*\\n\\s*");
  const [busy, setBusy] = useState(false);

  const mode: SplitMode = useMemo(() => {
    if (modeKind === "separator") return { kind: "separator", separator };
    if (modeKind === "regex") return { kind: "regex", pattern };
    return { kind: "lines" };
  }, [modeKind, separator, pattern]);

  const { items, error } = useMemo(() => {
    try {
      return { items: splitText(raw, mode), error: null as string | null };
    } catch (e) {
      return { items: [] as string[], error: describeError(e) };
    }
  }, [raw, mode]);

  const pairs = useMemo(() => assign(items, targets), [items, targets]);
  const spare = useMemo(() => leftover(items, targets), [items, targets]);

  const nameFor = (udid: string): string => {
    const d = devices.find((x) => x.udid === udid);
    return d?.name || d?.model || udid.slice(-6);
  };

  const send = async () => {
    if (!pairs.length) {
      pushToast("warn", "Chưa có gì để gửi", "Cần văn bản đã tách và ít nhất một máy.");
      return;
    }
    setBusy(true);
    try {
      const report = await distributeText(pairs);
      const outcome = groupInputOutcome(report);
      if (outcome.kind === "ok") pushToast("ok", "Đã phân phối văn bản", `${pairs.length} máy`);
      else if (outcome.kind === "partial") pushToast("warn", outcome.title, outcome.detail);
      else pushToast("error", outcome.title, outcome.detail);
    } catch (e) {
      toastError("Phân phối văn bản thất bại", e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <p className="hint">
        Chia một khối văn bản thành nhiều phần và gõ mỗi phần vào một máy, theo thứ tự máy đang
        hiển thị. Đường gõ Unicode (dấu tiếng Việt an toàn), chạy cả Android lẫn iOS.
      </p>
      <div className="group-tools-field">
        <label className="hint" htmlFor="gt-text">
          Văn bản
        </label>
        <textarea
          id="gt-text"
          value={raw}
          placeholder={"Mỗi dòng một máy…\nXin chào\nBạn khỏe không"}
          onChange={(e) => setRaw(e.target.value)}
        />
      </div>
      <div className="row">
        <label>
          Cách tách
          <select value={modeKind} onChange={(e) => setModeKind(e.target.value as SplitMode["kind"])}>
            <option value="lines">Theo dòng</option>
            <option value="separator">Ký tự phân tách</option>
            <option value="regex">Biểu thức chính quy</option>
          </select>
        </label>
        {modeKind === "separator" && (
          <label>
            Ký tự
            <input type="text" value={separator} onChange={(e) => setSeparator(e.target.value)} />
          </label>
        )}
        {modeKind === "regex" && (
          <label>
            Mẫu regex
            <input type="text" value={pattern} onChange={(e) => setPattern(e.target.value)} />
          </label>
        )}
      </div>
      {error && <p className="error">Regex không hợp lệ: {error}</p>}
      <p className="hint">
        {items.length} phần → {pairs.length}/{targetDevices.length} máy nhận
        {spare.extraItems > 0 && ` · dư ${spare.extraItems} phần`}
        {spare.extraDevices > 0 && ` · thiếu cho ${spare.extraDevices} máy`}
      </p>
      {pairs.length > 0 && (
        <div className="group-tools-preview">
          {pairs.map((p, i) => (
            <div className="row-item" key={p.udid}>
              <span className="who">
                #{i + 1} {nameFor(p.udid)}
              </span>
              <span className="what">{p.text}</span>
            </div>
          ))}
        </div>
      )}
      <div className="nurture-float-actions" style={{ marginTop: "0.7rem" }}>
        <button
          type="button"
          className="primary"
          disabled={busy || !pairs.length || Boolean(error)}
          onClick={() => void send()}
        >
          {busy ? "Đang gửi…" : `Gửi tới ${pairs.length} máy`}
        </button>
      </div>
    </>
  );
}
