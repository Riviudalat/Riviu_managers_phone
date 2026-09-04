import type { GroupTab } from "../deviceGroups";
import { useId, useRef, type KeyboardEvent } from "react";

interface Props {
  tabs: GroupTab[];
  active: string;
  onSelect: (id: string) => void;
}

/**
 * Group tabs above the device grid.
 *
 * Renders even with no groups, showing only "Tất cả". A strip that appears and
 * disappears as groups are created moves the grid up and down under the operator's
 * cursor; one that is always there costs a row and never surprises.
 */
export function GroupTabs({ tabs, active, onSelect }: Props) {
  const baseId = useId();
  const buttons = useRef(new Map<string, HTMLButtonElement>());
  const choose = (id: string) => {
    onSelect(id);
    queueMicrotask(() => buttons.current.get(id)?.focus());
  };
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const nextIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? tabs.length - 1
          : (index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
    const next = tabs[nextIndex];
    if (next) choose(next.id);
  };

  return (
    <div className="group-tabs" role="tablist" aria-label="Nhóm thiết bị">
      {tabs.map((tab, index) => (
        <button
          id={`${baseId}-${tab.id}`}
          key={tab.id}
          ref={(element) => {
            if (element) buttons.current.set(tab.id, element);
            else buttons.current.delete(tab.id);
          }}
          type="button"
          role="tab"
          aria-selected={tab.id === active}
          tabIndex={tab.id === active ? 0 : -1}
          className={tab.id === active ? "active" : ""}
          onClick={() => choose(tab.id)}
          onKeyDown={(event) => onKeyDown(event, index)}
        >
          {tab.color && (
            <span className="group-tabs-dot" style={{ background: tab.color }} aria-hidden="true" />
          )}
          {tab.label}
          <span className="group-tabs-count" aria-label={`${tab.count} máy`}>
            {tab.count}
          </span>
        </button>
      ))}
    </div>
  );
}
