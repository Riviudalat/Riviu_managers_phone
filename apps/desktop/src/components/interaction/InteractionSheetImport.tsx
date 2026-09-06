import { Download } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { interactionImportSheet, type InteractionSheetImport as ImportResult } from "../../api";
import { describeError } from "../../describeError";
import { linkErrorVi } from "../../interactionErrors";

export function InteractionSheetImport({ onApply }: { onApply: (urls: string[]) => void }) {
  const [url, setUrl] = useState("");
  const [column, setColumn] = useState("D");
  const [result, setResult] = useState<ImportResult | null>(null);
  const [selected, setSelected] = useState<number[]>([]);
  const [filter, setFilter] = useState("all");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ticket = useRef(0);
  useEffect(() => () => { ticket.current = -1; }, []);
  const invalidate = () => { ticket.current++; setResult(null); setSelected([]); setError(null); setBusy(false); };
  async function load() {
    const current = ++ticket.current; setBusy(true); setError(null); setResult(null); setSelected([]);
    try { const next = await interactionImportSheet(url, column); if (ticket.current === current) setResult(next); }
    catch (e) { if (ticket.current === current) setError(describeError(e)); }
    finally { if (ticket.current === current) setBusy(false); }
  }
  const rows = result?.rows ?? [];
  const eligible = rows.filter((r) => r.line.target && !r.duplicateOf);
  const shown = rows.filter((r) => filter === "all" || (filter === "valid" ? r.line.target && !r.duplicateOf : r.duplicateOf || !r.line.target));
  return <details className="interaction-sheet-import">
    <summary>Nhập từ Google Sheet</summary>
    <div className="interaction-sheet-source">
      <label>Link Sheet<input type="url" value={url} onChange={(e) => { invalidate(); setUrl(e.target.value); }} /></label>
      <label>Cột link<input value={column} maxLength={3} onChange={(e) => { invalidate(); setColumn(e.target.value.toUpperCase()); }} /></label>
      <button type="button" className="btn" disabled={busy || !url.trim() || !column} onClick={() => void load()}><Download size={14} aria-hidden="true" />{busy ? "Đang đọc…" : "Đọc Sheet"}</button>
    </div>
    {error && <p role="alert">{error}</p>}
    {result && <>
      <div className="interaction-sheet-toolbar">
        <output>{eligible.length} link hợp lệ · {rows.length - eligible.length} dòng cần kiểm tra · {selected.length} đã chọn</output>
        <select aria-label="Lọc dòng Sheet" value={filter} onChange={(e) => setFilter(e.target.value)}><option value="all">Tất cả</option><option value="valid">Hợp lệ</option><option value="errors">Trùng hoặc lỗi</option></select>
        <button type="button" onClick={() => setSelected(eligible.map((r) => r.row))}>Chọn link hợp lệ</button>
        <button type="button" disabled={!selected.length} onClick={() => setSelected([])}>Bỏ chọn</button>
      </div>
      {rows.length === 0 ? <p>Chưa có link trong cột đã chọn.</p> : <div className="interaction-sheet-rows"><table><thead><tr><th>Chọn</th><th>Dòng</th><th>Bài viết</th><th>Trạng thái</th></tr></thead><tbody>
        {shown.map((r) => <tr key={r.row}><td><input type="checkbox" aria-label={`Chọn dòng ${r.row}`} disabled={!r.line.target || Boolean(r.duplicateOf)} checked={selected.includes(r.row)} onChange={(e) => setSelected((prev) => e.target.checked ? [...prev, r.row] : prev.filter((n) => n !== r.row))} /></td><td>{r.row}</td><td>{r.line.target?.normalizedUrl ?? r.line.original}</td><td>{r.duplicateOf ? `Trùng dòng ${r.duplicateOf}` : r.line.target ? "Hợp lệ" : linkErrorVi(r.line.error)}</td></tr>)}
      </tbody></table></div>}
      <button type="button" className="btn" disabled={!selected.length} onClick={() => { onApply(eligible.filter((r) => selected.includes(r.row)).map((r) => r.line.target!.normalizedUrl)); setSelected([]); }}>Thêm {selected.length} bài đã chọn</button>
      <details><summary>Nguồn dữ liệu</summary><code>{result.sourceUrl}</code><code>{result.digest}</code></details>
    </>}
  </details>;
}
