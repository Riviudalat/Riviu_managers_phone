import type { GroupTab } from "../deviceGroups";

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
  return (
    <div className="group-tabs" role="tablist" aria-label="Nhóm thiết bị">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          aria-selected={tab.id === active}
          className={tab.id === active ? "active" : ""}
          onClick={() => onSelect(tab.id)}
        >
          {tab.color && <span className="group-tabs-dot" style={{ background: tab.color }} />}
          {tab.label}
          <span className="group-tabs-count">{tab.count}</span>
        </button>
      ))}
    </div>
  );
}
