import type { DragEvent } from "react";
import type { ActionCategory, ActionDefinition } from "../../types";
import { ACTION_PRESENTATION } from "./FlowActionNode";

export const FLOW_ACTION_MIME = "application/riviu-flow-action";

const CATEGORY_ORDER: Exclude<ActionCategory, "control">[] = [
  "app",
  "input",
  "timing",
  "evidence",
];

const CATEGORY_LABELS: Record<Exclude<ActionCategory, "control">, string> = {
  app: "App",
  input: "Input",
  timing: "Timing",
  evidence: "Evidence",
};

function beginActionDrag(event: DragEvent, action: ActionDefinition) {
  event.dataTransfer.effectAllowed = "copy";
  event.dataTransfer.setData(FLOW_ACTION_MIME, action.kind);
}

export function FlowPalette({
  catalog,
  open,
}: {
  catalog: ActionDefinition[];
  open: boolean;
}) {
  return (
    <aside className="flow-palette" data-testid="flow-palette" data-open={String(open)}>
      {CATEGORY_ORDER.map((category) => {
        const actions = catalog.filter(
          (action) => action.category === category && ACTION_PRESENTATION[action.kind],
        );
        if (actions.length === 0) return null;
        return (
          <section key={category} aria-label={category}>
            <h3>{CATEGORY_LABELS[category]}</h3>
            {actions.map((action) => {
              const presentation = ACTION_PRESENTATION[action.kind];
              if (!presentation) return null;
              const Icon = presentation.icon;
              return (
                <button
                  key={action.kind}
                  type="button"
                  draggable={action.disabledReason === null}
                  disabled={action.disabledReason !== null}
                  aria-disabled={action.disabledReason !== null}
                  title={action.disabledReason ?? action.label}
                  onDragStart={(event) => beginActionDrag(event, action)}
                >
                  <Icon aria-hidden="true" size={15} />
                  <span>{action.label}</span>
                </button>
              );
            })}
          </section>
        );
      })}
    </aside>
  );
}
