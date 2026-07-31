import { Check, Download, X } from "lucide-react";
import { useEffect, useState } from "react";
import { flowExport, flowValidate } from "../../api";
import { isFlowDocumentV2 } from "../../flow/validation";
import type { CompiledRevision, FlowDocumentV2 } from "../../types";

const MAX_FLOW_JSON_BYTES = 1_048_576;

function assertFlowDocumentShape(value: unknown): asserts value is FlowDocumentV2 {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("FlowJsonObjectRequired");
  }
  if (!isFlowDocumentV2(value)) throw new Error("FlowJsonShapeInvalid");
}

async function importFlowJson(
  raw: string,
  validate: (document: FlowDocumentV2) => Promise<CompiledRevision> = flowValidate,
): Promise<FlowDocumentV2> {
  if (new TextEncoder().encode(raw).byteLength > MAX_FLOW_JSON_BYTES) {
    throw new Error("FlowImportTooLarge");
  }
  const document: unknown = JSON.parse(raw);
  assertFlowDocumentShape(document);
  await validate(document);
  return structuredClone(document);
}

export interface FlowJsonDialogProps {
  document: FlowDocumentV2;
  onApply: (document: FlowDocumentV2) => void;
  onClose: () => void;
  validate?: (document: FlowDocumentV2) => Promise<CompiledRevision>;
  exportFlow?: (id: string, revision: number | null) => Promise<string>;
}

export function FlowJsonDialog({
  document,
  onApply,
  onClose,
  validate = flowValidate,
  exportFlow = flowExport,
}: FlowJsonDialogProps) {
  const serializedDocument = JSON.stringify(document, null, 2);
  const [raw, setRaw] = useState(() => serializedDocument);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<"apply" | "export" | null>(null);

  useEffect(() => {
    setRaw(serializedDocument);
    setError(null);
  }, [serializedDocument]);

  const apply = async () => {
    setBusy("apply");
    setError(null);
    try {
      onApply(await importFlowJson(raw, validate));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(null);
    }
  };

  const loadExport = async () => {
    setBusy("export");
    setError(null);
    try {
      setRaw(await exportFlow(document.id, document.revision));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section role="dialog" aria-modal="true" aria-label="Flow JSON" className="flow-dialog">
      <header>
        <strong>Flow JSON</strong>
        <button type="button" aria-label="Close JSON dialog" title="Close" onClick={onClose}>
          <X aria-hidden="true" size={16} />
        </button>
      </header>
      <label className="flow-field">
        <span>Document JSON</span>
        <textarea
          value={raw}
          rows={20}
          spellCheck={false}
          onChange={(event) => setRaw(event.currentTarget.value)}
        />
      </label>
      <p>Advanced document view</p>
      {error && <p role="alert">{error}</p>}
      <footer>
        <button type="button" disabled={busy !== null} onClick={() => void loadExport()}>
          <Download aria-hidden="true" size={15} />
          {busy === "export" ? "Loading..." : "Load saved export"}
        </button>
        <button type="button" disabled={busy !== null} onClick={() => void apply()}>
          <Check aria-hidden="true" size={15} />
          {busy === "apply" ? "Validating..." : "Validate and apply"}
        </button>
      </footer>
    </section>
  );
}
