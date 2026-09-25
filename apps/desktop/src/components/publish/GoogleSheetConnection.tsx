import { useEffect, useRef, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { googleSheetsCancel, googleSheetsConfigure, googleSheetsConnect, googleSheetsLogin,
  googleSheetsStatus, publishSheetCheck, publishSheetGetConfig } from "../../api";
import { describeError } from "../../describeError";
import { parseGoogleSheetUrl, type GoogleSheetTarget } from "./googleSheetUrl";
import type { GoogleSheetsConfiguration, GoogleSheetsStatus, PublishSheetCheckResult } from "../../types";
import { GoogleAppSetup } from "./GoogleAppSetup";
import { requestConfirm } from "../../confirmStore";
import { getSheetVerificationSession, invalidateSheetVerificationSession, isSheetVerificationInvalidated, setSheetVerificationSession, type CheckedSheet } from "./sheetVerificationSession";

type Target = GoogleSheetTarget;
type Action = "loading" | "login" | "check" | "picking" | "cancel" | "configure" | null;
type Props = { onReadyChange?: (ready: boolean) => void };
const sameTarget = (a: Target | null, b: Target | null) => !!a && !!b && a.spreadsheetId === b.spreadsheetId && a.sheetId === b.sheetId;
const accountKey = (s: GoogleSheetsStatus) => JSON.stringify([s.clientId, s.accountId, s.connected, s.active, s.writerId, s.hasSheetsScope,
  s.active ? parseGoogleSheetUrl(s.sheetUrl || "")?.url ?? s.sheetUrl : null]);

function boundedRead<T>(promise: Promise<T>, milliseconds = 15_000): Promise<T> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(Error("Đã hết thời gian đọc kết nối Google. Kết nối đã lưu được giữ nguyên; kiểm tra lại khi dịch vụ phản hồi.")), milliseconds);
    promise.then(resolve, reject).finally(() => clearTimeout(timeout));
  });
}
const readGoogleStatus = () => boundedRead(googleSheetsStatus());

