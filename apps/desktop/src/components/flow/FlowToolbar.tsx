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
  hint,
  disabled = false,
  onClick,
  children,
}: PropsWithChildren<{
  label: string;
  /** Tooltip text when it needs to say more than the name -- usually why the command is off. */
  hint?: string;
  disabled?: boolean;
  onClick: () => void;
}>) {
  return (
    <button
      type="button"
      className="flow-icon-command"
      title={hint ?? label}
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
      <div className="flow-toolbar-group flow-toolbar-primary" role="group" aria-label="Chọn và chỉnh Flow">
        <IconCommand label="Bật/tắt bảng hành động" onClick={props.onTogglePalette}>
          <PanelLeft size={16} />
        </IconCommand>
        <select
          aria-label="Flow"
          value={props.currentFlowId ?? ""}
          onChange={(event) => props.onSelectFlow(event.target.value)}
        >
          <option value="" disabled>
            Chọn Flow
          </option>
          {props.flows.map((flow) => (
            <option key={flow.id} value={flow.id}>
              {flow.name} / bản {flow.latestRevision}
            </option>
          ))}
        </select>
        <input
          className="flow-name-input"
          aria-label="Tên Flow"
          value={props.flowName}
          maxLength={120}
          onChange={(event) => props.onRename(event.currentTarget.value)}
        />
        <IconCommand label="Flow mới" onClick={props.onNew}>
          <Plus size={16} />
        </IconCommand>
        <IconCommand
          label="Nhân bản Flow"
          disabled={!props.currentFlowId}
          onClick={props.onDuplicate}
        >
          <Copy size={16} />
        </IconCommand>
        <IconCommand
          label="Lưu trữ Flow"
          disabled={!props.currentFlowId}
          onClick={props.onArchive}
        >
          <Archive size={16} />
        </IconCommand>
      </div>
      <span className="flow-toolbar-separator" />
      <div className="flow-toolbar-group flow-toolbar-history" role="group" aria-label="Lịch sử và dữ liệu Flow">
        <IconCommand label="Hoàn tác" disabled={!props.canUndo} onClick={props.onUndo}>
          <Undo2 size={16} />
        </IconCommand>
        <IconCommand label="Làm lại" disabled={!props.canRedo} onClick={props.onRedo}>
          <Redo2 size={16} />
        </IconCommand>
        <IconCommand label="Lưu bản" disabled={!canSave} onClick={props.onSave}>
          <Save size={16} />
        </IconCommand>
        <IconCommand label="Kiểm tra Flow" onClick={() => setPreviewOpen(true)}>
          <CheckCircle size={16} />
        </IconCommand>
        <IconCommand label="Nhập Flow" onClick={props.onImport}>
          <Upload size={16} />
        </IconCommand>
        <IconCommand
          label="Xuất Flow"
          // The backend exports a *saved* revision, so a dirty graph exported the stored copy and
          // reported success -- the downloaded file was missing the edits on screen, under the name
          // on screen, with nothing saying so. Gate it like Run, and say why it is off.
          hint={
            props.dirty
              ? "Xuất Flow — bản xuất là bản đã lưu; hãy Lưu bản trước"
              : "Xuất Flow"
          }
          disabled={!props.currentFlowId || props.dirty}
          onClick={props.onExport}
        >
          <Download size={16} />
        </IconCommand>
        <IconCommand label="Xem JSON" onClick={props.onJson}>
          <Braces size={16} />
        </IconCommand>
      </div>
      <div className="flow-toolbar-group flow-toolbar-actions" role="group" aria-label="Chạy và bố cục Flow">
        <button
          type="button"
          className="flow-run-command"
          disabled={!canRun}
          onClick={props.onRun}
        >
          <Play size={16} />
          Chạy Flow
        </button>
        <IconCommand label="Bật/tắt bảng thuộc tính" onClick={props.onToggleInspector}>
          <PanelRight size={16} />
        </IconCommand>
      </div>
      {previewOpen && (
        <section role="dialog" aria-label="Xem trước biên dịch" className="flow-compile-preview">
          {/* Three states, not two: every edit clears `compiled` before the debounced request
              goes out, so equating `compiled === null` with invalid announced "Invalid" over a
              document that had not been checked yet -- and kept announcing it for as long as
              validation took. */}
          <strong>
            {props.validationPending ? "Đang kiểm…" : props.compiled ? "Hợp lệ" : "Chưa hợp lệ"}
          </strong>
          <details>
            <summary>Chi tiết kỹ thuật</summary>
            <code>
              {props.compiled
                ? JSON.stringify(props.compiled.plan.contextPlan)
                : "Chưa có kế hoạch ngữ cảnh"}
            </code>
            <ul>
              {props.compiled?.plan.requiredCapabilities.map((id) => <li key={id}>{id}</li>)}
            </ul>
            <ul>
              {disabledActions.map((action) => (
                <li key={action.kind}>
                  {action.kind}: {action.disabledReason}
                </li>
              ))}
            </ul>
          </details>
          <button type="button" onClick={() => setPreviewOpen(false)}>
            Đóng
          </button>
        </section>
      )}
    </header>
  );
}
