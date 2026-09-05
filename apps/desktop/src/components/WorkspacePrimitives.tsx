import {
  useEffect,
  useId,
  useRef,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { Check, X } from "lucide-react";

export type StatusTone = "neutral" | "info" | "success" | "warning" | "error";

export function CommandBar({
  title,
  detail,
  tone = "neutral",
  actions,
}: {
  title: ReactNode;
  detail?: ReactNode;
  tone?: StatusTone;
  actions: ReactNode;
}) {
  return (
    <div className={`workspace-command-bar is-${tone}`}>
      <div className="workspace-command-copy" role="status" aria-live="polite">
        <strong>{title}</strong>
        {detail && <span>{detail}</span>}
      </div>
      <div className="workspace-command-actions">{actions}</div>
    </div>
  );
}

export function PageHeader({
  title,
  icon,
  description,
  meta,
  actions,
  dragRegion = false,
  density = "default",
  titleTestId,
}: {
  title: string;
  icon?: ReactNode;
  description?: ReactNode;
  meta?: ReactNode;
  actions?: ReactNode;
  dragRegion?: boolean;
  density?: "default" | "compact";
  titleTestId?: string;
}) {
  return (
    <header className={`page-header is-${density}`}>
      {icon && <span className="page-header-icon" aria-hidden="true">{icon}</span>}
      <div className="page-header-copy">
        <h1 data-testid={titleTestId}>{title}</h1>
        {description && <p>{description}</p>}
      </div>
      {dragRegion && <div className="page-header-drag" aria-hidden="true" />}
      {meta && <div className="page-header-meta">{meta}</div>}
      {actions && <div className="page-header-actions">{actions}</div>}
    </header>
  );
}

export interface WorkspaceTab {
  id: string;
  label: string;
  disabled?: boolean;
  panelId?: string;
}

export function WorkspaceTabs({
  label,
  tabs,
  value,
  onChange,
}: {
  label: string;
  tabs: WorkspaceTab[];
  value: string;
  onChange: (value: string) => void;
}) {
  const baseId = useId();
  const selectAndFocus = (tab: WorkspaceTab) => {
    onChange(tab.id);
    queueMicrotask(() => document.getElementById(`${baseId}-${tab.id}`)?.focus());
  };
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const enabled = tabs
      .map((tab, tabIndex) => ({ tab, tabIndex }))
      .filter(({ tab }) => !tab.disabled);
    if (!enabled.length) return;
    const current = enabled.findIndex(({ tabIndex }) => tabIndex === index);
    const next =
      event.key === "Home"
        ? enabled[0]
        : event.key === "End"
          ? enabled[enabled.length - 1]
          : enabled[
              (current + (event.key === "ArrowRight" ? 1 : -1) + enabled.length) %
                enabled.length
            ];
    selectAndFocus(next.tab);
  };

  return (
    <div className="workspace-tabs" role="tablist" aria-label={label}>
      {tabs.map((tab, index) => (
        <button
          id={`${baseId}-${tab.id}`}
          key={tab.id}
          type="button"
          role="tab"
          aria-selected={tab.id === value}
          aria-controls={tab.panelId}
          tabIndex={tab.id === value ? 0 : -1}
          disabled={tab.disabled}
          onClick={() => selectAndFocus(tab)}
          onKeyDown={(event) => onKeyDown(event, index)}
        >
          {tab.label}
        </button>
      ))}
    </div>
  );
}

export type WorkflowStepState = "complete" | "current" | "upcoming" | "warning" | "error";

export interface WorkflowStep {
  id: string;
  label: string;
  description?: string;
  state?: WorkflowStepState;
}

