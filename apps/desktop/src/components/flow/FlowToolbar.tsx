import {
  Archive,
  Braces,
  CheckCircle,
  Copy,
  Download,
  PanelLeft,
  PanelRight,
  Play,
  Plus,
  Redo2,
  Save,
  Undo2,
  Upload,
} from "lucide-react";
import { useState, type PropsWithChildren } from "react";
import type {
  ActionDefinition,
  CompiledRevision,
  FlowSummary,
  FlowValidationIssue,
} from "../../types";

interface FlowToolbarProps {
  flows: FlowSummary[];
  currentFlowId: string | null;
  flowName: string;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  compiled: CompiledRevision | null;
  issues: FlowValidationIssue[];
  catalog: ActionDefinition[];
  validationPending: boolean;
  savePending: boolean;
  onSelectFlow: (id: string) => void;
  onRename: (name: string) => void;
  onNew: () => void;
  onDuplicate: () => void;
  onArchive: () => void;
  onSave: () => void;
  onRun: () => void;
  onImport: () => void;
  onExport: () => void;
  onJson: () => void;
  onUndo: () => void;
  onRedo: () => void;
  onTogglePalette: () => void;
  onToggleInspector: () => void;
}

function IconCommand({
  label,
  disabled = false,
  onClick,
  children,
}: PropsWithChildren<{
  label: string;
  disabled?: boolean;
  onClick: () => void;
}>) {
  return (
    <button
      type="button"
      className="flow-icon-command"
      title={label}
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

export function FlowToolbar(props: FlowToolbarProps) {
  const [previewOpen, setPreviewOpen] = useState(false);
  const disabledActions = props.catalog.filter(
    (action): action is ActionDefinition & { disabledReason: string } =>
      action.disabledReason !== null,
  );
  const canSave =
    props.dirty &&
    props.compiled !== null &&
    props.issues.length === 0 &&
    !props.validationPending &&
    !props.savePending;
  const canRun =
    !props.dirty &&
    props.compiled !== null &&
    props.currentFlowId !== null &&
    !props.validationPending &&
    !props.savePending;

  return (
    <header className="flow-toolbar" data-testid="flow-toolbar">
      <IconCommand label="Toggle action palette" onClick={props.onTogglePalette}>
        <PanelLeft size={16} />
      </IconCommand>
      <select
        aria-label="Flow"
        value={props.currentFlowId ?? ""}
        onChange={(event) => props.onSelectFlow(event.target.value)}
      >
        <option value="" disabled>
          Select flow
        </option>
        {props.flows.map((flow) => (
          <option key={flow.id} value={flow.id}>
            {flow.name} / r{flow.latestRevision}
          </option>
        ))}
      </select>
      <input
        className="flow-name-input"
        aria-label="Flow name"
        value={props.flowName}
        maxLength={120}
        onChange={(event) => props.onRename(event.currentTarget.value)}
      />
      <IconCommand label="New flow" onClick={props.onNew}>
        <Plus size={16} />
      </IconCommand>
      <IconCommand
        label="Duplicate flow"
        disabled={!props.currentFlowId}
        onClick={props.onDuplicate}
      >
        <Copy size={16} />
      </IconCommand>
      <IconCommand
        label="Archive flow"
        disabled={!props.currentFlowId}
        onClick={props.onArchive}
      >
        <Archive size={16} />
      </IconCommand>
      <span className="flow-toolbar-separator" />
      <IconCommand label="Undo" disabled={!props.canUndo} onClick={props.onUndo}>
        <Undo2 size={16} />
      </IconCommand>
      <IconCommand label="Redo" disabled={!props.canRedo} onClick={props.onRedo}>
        <Redo2 size={16} />
      </IconCommand>
      <IconCommand label="Save revision" disabled={!canSave} onClick={props.onSave}>
        <Save size={16} />
      </IconCommand>
      <IconCommand label="Validate flow" onClick={() => setPreviewOpen(true)}>
        <CheckCircle size={16} />
      </IconCommand>
      <IconCommand label="Import flow" onClick={props.onImport}>
        <Upload size={16} />
      </IconCommand>
      <IconCommand
        label="Export flow"
        disabled={!props.currentFlowId}
        onClick={props.onExport}
      >
        <Download size={16} />
      </IconCommand>
      <IconCommand label="View JSON" onClick={props.onJson}>
        <Braces size={16} />
      </IconCommand>
      <button
        type="button"
        className="flow-run-command"
        disabled={!canRun}
        onClick={props.onRun}
      >
        <Play size={16} />
        Run flow
      </button>
      <IconCommand label="Toggle inspector" onClick={props.onToggleInspector}>
        <PanelRight size={16} />
      </IconCommand>
      {previewOpen && (
        <section role="dialog" aria-label="Compile preview" className="flow-compile-preview">
          <strong>{props.compiled ? "Valid" : "Invalid"}</strong>
          <code>
            {props.compiled
              ? JSON.stringify(props.compiled.plan.contextPlan)
              : "No context plan"}
          </code>
          <ul>
            {props.compiled?.plan.requiredCapabilities.map((id) => <li key={id}>{id}</li>)}
          </ul>
          <ul>
            {disabledActions.map((action) => (
              <li key={action.kind}>
                {action.label}: {action.disabledReason}
              </li>
            ))}
          </ul>
          <button type="button" onClick={() => setPreviewOpen(false)}>
            Close
          </button>
        </section>
      )}
    </header>
  );
}
