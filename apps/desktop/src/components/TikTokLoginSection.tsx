import { useCallback, useEffect, useState } from "react";
import {
  getDeviceMeta,
  listAppsLibrary,
  tiktokCredentialClear,
  tiktokCredentialSet,
  tiktokCredentialsList,
  tiktokReinstallAndLogin,
  type TikTokReloginReport,
  type TikTokReloginStage,
  type TikTokReloginStageResult,
} from "../api";
import { requestConfirm } from "../confirmStore";
import { describeError } from "../describeError";
import type { TikTokCredentialSummary } from "../generated-ipc";

// The only build the reinstall command accepts; the backend re-checks signer and SHA-256.
const GLOBAL_PACKAGE = "com.zhiliaoapp.musically";
const GLOBAL_VERSION = "45.7.3";
const GLOBAL_VERSION_CODE = "2024507030";

const stageLabels: Record<TikTokReloginStage, string> = {
  preflight: "Kiểm tra trước",
  logout: "Đăng xuất",
  uninstall: "Gỡ TikTok",
  install: "Cài TikTok 45.7.3",
  firstRun: "Màn hình lần đầu",
  login: "Đăng nhập",
  securitySheet: "Bảng kiểm tra bảo mật",
  proveHandle: "Xác nhận nick",
};
const outcomeLabels: Record<TikTokReloginStageResult["outcome"], string> = {
  done: "Xong",
  skipped: "Bỏ qua",
  failed: "Dừng",
  uncertain: "Chưa chắc chắn — kiểm tra máy",
};
const statusLabels: Record<string, string> = {
  loggedIn: "Lần trước: đã đăng nhập",
  loginSubmitted: "Lần trước: đã gửi đăng nhập nhưng chưa có kết quả — kiểm tra máy rồi lưu lại mật khẩu",
  loginUncertain: "Lần trước: đăng nhập chưa rõ kết quả — kiểm tra máy rồi lưu lại mật khẩu",
  "loginStopped:wrongPassword": "Lần trước: TikTok báo sai mật khẩu — lưu lại mật khẩu đúng",
  "loginStopped:tooManyAttempts": "Lần trước: TikTok báo đăng nhập quá nhiều lần",
  "loginStopped:verification": "Lần trước: TikTok đòi mã xác minh",
  "loginStopped:captcha": "Lần trước: TikTok hiện captcha",
  handleMismatch: "Lần trước: máy đăng nhập vào nick khác",
};

const bare = (value: string) => value.trim().replace(/^@+/, "").toLowerCase();

