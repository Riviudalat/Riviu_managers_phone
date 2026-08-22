import { useEffect, useRef, useState } from "react";
import type { DeviceGroup, DeviceInfo } from "../types";
import type { DeviceMenuNode } from "../deviceMenu";
import { DeviceFunctionList } from "./DeviceFunctionList";

interface Props {
  device: DeviceInfo;
  groups: DeviceGroup[];
  x: number;
  y: number;
  nodes: DeviceMenuNode[];
  onAddToGroup: (groupId: string) => void;
  onClose: () => void;
}

/**
 * Right-click menu on a phone tile — the whole per-phone function menu.
 *
 * A context menu and not a hover toolbar, because that is what the reference product does
 * and because the alternative does not fit: the tile is a live video frame with a caption
 * over it, and a row of always-visible buttons would cover the screen the operator is
 * watching.
 *
 * Only actions with a backend appear here, and that rule is the whole design — a row that
 * calls a command this app never wrote is a button that fails, which is worse than its
 * absence. What the rule does *not* excuse is a row being absent because nobody built the
 * command: measured against xiaowei's own phone menu on 21/08/2026 this menu had ten rows
 * against its thirty-five, and the eight commands the difference needed were written that
 * day. If a row is missing now, check `api.ts` before assuming it cannot exist.
 *
 * The rows themselves are drawn by `DeviceFunctionList`, which the focus overlay's side panel
 * also uses — so zooming into a phone cannot lose a function that the tile menu offers.
 */
export function DeviceContextMenu({
  device,
  groups,
  x,
  y,
  nodes,
  onAddToGroup,
  onClose,
}: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [searching, setSearching] = useState(false);

  useEffect(() => {
    // Close on anything that means "I am done here". Pointerdown rather than click so a
    // press that starts outside cannot land on a menu item that moved under it.
    const away = (event: PointerEvent) => {
      if (!ref.current?.contains(event.target as Node)) onClose();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("keydown", escape);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("keydown", escape);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      className="device-menu"
      role="menu"
      aria-label={`Tác vụ cho ${device.name}`}
      // Clamped to the viewport: opened near the right or bottom edge, an unclamped menu
      // renders half off-screen and the rows underneath are unreachable. The menu is now
      // tall enough to need the vertical clamp on almost every click, not just near the
      // bottom — hence a fixed height budget rather than "wherever it fits".
      style={{
        left: Math.min(x, window.innerWidth - 264),
        top: Math.min(y, Math.max(8, window.innerHeight - 420)),
      }}
      // The group section hides while a search is running, and the flag has to come from
      // the box that owns the query — hence this listener rather than a second query state.
      onInput={(event) => {
        const target = event.target as HTMLElement;
        if (target instanceof HTMLInputElement) setSearching(target.value.trim().length > 0);
      }}
    >
      <p className="device-menu-head" title={device.udid}>
        {device.name}
      </p>
      <div className="device-menu-scroll">
        <DeviceFunctionList
          nodes={nodes}
          platform={device.platform}
          autoFocusSearch
          menuSemantics
          onRun={onClose}
        />

        {groups.length > 0 && !searching && (
          <>
            <p className="device-menu-label">Thêm vào nhóm</p>
            {groups.map((group) => (
              <button
                key={group.id}
                type="button"
                role="menuitem"
                onClick={() => {
                  onClose();
                  onAddToGroup(group.id);
                }}
              >
                <span className="device-menu-dot" style={{ background: group.color }} />
                {group.name}
              </button>
            ))}
          </>
        )}
      </div>
    </div>
  );
}
