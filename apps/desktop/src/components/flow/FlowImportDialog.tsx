import { Upload, X } from "lucide-react";
import { useState } from "react";
import { flowImportLegacy } from "../../api";
import { describeError } from "../../describeError";
import type { FlowDocumentV2, LegacyImportResult } from "../../types";

export interface FlowImportDialogProps {
  onImport: (document: FlowDocumentV2) => void;
  onClose: () => void;
  importLegacy?: (scriptJson: string) => Promise<LegacyImportResult>;
}

export function FlowImportDialog({
  onImport,
  onClose,
  importLegacy = flowImportLegacy,
}: FlowImportDialogProps) {
  const [raw, setRaw] = useState("");
  const [result, setResult] = useState<LegacyImportResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const imported = await importLegacy(raw);
      setResult(imported);
      if (imported.document !== null && imported.diagnostics.length === 0) {
        onImport(imported.document);
      }
    } catch (reason) {
      // `flow_import_legacy` rejects with a `CommandError` object (`flow_commands.rs:111`), so
      // `String(reason)` printed `[object Object]` over the reason the JSON was refused.
      setError(describeError(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section role="dialog" aria-modal="true" aria-label="Nhập Flow cũ" className="flow-dialog">
      <header>
        <strong>Nhập Flow cũ</strong>
        <button type="button" aria-label="Đóng hộp thoại nhập" title="Đóng" onClick={onClose}>
          <X aria-hidden="true" size={16} />
        </button>
      </header>
      <label className="flow-field">
        <span>JSON script cũ</span>
        <textarea
          value={raw}
          rows={14}
          spellCheck={false}
          onChange={(event) => setRaw(event.currentTarget.value)}
        />
      </label>
      {error && <p role="alert">{error}</p>}
      {result && result.diagnostics.length > 0 && (
        <section aria-label="Chẩn đoán nhập">
          <strong>{result.diagnostics.length} diagnostics</strong>
          <ul>
            {result.diagnostics.map((diagnostic, index) => (
              <li key={`${diagnostic.stepIndex}-${diagnostic.code}-${index}`}>
                <strong>{diagnostic.code}</strong>
                <span>
                  Step {diagnostic.stepIndex}: {diagnostic.message}
                  {diagnostic.field ? ` (${diagnostic.field})` : ""}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
      <footer>
        <button type="button" onClick={onClose}>Hủy</button>
        <button type="button" disabled={busy || raw.trim() === ""} onClick={() => void submit()}>
          <Upload aria-hidden="true" size={15} />
          {busy ? "Importing..." : "Import"}
        </button>
      </footer>
    </section>
  );
}