/** Username/password for the phone's TikTok account, and the reinstall + login command. Android only. */
export function TikTokLoginSection({ udid, busy }: { udid: string; busy: boolean }) {
  const [assigned, setAssigned] = useState("");
  const [saved, setSaved] = useState<TikTokCredentialSummary | null>(null);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<TikTokReloginReport | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [meta, logins] = await Promise.all([getDeviceMeta(udid), tiktokCredentialsList()]);
      const handle = bare(meta.handle ?? "");
      setAssigned(handle);
      setSaved(logins.find((login) => login.handle === handle) ?? null);
      setUsername((current) => current || handle);
    } catch (cause) {
      setError(`Chưa đọc được đăng nhập TikTok: ${describeError(cause)}`);
    }
  }, [udid]);

  useEffect(() => {
    void load();
    // The typed password never outlives the panel.
    return () => setPassword("");
  }, [load]);

  const save = async () => {
    if (!username.trim() || !password) return;
    setWorking(true); setError(null);
    try {
      const summary = await tiktokCredentialSet(username, password);
      setPassword("");
      if (summary.handle === assigned) setSaved(summary);
    } catch (cause) {
      setError(`Chưa lưu đăng nhập: ${describeError(cause)}`);
    } finally { setWorking(false); }
  };

  const clear = async () => {
    const handle = bare(username) || assigned;
    if (!handle) return;
    if (!(await requestConfirm({ title: "Xóa đăng nhập đã lưu?", message: `Xóa username và mật khẩu của @${handle} khỏi kho mật khẩu của Windows.`, confirmLabel: "Xóa" }))) return;
    setWorking(true); setError(null);
    try {
      await tiktokCredentialClear(handle);
      if (handle === assigned) setSaved(null);
    } catch (cause) {
      setError(`Chưa xóa đăng nhập: ${describeError(cause)}`);
    } finally { setWorking(false); }
  };

  const reinstall = async () => {
    setError(null);
    let appId: string;
    let appName: string;
    try {
      const matches = (await listAppsLibrary()).filter((item) => item.platform === "android"
        && item.applicationId === GLOBAL_PACKAGE && item.versionName === GLOBAL_VERSION
        && item.versionCode === GLOBAL_VERSION_CODE);
      if (matches.length === 0) {
        setError(`Thư viện ứng dụng chưa có APK TikTok Global ${GLOBAL_VERSION}; thêm gói vào thư viện trước.`);
        return;
      }
      appId = matches[0].id;
      appName = matches[0].name;
    } catch (cause) {
      setError(`Chưa đọc được thư viện ứng dụng: ${describeError(cause)}`);
      return;
    }
    if (!(await requestConfirm({
      title: "Cài lại TikTok 45.7.3 + đăng nhập?",
      message: `Riviu sẽ đăng xuất, gỡ TikTok (mất dữ liệu TikTok trên máy), cài ${appName}, qua các màn hình đầu và đăng nhập @${assigned} đúng một lần. Gặp OTP, captcha hay sai mật khẩu thì dừng để bạn xử lý trên máy.`,
      confirmLabel: "Cài lại và đăng nhập",
    }))) return;
    setWorking(true); setReport(null);
    try {
      setReport(await tiktokReinstallAndLogin(udid, appId));
    } catch (cause) {
      setError(`Chưa chạy được: ${describeError(cause)}`);
    } finally {
      setWorking(false);
      void load();
    }
  };

  const typedOther = Boolean(assigned) && Boolean(username.trim()) && bare(username) !== assigned;
  const canReinstall = !busy && !working && Boolean(assigned) && saved?.presence === "stored";

  return (
    <section className="device-detail-evidence" aria-label="Đăng nhập TikTok">
      <h3>Đăng nhập TikTok</h3>
      <p>{assigned ? `Nick gán cho máy: @${assigned}` : "Máy chưa gán nick TikTok."}</p>
      <p>
        {saved
          ? saved.presence === "stored"
            ? `Đã lưu đăng nhập (${saved.maskedIdentifier}).`
            : saved.presence === "missing" ? "Kho mật khẩu không còn mục này; nhập lại." : "Chưa đọc được kho mật khẩu."
          : "Chưa lưu mật khẩu cho nick này."}
      </p>
      {saved?.lastStatus && <p>{statusLabels[saved.lastStatus] ?? `Lần trước: ${saved.lastStatus}`}</p>}
      <div className="device-app-choice">
        <label>
          <span>Tên đăng nhập (username)</span>
          <input type="text" autoComplete="off" spellCheck={false} value={username} disabled={working} onChange={(event) => setUsername(event.target.value)} />
        </label>
        <label>
          <span>Mật khẩu</span>
          <input type="password" autoComplete="new-password" value={password} disabled={working} onChange={(event) => setPassword(event.target.value)} />
        </label>
        {typedOther && <p>Username này khác nick gán cho máy; lệnh cài lại chỉ dùng đăng nhập của @{assigned}.</p>}
        <div>
          <button type="button" disabled={working || !username.trim() || !password} onClick={() => void save()}>{working ? "Đang xử lý…" : "Lưu đăng nhập"}</button>
          <button type="button" disabled={working || !(bare(username) || assigned)} onClick={() => void clear()}>Xóa đăng nhập</button>
        </div>
      </div>
      <button type="button" disabled={!canReinstall} onClick={() => void reinstall()}>{working ? "Đang chạy…" : `Cài lại TikTok ${GLOBAL_VERSION} + đăng nhập`}</button>
      {error && <p role="alert">{error}</p>}
      {report?.udid === udid && <>
        <p role="status">{report.completed ? `Xong: máy đã đăng nhập @${report.handle}.` : "Chưa xong; xem bước dừng bên dưới."}</p>
        <dl className="device-detail-list">{report.stages.map((stage, index) => <div key={`${stage.stage}-${index}`}>
          <dt>{stageLabels[stage.stage] ?? stage.stage}</dt>
          <dd>{outcomeLabels[stage.outcome]}<small style={{ display: "block" }}>{stage.detail}</small></dd>
        </div>)}</dl>
      </>}
    </section>
  );
}
