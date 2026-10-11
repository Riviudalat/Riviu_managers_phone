import { Menu } from "@base-ui/react/menu";
import { useEffect, useId, useRef, useState, type ComponentType, type ReactNode, type RefObject } from "react";
import "./menu.css";

/**
 * The app's one menu primitive: Base UI's Menu, drawn with plain CSS.
 *
 * Base UI owns what a hand-rolled menu kept getting wrong — `menu`/`menuitem`/`group` roles
 * that pass axe, roving focus with the arrow keys, typeahead, Escape that closes one level,
 * submenus that survive the pointer crossing the gap, and focus going back to whatever was
 * focused before the menu opened (the tile, for the device menu). The styling stays ours,
 * through the scoped aliases in `menu.css`; nothing here redefines `--accent`.
 *
 * **Never modal.** A modal menu locks page scroll and blocks every pointer outside it, and the
 * phone windows behind it must keep taking clicks while a menu is open. A click outside still
 * closes the menu, as it always did.
 */

interface ContextMenuProps {
  open: boolean;
  /** Viewport point the menu opens at: the pointer, or the launcher's corner for the keyboard. */
  point: { x: number; y: number };
  /** The accessible name of the `menu` element. */
  label: string;
  onOpenChange: (open: boolean) => void;
  /**
   * After the menu has finished closing. The place to unmount it, and to run whatever the
   * chosen row does — running it earlier lets the menu hand focus back to the launcher *after*
   * the row's own dialog took it.
   */
  onClosed?: () => void;
  /**
   * Rendered above the rows, outside the `menu` element: a title and a search box are not menu
   * items, and inside `role="menu"` they are an axe `aria-required-children` failure. An
   * input here hands ArrowDown/ArrowUp to the rows.
   */
  header?: ReactNode;
  /** Focused once the menu is open instead of the first row; a header field, usually. */
  initialFocus?: RefObject<HTMLElement | null>;
  children: ReactNode;
}

export function ContextMenu({
  open,
  point,
  label,
  onOpenChange,
  onClosed,
  header,
  initialFocus,
  children,
}: ContextMenuProps) {
  const listRef = useRef<HTMLDivElement>(null);
  const triggerId = useId();
  // Whatever had focus when the menu opened — the tile, for the keyboard or a right-click —
  // read during the first render, before Base UI moves focus into the menu.
  const [launcher] = useState(() => {
    const active = document.activeElement;
    return { current: active instanceof HTMLElement && active !== document.body ? active : null };
  });

  // Base UI puts the opening focus on the list itself, a frame or two after open and later than
  // any effect here. Until the operator has done something, focus arriving on the bare list is
  // that opening move and is handed to `initialFocus`; after that it is the operator tabbing in.
  const opening = useRef(Boolean(initialFocus));
  useEffect(() => {
    if (!open || !initialFocus) return;
    const frame = window.requestAnimationFrame(() => initialFocus.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [open, initialFocus]);

  const enterList = (key: string) => {
    const rows = listRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])');
    if (!rows?.length) return false;
    (key === "ArrowUp" ? rows[rows.length - 1] : rows[0]).focus();
    return true;
  };

  return (
    <Menu.Root
      open={open}
      modal={false}
      // Hover is a look (`:hover` in menu.css), not focus: a mouse resting where the menu opens
      // would otherwise pull focus out of the search box and eat what the operator types.
      highlightItemOnHover={false}
      triggerId={triggerId}
      onOpenChange={(next) => onOpenChange(next)}
      onOpenChangeComplete={(next) => {
        if (!next) onClosed?.();
      }}
    >
      {/* A zero-size trigger at the point, never shown or tabbed to. Base UI positions against
          it — shifting the menu to stay on screen, which the old menu did with a
          fixed height budget — and links submenus to the root through it: without a mounted
          trigger, opening a submenu reads as a sibling menu opening and closes the root. */}
      <Menu.Trigger
        id={triggerId}
        className="ui-menu-anchor"
        style={{ left: point.x, top: point.y }}
        render={<span />}
        nativeButton={false}
        tabIndex={-1}
        aria-hidden="true"
        // The menu is `aria-labelledby` its trigger, so the trigger carries the menu's name.
        aria-label={label}
      />
      <Menu.Portal>
        {/* Shifted, never flipped, to stay on screen — a tall menu opened low on the page
            would flip above the point and lose its header off the top. */}
        <Menu.Positioner
          className="ui-menu-positioner"
          side="bottom"
          align="start"
          collisionPadding={8}
          collisionAvoidance={{ side: "shift", align: "shift" }}
        >
          <div className="ui-menu" onPointerDown={() => (opening.current = false)}>
            {header && (
              <div
                className="ui-menu-header"
                onKeyDown={(event) => {
                  opening.current = false;
                  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
                  if (enterList(event.key)) event.preventDefault();
                }}
              >
                {header}
              </div>
            )}
            {/* Focus goes back to the launcher, never to the hidden trigger. A tab stop, so Tab
                from the search box reaches the rows and the scrolling list is reachable by
                keyboard (axe `scrollable-region-focusable`). */}
            <Menu.Popup
              ref={listRef}
              className="ui-menu-list"
              aria-label={label}
              finalFocus={launcher}
              tabIndex={0}
              onFocus={(event) => {
                if (!opening.current || event.target !== event.currentTarget) return;
                opening.current = false;
                initialFocus?.current?.focus();
              }}
            >
              {children}
            </Menu.Popup>
          </div>
        </Menu.Positioner>
      </Menu.Portal>
    </Menu.Root>
  );
}

