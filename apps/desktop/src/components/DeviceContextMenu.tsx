import { useEffect, useRef } from "react";
import type { DeviceGroup, DeviceInfo } from "../types";

export interface DeviceMenuAction {
  id: string;
  label: string;
  danger?: boolean;
  run: () => void;
}

interface Props {
  device: DeviceInfo;
  groups: DeviceGroup[];
  x: number;
  y: number;
  actions: DeviceMenuAction[];
  onAddToGroup: (groupId: string) => void;
  onClose: () => void;
}

/**
 * Right-click menu on a phone tile.
 *
 * A context menu and not a hover toolbar, because that is what the reference product
 * does and because the alternative does not fit: the tile is a live video frame with a
 * caption over it, and a row of always-visible buttons would cover the screen the
 * operator is watching.
 *
 * Only actions with a backend appear here, and that rule is the whole design — a row that
 * calls a command this app never wrote is a button that fails, which is worse than its
 * absence. The rule has not changed; what changed is the list. An adb command box, rotate
 * and APK install were once named here as deliberate omissions, and all three were built
 * afterwards without this note being corrected, so it spent a while describing the opposite
 * of the truth. If a row is missing now, check `api.ts` before assuming it cannot exist.
 */
export function DeviceContextMenu({
  device,
  groups,
  x,
  y,
  actions,
  onAddToGroup,
  onClose,
}: Props) {
  const ref = useRef<HTMLDivElement>(null);

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
      // renders half off-screen and the rows underneath are unreachable.
      style={{
        left: Math.min(x, window.innerWidth - 208),
        top: Math.min(y, Math.max(8, window.innerHeight - 260)),
      }}
    >
      <p className="device-menu-head" title={device.udid}>
        {device.name}
      </p>
      {actions.map((action) => (
        <button
          key={action.id}
          type="button"
          role="menuitem"
          className={action.danger ? "danger" : ""}
          onClick={() => {
            onClose();
            action.run();
          }}
        >
          {action.label}
        </button>
      ))}
      {groups.length > 0 && (
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
  );
}
