import { Upload, X } from "lucide-react";
import { useState } from "react";
import { flowImportLegacy } from "../../api";
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
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section role="dialog" aria-modal="true" aria-label="Import legacy flow" className="flow-dialog">
      <header>
        <strong>Import legacy flow</strong>
        <button type="button" aria-label="Close import dialog" title="Close" onClick={onClose}>
          <X aria-hidden="true" size={16} />
        </button>
      </header>
      <label className="flow-field">
        <span>Legacy script JSON</span>
        <textarea
          value={raw}
          rows={14}
          spellCheck={false}
          onChange={(event) => setRaw(event.currentTarget.value)}
        />
      </label>
      {error && <p role="alert">{error}</p>}
      {result && result.diagnostics.length > 0 && (
        <section aria-label="Import diagnostics">
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
        <button type="button" onClick={onClose}>Cancel</button>
        <button type="button" disabled={busy || raw.trim() === ""} onClick={() => void submit()}>
          <Upload aria-hidden="true" size={15} />
          {busy ? "Importing..." : "Import"}
        </button>
      </footer>
    </section>
  );
}