type MenuIcon = ComponentType<{ size?: number }>;

interface MenuItemProps {
  icon?: MenuIcon;
  /** Drawn where an icon would be, for a row whose mark is not an icon (a group's colour dot). */
  marker?: ReactNode;
  danger?: boolean;
  disabled?: boolean;
  title?: string;
  onSelect?: () => void;
  children: ReactNode;
}

export function MenuItem({ icon: Icon, marker, danger, disabled, title, onSelect, children }: MenuItemProps) {
  return (
    <Menu.Item
      className={`ui-menu-item${danger ? " is-danger" : ""}`}
      disabled={disabled}
      title={title}
      onClick={onSelect}
    >
      {Icon && <Icon size={16} />}
      {marker}
      <span className="ui-menu-text">{children}</span>
    </Menu.Item>
  );
}

/**
 * A sentence in place of rows — "still asking the phone", "nothing matched". A disabled item
 * rather than a paragraph, because a `menu` must own at least one item and a paragraph inside
 * one is not announced as part of it.
 */
export function MenuNote({ children }: { children: ReactNode }) {
  return (
    <Menu.Item className="ui-menu-item ui-menu-note" disabled>
      <span className="ui-menu-text">{children}</span>
    </Menu.Item>
  );
}

interface MenuSubmenuProps {
  label: string;
  icon?: MenuIcon;
  title?: string;
  /** Called when the submenu opens or closes; the hook for rows fetched on demand. */
  onOpenChange?: (open: boolean) => void;
  children: ReactNode;
}

/** Opens on hover, on click and on ArrowRight; ArrowLeft and Escape close only this level. */
export function MenuSubmenu({ label, icon: Icon, title, onOpenChange, children }: MenuSubmenuProps) {
  return (
    <Menu.SubmenuRoot onOpenChange={(next) => onOpenChange?.(next)}>
      <Menu.SubmenuTrigger className="ui-menu-item ui-menu-parent" title={title}>
        {Icon && <Icon size={16} />}
        <span className="ui-menu-text">{label}</span>
        <span className="ui-menu-chev" aria-hidden="true">▸</span>
      </Menu.SubmenuTrigger>
      <Menu.Portal>
        <Menu.Positioner className="ui-menu-positioner" sideOffset={2} alignOffset={-5} collisionPadding={8}>
          <Menu.Popup className="ui-menu ui-menu-list ui-menu-sub" aria-label={label}>
            {children}
          </Menu.Popup>
        </Menu.Positioner>
      </Menu.Portal>
    </Menu.SubmenuRoot>
  );
}

export function MenuGroup({ label, children }: { label: string; children: ReactNode }) {
  return (
    <Menu.Group className="ui-menu-group">
      <Menu.GroupLabel className="ui-menu-group-label">{label}</Menu.GroupLabel>
      {children}
    </Menu.Group>
  );
}

interface ButtonMenuProps {
  /** The trigger's visible text, which is also the menu's accessible name. */
  label: string;
  icon?: MenuIcon;
  className?: string;
  disabled?: boolean;
  children: ReactNode;
}

/**
 * A toolbar button that opens a short list of rarely used actions — the "Nâng cao" menu that
 * keeps raw-JSON import/export out of the everyday toolbar. Same rows, focus return and
 * non-modal behaviour as `ContextMenu`, anchored to a visible button instead of a point.
 */
export function ButtonMenu({ label, icon: Icon, className, disabled, children }: ButtonMenuProps) {
  return (
    <Menu.Root modal={false}>
      <Menu.Trigger className={className} disabled={disabled}>
        {Icon && <Icon size={16} />}
        {label}
        <span className="ui-menu-chev" aria-hidden="true">▾</span>
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Positioner
          className="ui-menu-positioner"
          side="bottom"
          align="end"
          sideOffset={4}
          collisionPadding={8}
        >
          <Menu.Popup className="ui-menu ui-menu-list" aria-label={label}>
            {children}
          </Menu.Popup>
        </Menu.Positioner>
      </Menu.Portal>
    </Menu.Root>
  );
}
