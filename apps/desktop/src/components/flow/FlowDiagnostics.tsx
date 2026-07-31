import type { FlowValidationIssue } from "../../types";

export function FlowDiagnostics({
  issues,
  onSelectNode,
}: {
  issues: FlowValidationIssue[];
  onSelectNode?: (nodeId: string) => void;
}) {
  return (
    <section className="flow-diagnostics" aria-label="Flow diagnostics">
      <header>
        <strong>Diagnostics</strong>
        <span>{issues.length}</span>
      </header>
      {issues.length === 0 ? (
        <p>Valid</p>
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
