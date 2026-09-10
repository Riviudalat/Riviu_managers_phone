import { useEffect, useRef, useState } from "react";
import { CheckCircle2, CircleAlert, Link2, LoaderCircle } from "lucide-react";
import { publishSheetPrepare, publishSheetGetConfig } from "../../api";
import { describeError } from "../../describeError";
import type { PublishSheetCheckResult } from "../../types";

export function PublishSheetConnection({ onReadyChange }: { onReadyChange?: (ready: boolean) => void }) {
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<PublishSheetCheckResult | null>(null);
  const [error, setError] = useState("");
  const [linkEntered, setLinkEntered] = useState(false);
  const [configLoaded, setConfigLoaded] = useState(false);
  const ticket = useRef(0);
  const edited = useRef(false);
  const inFlight = useRef(false);
  const ready = configLoaded && !error && (!linkEntered || (!busy && result?.connectionVerified === true));
  useEffect(() => { onReadyChange?.(ready); }, [onReadyChange, ready]);
  useEffect(() => {
    let mounted = true;
    void publishSheetGetConfig().then(config => {
      if (mounted && !edited.current) {
        const saved = config.sheetUrl ?? "";
        setUrl(saved);
        if (saved.trim()) setLinkEntered(true);
        if (saved.trim() && config.hasToken) {
          inFlight.current = true; setBusy(true);
          const current = ++ticket.current;
          void publishSheetPrepare(saved).then(next => { if (mounted && current === ticket.current) setResult(next); })
            .catch(e => { if (mounted && current === ticket.current) setError(describeError(e)); })
            .finally(() => { inFlight.current = false; if (mounted && current === ticket.current) setBusy(false); });
        }
      }
      if (mounted) setConfigLoaded(true);
    }).catch(e => { if (mounted && !edited.current) setError(describeError(e)); });
    return () => { mounted = false; ticket.current += 1; };
  }, []);
  const check = async () => {
    const value = url.trim();
    if (!value || inFlight.current) return;
    inFlight.current = true;
    const current = ++ticket.current;
    edited.current = true;
    setBusy(true); setError(""); setResult(null);
    try {
      const next = await publishSheetPrepare(value);
      if (current === ticket.current) { setResult(next); setConfigLoaded(true); }
    } catch (e) { if (current === ticket.current) setError(describeError(e)); }
    finally { inFlight.current = false; if (current === ticket.current) setBusy(false); }
  };
  const verified = result?.connectionVerified === true;
  return <div className="publish-sheet-connection">
    <label htmlFor="publish-sheet-link"><Link2 size={15} aria-hidden="true"/>Link Google Sheet</label>
    <div className="publish-sheet-link-controls"><input id="publish-sheet-link" type="url" aria-label="Link Google Sheet"
      placeholder="https://docs.google.com/spreadsheets/d/..." value={url} disabled={busy}
      onChange={e => { if (inFlight.current) return; edited.current = true; setLinkEntered(true); ticket.current += 1; setUrl(e.target.value); setError(""); setResult(null); }}
      onKeyDown={e => { if (e.key === "Enter") { e.preventDefault(); void check(); } }}/>
      <button type="button" onClick={() => void check()} disabled={!url.trim() || busy}>{busy ? <LoaderCircle className="publish-check-spinner" size={15} aria-hidden="true"/> : <CheckCircle2 size={15} aria-hidden="true"/>}{busy ? "Đang chuẩn bị…" : "Kết nối Sheet"}</button>
    </div>
    {(result || error) && <p role={error ? "alert" : "status"} className={`publish-sheet-result ${verified ? "is-verified" : "needs-attention"}`}>
      {verified ? <CheckCircle2 size={14} aria-hidden="true"/> : <CircleAlert size={14} aria-hidden="true"/>}{error || result?.message}
    </p>}
  </div>;
}