export function WorkflowStepper({
  steps,
  current,
  label = "Tiến trình",
  onStepChange,
}: {
  steps: WorkflowStep[];
  current: string;
  label?: string;
  onStepChange?: (id: string) => void;
}) {
  const currentIndex = steps.findIndex((step) => step.id === current);
  return (
    <ol
      className="workflow-stepper"
      aria-label={label}
      style={{ "--workflow-step-count": steps.length } as CSSProperties}
    >
      {steps.map((step, index) => {
        const state =
          step.state ??
          (index < currentIndex ? "complete" : index === currentIndex ? "current" : "upcoming");
        const content = <>
            <span className="workflow-step-marker" aria-hidden="true">
              {state === "complete" ? <Check size={13} /> : index + 1}
            </span>
            <span className="workflow-step-copy">
              <strong>{step.label}</strong>
              {step.description && <small>{step.description}</small>}
            </span>
          </>;
        return (
          <li key={step.id} className={`workflow-step is-${state}`} aria-current={state === "current" ? "step" : undefined}>
            {onStepChange ? <button type="button" className="workflow-step-link"
              aria-label={step.label} aria-current={state === "current" ? "step" : undefined}
              onClick={() => onStepChange(step.id)}>{content}</button> : content}
          </li>
        );
      })}
    </ol>
  );
}

export function SummaryRail({
  title = "Tóm tắt",
  children,
  actions,
}: {
  title?: string;
  children: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <aside className="summary-rail" aria-label={title}>
      <div className="summary-rail-heading">
        <h2>{title}</h2>
        {actions}
      </div>
      <div className="summary-rail-content">{children}</div>
    </aside>
  );
}

export function StatusChip({
  tone = "neutral",
  children,
  title,
}: {
  tone?: StatusTone;
  children: ReactNode;
  title?: string;
}) {
  return (
    <span className={`status-chip is-${tone}`} title={title}>
      <span className="status-chip-dot" aria-hidden="true" />
      {children}
    </span>
  );
}

export { ResponsiveTable, type ResponsiveTableColumn } from "./table/ResponsiveTable";

export function DetailDrawer({
  open,
  title,
  description,
  onClose,
  children,
  footer,
}: {
  open: boolean;
  title: string;
  description?: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
}) {
  const titleId = useId();
  const descriptionId = useId();
  const drawerRef = useRef<HTMLElement>(null);
  const previousFocus = useRef<HTMLElement | null>(null);
  const onCloseRef = useRef(onClose);

  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    if (!open) return;
    previousFocus.current = document.activeElement as HTMLElement | null;
    const drawer = drawerRef.current;
    const focusable = () =>
      Array.from(
        drawer?.querySelectorAll<HTMLElement>(
          'button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])',
        ) ?? [],
      );
    focusable()[0]?.focus();
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onCloseRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const items = focusable();
      if (!items.length) return;
      const first = items[0];
      const last = items[items.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      previousFocus.current?.focus();
    };
  }, [open]);

  if (!open) return null;
  return (
    <div
      className="detail-drawer-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <aside
        ref={drawerRef}
        className="detail-drawer"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description ? descriptionId : undefined}
      >
        <header className="detail-drawer-header">
          <div>
            <h2 id={titleId}>{title}</h2>
            {description && <p id={descriptionId}>{description}</p>}
          </div>
          <button type="button" className="icon-btn" aria-label="Đóng" onClick={onClose}>
            <X size={18} aria-hidden="true" />
          </button>
        </header>
        <div className="detail-drawer-body">{children}</div>
        {footer && <footer className="detail-drawer-footer">{footer}</footer>}
      </aside>
    </div>
  );
}

export function FormSection({
  title,
  description,
  actions,
  children,
}: {
  title: string;
  description?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
}) {
  const titleId = useId();
  return (
    <section className="form-section" aria-labelledby={titleId}>
      <header className="form-section-header">
        <div>
          <h2 id={titleId}>{title}</h2>
          {description && <p>{description}</p>}
        </div>
        {actions && <div className="form-section-actions">{actions}</div>}
      </header>
      <div className="form-section-body">{children}</div>
    </section>
  );
}
