import { useEffect, useRef, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { googleSheetsCancel, googleSheetsConnect, googleSheetsLogin, googleSheetsPickFile,
  googleSheetsStatus, publishSheetCheck, publishSheetGetConfig } from "../../api";
import { describeError } from "../../describeError";
import { parseGoogleSheetUrl, type GoogleSheetTarget } from "./googleSheetUrl";
import type { GoogleSheetsStatus, PublishSheetCheckResult } from "../../types";

type Target = GoogleSheetTarget;
type Action = "loading" | "login" | "check" | "picking" | "cancel" | null;
type Props = { onReadyChange?: (ready: boolean) => void };
const sameTarget = (a: Target | null, b: Target | null) => !!a && !!b && a.spreadsheetId === b.spreadsheetId && a.sheetId === b.sheetId;
const accountKey = (s: GoogleSheetsStatus) => JSON.stringify([s.accountId, s.connected, s.active, s.writerId,
  s.active ? parseGoogleSheetUrl(s.sheetUrl || "")?.url ?? s.sheetUrl : null]);

export function GoogleSheetConnection({ onReadyChange }: Props) {
  const [url, setUrl] = useState("");
  const [status, setStatus] = useState<GoogleSheetsStatus | null>(null);
  const [action, setAction] = useState<Action>("loading");
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{ value: PublishSheetCheckResult; account: string } | null>(null);
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
    if (statusRef.current && accountKey(statusRef.current) !== accountKey(next)) setResult(null);
    statusRef.current = next; setStatus(next);
  };
  const finish = (ticket: number) => { if (flight.current === ticket) { flight.current = null; if (mounted.current) setAction(null); } };
  const verifiedResult = (value: PublishSheetCheckResult, target: Target, next: GoogleSheetsStatus, ticket: number, revision: number) => {
    if (!valid(ticket) || urlRevision.current !== revision) return;
    if (value.spreadsheetId !== target.spreadsheetId || value.sheetGid !== target.sheetId) throw Error("Kết quả không khớp bảng và tab trong link đã nhập.");
    setResult({ value, account: accountKey(next) });
  };
  const awaitBrowser = async (first: GoogleSheetsStatus, ticket: number): Promise<GoogleSheetsStatus | null> => {
    let next = first; const deadline = Date.now() + 300_000;
    while (valid(ticket) && next.phase !== "idle") {
      applyStatus(next);
      if (Date.now() >= deadline) { await googleSheetsCancel(); throw Error("Đã hết thời gian chờ Google. Vui lòng thử lại."); }
      await pause(); if (!valid(ticket)) return null;
      next = await googleSheetsStatus();
    }
    if (!valid(ticket)) return null;
    applyStatus(next); if (next.error) throw Error(next.error); return next;
  };
  useEffect(() => {
    mounted.current = true; const ticket = ++generation.current; flight.current = ticket;
    void (async () => {
      const [google, config] = await Promise.allSettled([googleSheetsStatus(), publishSheetGetConfig()]);
      if (!valid(ticket)) return;
      if (google.status === "rejected") throw google.reason;
      const next = google.value; applyStatus(next);
      const initial = next.sheetUrl || (config.status === "fulfilled" ? config.value.sheetUrl : "") || "";
      if (!edited.current) { urlRef.current = initial; setUrl(initial); }
      if (next.error) setError(next.error);
      const target = parseGoogleSheetUrl(urlRef.current), revision = urlRevision.current;
      if (next.active && next.connected && next.phase === "idle" && target && sameTarget(target, parseGoogleSheetUrl(next.sheetUrl || ""))) {
        verifiedResult(await publishSheetCheck(target.url), target, next, ticket, revision);
      }
    })().catch(e => { if (valid(ticket)) setError(describeError(e)); }).finally(() => finish(ticket));
    const onFocus = () => refreshFocus.current(); window.addEventListener("focus", onFocus);
    return () => { mounted.current = false; generation.current += 1; flight.current = null; stopWait(); window.removeEventListener("focus", onFocus); };
  }, []);
  refreshFocus.current = () => {
    if (flight.current !== null) return;
    const ticket = generation.current;
    void googleSheetsStatus().then(next => {
      if (valid(ticket) && flight.current === null) { applyStatus(next); if (next.error) setError(next.error); }
    }).catch(e => { if (valid(ticket) && flight.current === null) { setResult(null); setError(describeError(e)); } });
  };
  const login = async () => {
    if (flight.current !== null) return;
    const ticket = ++generation.current; flight.current = ticket; browserInvoked.current = false; setAction("login"); setError(null); setResult(null);
    try {
      const current = await googleSheetsStatus(); if (!valid(ticket)) return; applyStatus(current);
      if (!current.configured) throw Error("Chưa có cấu hình Google trên máy này.");
      browserInvoked.current = true;
      await awaitBrowser(current.phase !== "idle" ? current : await googleSheetsLogin(), ticket);
    } catch (e) { if (valid(ticket)) setError(describeError(e)); }
    finally { finish(ticket); }
  };
  const cancel = async () => {
    if (!browserInvoked.current || (action !== "login" && action !== "picking")) return;
    browserInvoked.current = false;
    const ticket = ++generation.current; flight.current = ticket; stopWait(); setAction("cancel"); setResult(null);
    try { const next = await googleSheetsCancel(); if (valid(ticket)) { applyStatus(next); setError(null); } }
    catch (e) { if (valid(ticket)) setError(describeError(e)); }
    finally { finish(ticket); }
  };
  const check = async () => {
    if (flight.current !== null) return;
    setResult(null); setError(null); const target = parseGoogleSheetUrl(urlRef.current);
    if (!target) { setError("Nhập link Google Sheet hợp lệ; tab lấy từ gid trong link."); return; }
    const ticket = ++generation.current, revision = urlRevision.current; flight.current = ticket; setAction("check");
    try {
      let current = await googleSheetsStatus(); if (!valid(ticket) || urlRevision.current !== revision) return; applyStatus(current);
      if (!current.connected) throw Error("Đăng nhập Google trước khi kiểm tra kết nối.");
      if (current.error) throw Error(current.error);
      if (current.phase !== "idle") throw Error("Hoàn tất hoặc hủy cửa sổ Google đang mở rồi kiểm tra lại.");
      if (current.active && sameTarget(target, parseGoogleSheetUrl(current.sheetUrl || ""))) {
        verifiedResult(await publishSheetCheck(target.url), target, current, ticket, revision); return;
      }
      if (current.selectedFileId !== target.spreadsheetId) {
        if (!current.pickerConfigured) throw Error("Chưa có cấu hình Google Picker trên máy này.");
        setAction("picking"); browserInvoked.current = true; const selected = await awaitBrowser(await googleSheetsPickFile(), ticket);
        if (!selected || urlRevision.current !== revision) return; current = selected;
        if (current.selectedFileId !== target.spreadsheetId) throw Error("Bảng đã chọn không khớp link. Chọn đúng bảng trong cửa sổ Google rồi kiểm tra lại.");
      }
      if (!valid(ticket) || urlRevision.current !== revision) return; setAction("check");
      const checked = await googleSheetsConnect(target.spreadsheetId, target.sheetId, true);
      if (!valid(ticket) || urlRevision.current !== revision) return;
      const connected = await googleSheetsStatus(); if (!valid(ticket)) return; applyStatus(connected);
      if (!connected.connected || !connected.active || !sameTarget(target, parseGoogleSheetUrl(connected.sheetUrl || ""))) throw Error("Kết nối Google chưa xác nhận đúng bảng và tab. Kiểm tra lại.");
      verifiedResult(checked, target, connected, ticket, revision);
    } catch (e) { if (valid(ticket)) setError(describeError(e)); }
    finally { finish(ticket); }
  };
  const target = parseGoogleSheetUrl(url);
  const ready = action === null && !error && status?.connected === true && status.active && result?.account === accountKey(status)
    && result.value.connectionVerified && result.value.reportingReady === true && target?.spreadsheetId === result.value.spreadsheetId && target.sheetId === result.value.sheetGid;
  useEffect(() => { onReadyChange?.(ready === true); }, [ready, onReadyChange]);
  const canCancel = action === "login" || action === "picking";
  const message = error || (action === "picking" ? "Chọn đúng bảng trong cửa sổ Google để cấp quyền." : action === "login" ? "Hoàn tất đăng nhập trong trình duyệt Google."
    : action === "loading" ? "Đang đọc kết nối Google…" : action ? "Đang kiểm tra kết nối…" : ready ? "Kết nối đã xác minh."
      : result?.value.message || (status?.connected ? "Chưa xác minh kết nối bảng." : "Chưa đăng nhập Google."));
  return <div className="publish-sheet-connection google-sheet-connection" data-google-sheet-focus tabIndex={-1}>
    <label htmlFor="publish-sheet-link">Link Google Sheet</label>
    <div className="google-sheet-compact-controls">
      <input id="publish-sheet-link" type="url" aria-label="Link Google Sheet" value={url} placeholder="https://docs.google.com/spreadsheets/d/...#gid=0"
        onChange={event => { edited.current = true; urlRevision.current += 1; urlRef.current = event.target.value; setUrl(event.target.value); setResult(null); setError(null); }}
        onKeyDown={event => { if (event.key === "Enter") { event.preventDefault(); void check(); } }} />
      <button type="button" disabled={!canCancel && action !== null} title={status?.email || "Đăng nhập Google"} onClick={() => void (canCancel ? cancel() : login())}>{canCancel ? "Hủy đăng nhập" : "Đăng nhập Google"}</button>
      <button type="button" disabled={action !== null || !url.trim()} onClick={() => void check()}>{(action === "check" || action === "picking") && <LoaderCircle className="publish-check-spinner" size={14} aria-hidden="true" />}Kiểm tra kết nối</button>
    </div>
    <p role={error ? "alert" : "status"} className={`publish-sheet-result ${ready ? "is-verified" : "needs-attention"}`}>{status?.email ? `${status.email} · ` : ""}{message}</p>
  </div>;
}
