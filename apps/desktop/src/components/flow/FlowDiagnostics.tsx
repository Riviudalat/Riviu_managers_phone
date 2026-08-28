import type { FlowValidationIssue } from "../../types";

export function FlowDiagnostics({
  issues,
  pending = false,
  onSelectNode,
}: {
  issues: FlowValidationIssue[];
  /** A validation request is in flight, so an empty issue list is not yet an answer. */
  pending?: boolean;
  onSelectNode?: (nodeId: string) => void;
}) {
  return (
    <section className="flow-diagnostics" aria-label="Chẩn đoán Flow">
      <header>
        <strong>Chẩn đoán</strong>
        <span>{issues.length}</span>
      </header>
      {/* Each edit clears the issue list before the debounced request goes out, so deriving
          validity from emptiness alone announced "Hợp lệ" over a document that had just lost a
          required field -- and kept announcing it for as long as validation took, or forever if it
          hung. Save and Run are disabled meanwhile, so this was a lie the UI told, not a door it
          opened. */}
      {pending ? (
        <p role="status">Đang kiểm…</p>
      ) : issues.length === 0 ? (
        <p>Hợp lệ</p>
      ) : (
        <ul>
          {issues.map((issue, index) => (
            <li key={`${issue.code}-${issue.nodeId ?? "document"}-${index}`}>
              <button
                type="button"
                disabled={!issue.nodeId || !onSelectNode}
                onClick={() => issue.nodeId && onSelectNode?.(issue.nodeId)}
              >
                <strong>{issue.code}</strong>
                <span>{issue.message}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
