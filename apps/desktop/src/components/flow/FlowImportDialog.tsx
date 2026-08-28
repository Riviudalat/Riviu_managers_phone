import { Upload, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
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
  // Both close controls stay enabled while the conversion is running, and the textarea stays
  // editable. Without a guard, clicking Import and then Hủy still replaced the open document when
  // the backend answered -- an explicitly cancelled import applying itself.
  const live = useRef(true);
  // Set on mount as well as cleared on unmount. Only clearing it is wrong under StrictMode, which
  // mounts, unmounts and remounts every effect: the cleanup ran, nothing set the flag back, and the
  // guard then rejected every result for the rest of the component's life. The e2e import stopped
  // applying entirely; jsdom tests do not wrap in StrictMode, so they never saw it.
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);

  const close = () => {
    live.current = false;
    onClose();
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const imported = await importLegacy(raw);
      if (!live.current) return;
      setResult(imported);
      if (imported.document !== null && imported.diagnostics.length === 0) {
        onImport(imported.document);
      }
    } catch (reason) {
      // `flow_import_legacy` rejects with a `CommandError` object (`flow_commands.rs:111`), so
      // `String(reason)` printed `[object Object]` over the reason the JSON was refused.
      if (live.current) setError(describeError(reason));
    } finally {
      if (live.current) setBusy(false);
    }
  };

  return (
    <section role="dialog" aria-modal="true" aria-label="Nhập Flow cũ" className="flow-dialog">
      <header>
        <strong>Nhập Flow cũ</strong>
        <button type="button" aria-label="Đóng hộp thoại nhập" title="Đóng" onClick={close}>
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
                  {/* `step_index` comes from `enumerate()`, so it is zero-based. Printing it raw
                      sent the operator to the step before the broken one. */}
                  Step {diagnostic.stepIndex + 1}: {diagnostic.message}
                  {diagnostic.field ? ` (${diagnostic.field})` : ""}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
      <footer>
        <button type="button" onClick={close}>Hủy</button>
        <button type="button" disabled={busy || raw.trim() === ""} onClick={() => void submit()}>
          <Upload aria-hidden="true" size={15} />
          {busy ? "Importing..." : "Import"}
        </button>
      </footer>
    </section>
  );
}
