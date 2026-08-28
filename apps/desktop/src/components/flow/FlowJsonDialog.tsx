import { Check, Download, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
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
  const [stale, setStale] = useState(false);
  // The text the textarea was last filled from. Comparing against it is how the dialog tells
  // "the operator has not touched this" from "the operator has edits I must not throw away".
  const synced = useRef(serializedDocument);
  // The current text, readable from the effect below without making the effect depend on it.
  const typed = useRef(raw);
  typed.current = raw;
  // Validation is asynchronous while both the close button and the textarea stay live, and nothing
  // used to bind the result to the text that was submitted. So clicking Validate and then editing,
  // or closing, still applied the click-time JSON when the old request resolved: the latest visible
  // edits vanished, or an explicitly closed dialog rewrote the document.
  const live = useRef(true);
  const submission = useRef(0);
  useEffect(() => () => {
    live.current = false;
  }, []);

  // Deliberately keyed to `serializedDocument` alone. Depending on `raw` as well meant every
  // keystroke re-entered this and announced an external change that had not happened.
  useEffect(() => {
    if (typed.current === synced.current) {
      // Untouched: follow the document.
      synced.current = serializedDocument;
      setRaw(serializedDocument);
      setError(null);
      setStale(false);
      return;
    }
    // Edited, and a new revision arrived underneath -- a `flowUpdated` invalidation, say. Silently
    // resetting the textarea threw the operator's work away with no message, so keep it and say the
    // flow moved; "Load saved export" is the way back.
    setStale(true);
  }, [serializedDocument]);

  const close = () => {
    live.current = false;
    onClose();
  };

  const apply = async () => {
    const ticket = (submission.current += 1);
    const submitted = raw;
    setBusy("apply");
    setError(null);
    try {
      const applied = await importFlowJson(submitted, validate);
      // Only if this is still the newest submission, the dialog is still open, and the text has not
      // moved on since it was sent.
      if (!live.current || submission.current !== ticket) return;
      // `typed.current`, not `raw`: `raw` here is the value captured by the render this callback
      // was created in, so comparing it to `submitted` compared a value with itself.
      if (typed.current !== submitted) {
        setError("JSON đã thay đổi trong lúc kiểm tra — bấm lại để kiểm bản hiện tại.");
        return;
      }
      onApply(applied);
    } catch (reason) {
      if (live.current && submission.current === ticket) setError(describeError(reason));
    } finally {
      if (live.current && submission.current === ticket) setBusy(null);
    }
  };

  const loadExport = async () => {
    setBusy("export");
    setError(null);
    try {
      const exported = await exportFlow(document.id, document.revision);
      if (!live.current) return;
      synced.current = exported;
      setRaw(exported);
      setStale(false);
    } catch (reason) {
      if (live.current) setError(describeError(reason));
    } finally {
      if (live.current) setBusy(null);
    }
  };

  return (
    <section role="dialog" aria-modal="true" aria-label="Flow JSON" className="flow-dialog">
      <header>
        <strong>Flow JSON</strong>
        <button type="button" aria-label="Đóng hộp thoại JSON" title="Đóng" onClick={close}>
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
      {stale && (
        <p role="alert">
          Flow đã được cập nhật ở nơi khác trong lúc anh sửa. Văn bản dưới đây vẫn là của anh — bấm
          “Load saved export” nếu muốn lấy bản mới nhất.
        </p>
      )}
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