export function GoogleSheetConnection({ onReadyChange }: Props) {
  const [url, setUrl] = useState("");
  const [status, setStatus] = useState<GoogleSheetsStatus | null>(null);
  const [action, setAction] = useState<Action>("loading");
  const [error, setError] = useState<string | null>(null);
  const [setupOpen, setSetupOpen] = useState(false);
  const [result, setResult] = useState<CheckedSheet | null>(null);
  const mounted = useRef(true), edited = useRef(false);
  const generation = useRef(0), flight = useRef<number | null>(null);
  const browserInvoked = useRef(false);
  const urlRevision = useRef(0);
  const urlRef = useRef(""), statusRef = useRef<GoogleSheetsStatus | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null), wake = useRef<(() => void) | null>(null);
  const refreshFocus = useRef<() => void>(() => {});
  const valid = (ticket: number) => mounted.current && generation.current === ticket;
  const stopWait = () => { if (timer.current) clearTimeout(timer.current); timer.current = null; wake.current?.(); wake.current = null; };
  const pause = () => new Promise<void>(resolve => { wake.current = resolve; timer.current = setTimeout(() => { timer.current = null; wake.current = null; resolve(); }, 1500); });
  const applyStatus = (next: GoogleSheetsStatus) => {
    const key = accountKey(next);
    const verifiedSession = getSheetVerificationSession();
    if ((statusRef.current && accountKey(statusRef.current) !== key)
      || (verifiedSession && (verifiedSession.account !== key || !next.connected || !next.active || next.phase !== "idle" || !!next.error))) {
      setResult(null);
      invalidateSheetVerificationSession();
    }
    statusRef.current = next; setStatus(next);
  };
  const finish = (ticket: number) => { if (flight.current === ticket) { flight.current = null; if (mounted.current) setAction(null); } };
  const verifiedResult = (value: PublishSheetCheckResult, target: Target, next: GoogleSheetsStatus, ticket: number, revision: number) => {
    if (!valid(ticket) || urlRevision.current !== revision) return;
    if (value.spreadsheetId !== target.spreadsheetId || value.sheetGid !== target.sheetId) throw Error("Kết quả không khớp bảng và tab trong link đã nhập.");
    const checked = { account: accountKey(next), target, ready: value.connectionVerified && value.reportingReady === true, message: value.message };
    if (checked.ready) setSheetVerificationSession(checked);
    else invalidateSheetVerificationSession();
    setResult(checked);
  };
  const awaitBrowser = async (first: GoogleSheetsStatus, ticket: number): Promise<GoogleSheetsStatus | null> => {
    let next = first; const deadline = Date.now() + 300_000;
    while (valid(ticket) && next.phase !== "idle") {
      applyStatus(next);
      if (Date.now() >= deadline) { await googleSheetsCancel(); throw Error("Đã hết thời gian chờ Google. Vui lòng thử lại."); }
      await pause(); if (!valid(ticket)) return null;
      next = await readGoogleStatus();
    }
    if (!valid(ticket)) return null;
    applyStatus(next); if (next.error) throw Error(next.error); return next;
  };
  useEffect(() => {
    mounted.current = true; const ticket = ++generation.current; flight.current = ticket;
    void (async () => {
      const [google, config] = await Promise.allSettled([readGoogleStatus(), boundedRead(publishSheetGetConfig())]);
      if (!valid(ticket)) return;
      if (google.status === "rejected") throw google.reason;
      const next = google.value; applyStatus(next);
      const initial = next.sheetUrl || (config.status === "fulfilled" ? config.value.sheetUrl : "") || "";
      if (!edited.current) { urlRef.current = initial; setUrl(initial); }
      if (next.error) setError(next.error);
      const target = parseGoogleSheetUrl(initial);
      const verifiedSession = getSheetVerificationSession();
      if (!edited.current && next.active && next.connected && next.phase === "idle" && !next.error
        && verifiedSession?.account === accountKey(next) && sameTarget(target, verifiedSession.target)) {
        setResult({ ...verifiedSession, message: "Đã kiểm tra trong phiên này; sẽ đối chiếu lại trước khi đăng." });
      }
      // Reuse only local proof; publish preflight still checks the remote target.
      // Loading the page must not acquire a shared writer lock.
    })().catch(e => { if (valid(ticket)) { invalidateSheetVerificationSession(); setError(describeError(e)); } }).finally(() => finish(ticket));
    const onFocus = () => refreshFocus.current(); window.addEventListener("focus", onFocus);
    return () => { mounted.current = false; generation.current += 1; flight.current = null; stopWait(); window.removeEventListener("focus", onFocus); };
  }, []);
  useEffect(() => {
    refreshFocus.current = () => {
      if (flight.current !== null) return;
      const ticket = generation.current;
      void readGoogleStatus().then(next => {
        if (valid(ticket) && flight.current === null) { applyStatus(next); if (next.error) setError(next.error); }
      }).catch(e => { if (valid(ticket) && flight.current === null) { invalidateSheetVerificationSession(); setResult(null); setError(describeError(e)); } });
    };
  });
  const connectTarget = async (target: Target, ticket: number, revision: number) => {
    if (!valid(ticket) || urlRevision.current !== revision) return;
    setAction("check");
    let checked: PublishSheetCheckResult;
    try { checked = await googleSheetsConnect(target.spreadsheetId, target.sheetId, true, false); }
    catch (cause) {
      if (!valid(ticket) || urlRevision.current !== revision) return;
      const code = cause && typeof cause === "object" && "code" in cause ? cause.code : null;
      if (code !== "SharedSheetUpgradeRequired") throw cause;
      const confirmed = await requestConfirm({
        title: "Nâng cấp tab để nhiều máy cùng ghi?",
        message: "Hãy dừng ghi Sheet trên tất cả bản app cũ trước khi nâng cấp tab này. Dữ liệu và liên kết đang có được giữ nguyên; sau nâng cấp, chỉ các bản app hỗ trợ ghi đa máy được tiếp tục. Không cần tạo tab mới.",
        confirmLabel: "Đã dừng bản cũ · Nâng cấp", cancelLabel: "Chưa nâng cấp", danger: true,
      });
      if (!valid(ticket) || urlRevision.current !== revision) return;
      if (!confirmed) {
        const previous = await googleSheetsCancel();
        if (!valid(ticket) || urlRevision.current !== revision) return;
        applyStatus(previous);
        throw Error("Chưa nâng cấp tab. Dừng ghi từ bản app cũ rồi kiểm tra lại kết nối; dữ liệu giữ nguyên.");
      }
      checked = await googleSheetsConnect(target.spreadsheetId, target.sheetId, true, true);
    }
    if (!valid(ticket) || urlRevision.current !== revision) return;
    const connected = await readGoogleStatus();
    if (!valid(ticket) || urlRevision.current !== revision) return;
    applyStatus(connected);
    if (!connected.connected || !connected.active || !sameTarget(target, parseGoogleSheetUrl(connected.sheetUrl || ""))) throw Error("Google chưa xác nhận đúng bảng và tab. Kiểm tra lại quyền sửa của tài khoản.");
    verifiedResult(checked, target, connected, ticket, revision);
  };
  const login = async () => {
    if (flight.current !== null) return;
    invalidateSheetVerificationSession();
    const ticket = ++generation.current, revision = urlRevision.current; flight.current = ticket; browserInvoked.current = false; setAction("login"); setError(null); setResult(null);
    try {
      const current = await readGoogleStatus(); if (!valid(ticket)) return; applyStatus(current);
      if (!current.configured) {
        setSetupOpen(true);
        throw Error("Chưa có cấu hình Google trên máy này. Mở Thiết lập Google bên dưới để bổ sung.");
      }
      browserInvoked.current = true;
      const loggedIn = await awaitBrowser(current.phase !== "idle" ? current : await googleSheetsLogin(), ticket);
      if (!loggedIn || !valid(ticket) || urlRevision.current !== revision) return;
      const target = parseGoogleSheetUrl(urlRef.current);
      if (target) await connectTarget(target, ticket, revision);
    } catch (e) { if (valid(ticket)) setError(describeError(e)); }
    finally { finish(ticket); }
  };
  const cancel = async () => {
    if (!browserInvoked.current || (action !== "login" && action !== "picking")) return;
    invalidateSheetVerificationSession();
    browserInvoked.current = false;
    const ticket = ++generation.current; flight.current = ticket; stopWait(); setAction("cancel"); setResult(null);
    try { const next = await googleSheetsCancel(); if (valid(ticket)) { applyStatus(next); setError(null); } }
    catch (e) { if (valid(ticket)) setError(describeError(e)); }
    finally { finish(ticket); }
  };
  const configure = async (config: GoogleSheetsConfiguration) => {
    if (flight.current !== null) return false;
    invalidateSheetVerificationSession();
    const ticket = ++generation.current; flight.current = ticket;
    setAction("configure"); setError(null); setResult(null);
    try {
      const next = await googleSheetsConfigure(config);
      if (!valid(ticket)) return false;
      applyStatus(next);
      if (!next.configured) throw Error("Cấu hình Google chưa đầy đủ; kiểm tra lại thông tin đã nhập.");
      setSetupOpen(false); return true;
    } catch (e) { if (valid(ticket)) setError(describeError(e)); return false; }
    finally { finish(ticket); }
  };
  const check = async () => {
    if (flight.current !== null) return;
    invalidateSheetVerificationSession();
    setResult(null); setError(null); const target = parseGoogleSheetUrl(urlRef.current);
    if (!target) { setError("Nhập link Google Sheet hợp lệ; tab lấy từ gid trong link."); return; }
    const ticket = ++generation.current, revision = urlRevision.current; flight.current = ticket; setAction("check");
    try {
      const current = await readGoogleStatus(); if (!valid(ticket) || urlRevision.current !== revision) return; applyStatus(current);
      if (!current.connected) throw Error("Đăng nhập Google trước khi kiểm tra kết nối.");
      if (current.phase !== "idle") throw Error("Hoàn tất hoặc hủy cửa sổ Google đang mở rồi kiểm tra lại.");
      if (current.active && sameTarget(target, parseGoogleSheetUrl(current.sheetUrl || ""))) {
        try {
          const checked = await boundedRead(publishSheetCheck(target.url), 120_000);
          if (!valid(ticket) || urlRevision.current !== revision) return;
          verifiedResult(checked, target, current, ticket, revision); return;
        } catch (cause) {
          if (!cause || typeof cause !== "object" || !("code" in cause) || cause.code !== "SharedSheetUpgradeRequired") throw cause;
        }
      }
      await connectTarget(target, ticket, revision);
    } catch (e) { if (valid(ticket)) setError(describeError(e)); }
    finally { finish(ticket); }
  };
  const target = parseGoogleSheetUrl(url);
  const verified = action === null && !error && status?.connected === true && status.active && status.phase === "idle" && result?.account === accountKey(status)
    && result.ready && sameTarget(target, result.target);
  const savedBinding = action === null && !error && status?.connected === true && status.active && status.phase === "idle"
    && status.hasSheetsScope === true && !!status.writerId && sameTarget(target, parseGoogleSheetUrl(status.sheetUrl || ""));
  const canPreflight = verified || (result === null && savedBinding && !isSheetVerificationInvalidated());
  useEffect(() => { onReadyChange?.(canPreflight === true); }, [canPreflight, onReadyChange]);
  const canCancel = action === "login" || action === "picking";
  const message = error || (action === "picking" ? "Chọn đúng bảng trong cửa sổ Google để cấp quyền." : action === "login" ? "Hoàn tất đăng nhập trong trình duyệt Google."
    : action === "loading" ? "Đang đọc kết nối Google…" : action ? "Đang kiểm tra kết nối…" : verified ? "Kết nối đã xác minh."
      : result?.message || (savedBinding && !isSheetVerificationInvalidated() ? "Đã liên kết Google Sheet; sẽ kiểm tra quyền ghi trước khi đăng."
        : status && !status.configured ? "Bản app chưa có cấu hình Google. Mở Thiết lập Google để bổ sung."
        : status?.connected ? "Chưa xác minh kết nối bảng." : "Chưa đăng nhập Google."));
  return <div className="publish-sheet-connection google-sheet-connection" data-google-sheet-focus tabIndex={-1}>
    <label htmlFor="publish-sheet-link">Link Google Sheet</label>
    <div className="google-sheet-compact-controls">
      <input id="publish-sheet-link" type="url" aria-label="Link Google Sheet" value={url} placeholder="https://docs.google.com/spreadsheets/d/...#gid=0"
        onChange={event => { edited.current = true; invalidateSheetVerificationSession(); urlRevision.current += 1; urlRef.current = event.target.value; setUrl(event.target.value); setResult(null); setError(null); }}
        onKeyDown={event => { if (event.key === "Enter") { event.preventDefault(); void check(); } }} />
      <button type="button" disabled={!canCancel && action !== null} title={status?.email || "Đăng nhập Google"} onClick={() => void (canCancel ? cancel() : login())}>{canCancel ? "Hủy đăng nhập" : "Đăng nhập Google"}</button>
      <button type="button" disabled={action !== null || !url.trim()} onClick={() => void check()}>{(action === "check" || action === "picking") && <LoaderCircle className="publish-check-spinner" size={14} aria-hidden="true" />}Kiểm tra kết nối</button>
    </div>
    <p tabIndex={0} role={error ? "alert" : "status"} className={`publish-sheet-result ${verified ? "is-verified" : savedBinding && result === null && !isSheetVerificationInvalidated() ? "is-linked" : "needs-attention"}`}>{status?.email ? `${status.email} · ` : ""}{message}</p>
    {status && !status.configured && <GoogleAppSetup key={status.clientId} clientId={status.clientId}
      busy={action !== null || status.phase !== "idle"} open={setupOpen} onOpenChange={setSetupOpen} onSave={configure} />}
  </div>;
}
