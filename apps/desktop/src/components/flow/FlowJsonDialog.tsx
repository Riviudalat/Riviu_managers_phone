import { Check, Download, X } from "lucide-react";
import { useEffect, useState } from "react";
import { flowExport, flowValidate } from "../../api";
import { describeError } from "../../describeError";
import { isFlowDocumentV2, normalizeFlowIssues } from "../../flow/validation";
import type { CompiledRevision, FlowDocumentV2 } from "../../types";

const MAX_FLOW_JSON_BYTES = 1_048_576;

/**
 * One line for a rejected `flow_validate`.
 *
 * `flow_validate` is the only command in the app that rejects with a **`Vec<CommandError>`**
 * (`flow_commands.rs:69`) — an *array* of objects. `String(...)` on that yields
 * `"[object Object]"`, which is exactly what this dialog used to show instead of naming the node
 * that failed to compile. `normalizeFlowIssues` already existed for this shape and handles all
 * three cases (array, single, neither); the join is because this dialog has one error line.
 */
function describeValidationFailure(reason: unknown): string {
  return normalizeFlowIssues(reason)
    .map((issue) => (issue.code ? `${issue.code}: ${issue.message}` : issue.message))
    .join(" · ");
}

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
  // Translated here rather than in the caller's catch, because only this line knows the
  // rejection is a `Vec<CommandError>`; everything else this function throws is a real `Error`.
  try {
    await validate(document);
  } catch (reason) {
    throw new Error(describeValidationFailure(reason));
  }
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
      setError(describeError(reason));
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
      setError(describeError(reason));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section role="dialog" aria-modal="true" aria-label="Flow JSON" className="flow-dialog">
      <header>
        <strong>Flow JSON</strong>
        <button type="button" aria-label="Đóng hộp thoại JSON" title="Đóng" onClick={onClose}>
          <X aria-hidden="true" size={16} />
        </button>
      </header>
      <label className="flow-field">
        <span>JSON tài liệu</span>
        <textarea
          value={raw}
          rows={20}
          spellCheck={false}
          onChange={(event) => setRaw(event.currentTarget.value)}
        />
      </label>
      <p>Xem tài liệu nâng cao</p>
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
