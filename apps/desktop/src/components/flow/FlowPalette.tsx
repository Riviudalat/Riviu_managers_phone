import type { DragEvent } from "react";
import type { ActionCategory, ActionKind, ActionDefinition } from "../../types";
import { ACTION_PRESENTATION } from "./FlowActionNode";

export const FLOW_ACTION_MIME = "application/riviu-flow-action";

const CATEGORY_ORDER: ActionCategory[] = [
  "app",
  "input",
  "timing",
  "evidence",
  "control",
];

const CATEGORY_LABELS: Record<ActionCategory, string> = {
  app: "Ứng dụng",
  input: "Thao tác",
  timing: "Thời gian",
  evidence: "Bằng chứng",
  control: "Điều khiển",
};

/**
 * The two nodes the canvas owns, rather than ones an operator drops.
 *
 * This list is why the palette now filters by **kind**. It used to exclude the whole
 * `control` category — `Exclude<ActionCategory, "control">` — which does keep Start and End
 * out, but takes `ifVision` with them, because the backend files all four under `control`
 * (`catalog.rs::category`). `ifVision` is the only branching action there is, its two ports
 * are drawn, its config has a default and the compiler accepts it, so the whole feature was
 * finished and unreachable: no conditional flow could be built at all.
 *
 * `rawHttp`, `rawWda` and `shell` are also `control` and stay out, but for a different and
 * deliberate reason — they have no entry in `ACTION_PRESENTATION`, which is the list of
 * actions this UI has decided to offer. Putting them on the canvas is a separate decision,
 * not a consequence of this one.
 */
const STRUCTURAL_KINDS: ActionKind[] = ["start", "end"];

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
          (action) =>
            action.category === category &&
            !STRUCTURAL_KINDS.includes(action.kind) &&
            ACTION_PRESENTATION[action.kind],
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
                  title={action.disabledReason ?? presentation.label}
                  onDragStart={(event) => beginActionDrag(event, action)}
                >
                  <Icon aria-hidden="true" size={15} />
                  {/* Same source as the canvas node title, so an action cannot
                      be called one thing in the palette and another once
                      dropped. The backend's `label` stays the canonical name. */}
                  <span>{presentation.label}</span>
                </button>
              );
            })}
          </section>
        );
      })}
    </aside>
  );
}
