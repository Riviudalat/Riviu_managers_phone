import { useState } from "react";
import { factoryReset, isRooted, rootShell, setDeviceIdentity } from "../../api";
import { requestConfirm } from "../../confirmStore";
import { describeError } from "../../describeError";
import { pushToast } from "../../toastStore";
import { fanOutReasons } from "../../fanout";
import { randomAndroidId, randomMac, randomSerial } from "./randomIdentity";

/**
 * Root tier (C, xiaowei "ROOT 模式"): batch identity change ("一键新机"), factory reset and a
 * root shell, all scoped to the current selection. Each phone that lacks Magisk `su` reports
 * that per-field rather than half-applying — only Android ID changes without root (adb carries
 * WRITE_SECURE_SETTINGS). Every phone gets its *own* random identity: a farm of clones defeats
 * the point.
 */
export function RootTool({ targets, scopeLabel }: { targets: string[]; scopeLabel: string }) {
  const [rootStatus, setRootStatus] = useState<"idle" | "checking" | "done">("idle");
  const [rootedCount, setRootedCount] = useState(0);
  const [changeAndroidId, setChangeAndroidId] = useState(true);
  const [changeSerial, setChangeSerial] = useState(true);
  const [changeMac, setChangeMac] = useState(true);
  const [shellCmd, setShellCmd] = useState("");
  const [busy, setBusy] = useState<null | "probe" | "identity" | "reset" | "shell">(null);
  const [logText, setLogText] = useState("");

  const noTargets = () => {
    pushToast("warn", "Chưa có máy", "Chọn máy rồi thử lại.");
    return true;
  };

  const probe = async () => {
    if (!targets.length) return void noTargets();
    setBusy("probe");
    setRootStatus("checking");
    const results = await Promise.allSettled(targets.map((u) => isRooted(u)));
    setBusy(null);
    setRootedCount(results.filter((r) => r.status === "fulfilled" && r.value).length);
    setRootStatus("done");
    // **A phone that could not be asked is not a phone that answered no.** The count above
    // treated both the same, so "3/20 đã root" could equally have meant seventeen unrooted
    // phones or seventeen that failed to answer -- and those have completely different fixes.
    const unreachable = fanOutReasons(targets, results);
    if (unreachable) {
      pushToast("warn", "Có máy không trả lời được", unreachable);
    }
  };

  const applyIdentity = async () => {
    if (!targets.length) return void noTargets();
    if (!changeAndroidId && !changeSerial && !changeMac) {
      pushToast("warn", "Chưa chọn trường", "Tích ít nhất một mục để đổi.");
      return;
    }
    setBusy("identity");
    const results = await Promise.allSettled(
      targets.map((u) => {
        const identity: { androidId?: string; serialno?: string; mac?: string } = {};
        if (changeAndroidId) identity.androidId = randomAndroidId();
        if (changeSerial) identity.serialno = randomSerial();
        if (changeMac) identity.mac = randomMac();
        return setDeviceIdentity(u, identity);
      }),
    );
    setBusy(null);
    setLogText(
      results
        .map((r, i) =>
          r.status === "fulfilled" ? `${targets[i]}: ${r.value}` : `${targets[i]}: ✗ ${describeError(r.reason)}`,
        )
        .join("\n"),
    );
    const ok = results.filter((r) => r.status === "fulfilled").length;
    pushToast(ok === targets.length ? "ok" : "warn", "Đổi định danh", `${ok}/${targets.length} máy`);
  };

  const runReset = async () => {
    if (!targets.length) return void noTargets();
    const sure = await requestConfirm({
      title: `Khôi phục gốc ${targets.length} máy?`,
      message:
        "Toàn bộ dữ liệu trên các máy đã chọn sẽ bị xoá sạch và KHÔNG THỂ hoàn tác. Chỉ máy đã root mới thực thi được.",
      confirmLabel: "Khôi phục gốc",
      danger: true,
    });
    if (!sure) return;
    setBusy("reset");
    const results = await Promise.allSettled(targets.map((u) => factoryReset(u)));
    setBusy(null);
    setLogText(
      results
        .map((r, i) =>
          r.status === "fulfilled"
            ? `${targets[i]}: đã gửi lệnh khôi phục`
            : `${targets[i]}: ✗ ${describeError(r.reason)}`,
        )
        .join("\n"),
    );
    const ok = results.filter((r) => r.status === "fulfilled").length;
    if (ok === targets.length) pushToast("ok", "Đã gửi lệnh khôi phục gốc", `${ok} máy`);
    else
      // Was "Máy còn lại chưa root." — a guess standing in for what the phone said. A factory
      // reset can fail for several reasons and the operator is entitled to the real one.
      pushToast(
        "warn",
        `Khôi phục ${ok}/${targets.length} máy`,
        fanOutReasons(targets, results) ?? "Máy còn lại chưa root.",
      );
  };

  const runShell = async () => {
    const cmd = shellCmd.trim();
    if (!cmd) {
      pushToast("warn", "Chưa có lệnh", "Nhập lệnh shell rồi chạy.");
      return;
    }
    if (!targets.length) return void noTargets();
    setBusy("shell");
    const results = await Promise.allSettled(targets.map((u) => rootShell(u, cmd)));
    setBusy(null);
    setLogText(
      results
        .map((r, i) =>
          r.status === "fulfilled"
            ? `${targets[i]}:\n${r.value.trim() || "(trống)"}`
            : `${targets[i]}: ✗ ${describeError(r.reason)}`,
        )
        .join("\n\n"),
    );
  };

  return (
    <>
      <p className="hint">
        Tầng ROOT cho {scopeLabel} (Android, cần Magisk <code>su</code>). Rủi ro cao, chỉ dùng
        trên thiết bị hợp pháp của bạn: đổi định danh chống trùng, khôi phục gốc, lệnh root.
      </p>

      <div className="row">
        <button type="button" className="ghost" disabled={busy !== null} onClick={() => void probe()}>
          {busy === "probe" ? "Đang kiểm tra…" : "Kiểm tra trạng thái root"}
        </button>
        {rootStatus === "done" && (
          <span className="hint">
            {rootedCount}/{targets.length} máy đã root
          </span>
        )}
      </div>

      <fieldset className="group-tools-fieldset">
        <legend>Máy mới — đổi định danh (mỗi máy một giá trị ngẫu nhiên)</legend>
        <p className="hint">
          Đổi định danh mà ứng dụng đọc được (Android ID / serial / MAC Wi-Fi), không đổi IMEI
          baseband. Android ID không cần root; serial &amp; MAC cần root.
        </p>
        <label className="check">
          <input
            type="checkbox"
            checked={changeAndroidId}
            onChange={(e) => setChangeAndroidId(e.target.checked)}
          />
          Android ID
        </label>
        <label className="check">
          <input type="checkbox" checked={changeSerial} onChange={(e) => setChangeSerial(e.target.checked)} />
          Serial (cần root)
        </label>
        <label className="check">
          <input type="checkbox" checked={changeMac} onChange={(e) => setChangeMac(e.target.checked)} />
          MAC Wi-Fi (cần root)
        </label>
        <div className="nurture-float-actions">
          <button type="button" className="primary" disabled={busy !== null} onClick={() => void applyIdentity()}>
            {busy === "identity" ? "Đang đổi…" : "Tạo ngẫu nhiên & áp mỗi máy"}
          </button>
        </div>
      </fieldset>

      <fieldset className="group-tools-fieldset">
        <legend>Lệnh root (nâng cao)</legend>
        <div className="row">
          <input
            type="text"
            style={{ flex: 1 }}
            placeholder="vd: getprop ro.serialno"
            value={shellCmd}
            onChange={(e) => setShellCmd(e.target.value)}
          />
          <button type="button" className="ghost" disabled={busy !== null} onClick={() => void runShell()}>
            {busy === "shell" ? "Đang chạy…" : "Chạy (su)"}
          </button>
        </div>
      </fieldset>

      {logText && <pre className="group-tools-log">{logText}</pre>}

      <fieldset className="group-tools-fieldset danger-zone">
        <legend>Vùng nguy hiểm</legend>
        <div className="nurture-float-actions">
          <button type="button" className="danger" disabled={busy !== null} onClick={() => void runReset()}>
            {busy === "reset" ? "Đang gửi…" : `Khôi phục gốc ${scopeLabel}`}
          </button>
        </div>
      </fieldset>
    </>
  );
}
