import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { describeError } from "../toastStore";
import { createPortal } from "react-dom";
import {
  filterDeviceMenu,
  gateDeviceMenu,
  isSubmenu,
  type DeviceMenuNode,
} from "../deviceMenu";
import type { DevicePlatform } from "../types";

interface Props {
  nodes: DeviceMenuNode[];
  platform: DevicePlatform;
  /** Called before a row runs, so a menu can close itself first. */
  onRun?: () => void;
  /** Hidden when the list is short enough to read; the menu always shows it. */
  showSearch?: boolean;
  /**
   * Focus the box as soon as the surface opens. True for the right-click menu, which is
   * opened *at* the pointer and answered by typing; false for the overlay panel, which
   * stays open while the operator drives the phone — stealing focus there would send their
   * next keystroke into a filter box instead of the phone.
   */
  autoFocusSearch?: boolean;
  /**
   * Mark the rows as `menuitem`.
   *
   * True inside the right-click menu, whose container carries `role="menu"` — rows there are
   * menu items and a screen reader should say so. False in the focus overlay's panel, which is
   * an `<aside>`: a `menuitem` with no `menu` around it is invalid ARIA and reads worse than
   * the plain button it actually is. Rows inside a flyout are always menu items, because the
   * flyout itself is a menu.
   */
  menuSemantics?: boolean;
}

/** Width of a flyout, in px. Matches `.device-flyout` in App.css; used to decide which side. */
const FLYOUT_WIDTH = 236;
/**
 * How long the pointer may be off both the row and its flyout before it closes.
 *
 * There is a gap of a couple of pixels between the two, and travelling across it is a normal
 * mouse movement — closing instantly makes a submenu impossible to reach, which is the classic
 * hover-menu bug. 180 ms is long enough to cross and short enough not to feel stuck.
 */
const FLYOUT_GRACE_MS = 180;

interface FlyoutState {
  id: string;
  /** Viewport coordinates, already clamped. */
  left: number;
  top: number;
}

/**
 * The per-phone function rows, rendered the same way wherever they appear.
 *
 * This exists because of a question with one right answer: "when I zoom into a phone, do I
 * still get those functions?" A tile's right-click menu and the focus overlay's side panel
 * are two views of the same phone, so a function present in one and missing in the other is a
 * bug in the product rather than a difference in layout. The rows come from one catalog
 * (`App.tsx`), the search and platform gating from one module (`deviceMenu.ts`), and the
 * drawing from here.
 *
 * **Submenus open on hover, in a flyout beside the row.** They used to expand inline on a
 * click, on the reasoning that a flyout must be clamped to the viewport at every level. The
 * clamping is real and is done below — but inline expansion was the wrong trade twice over: it
 * shifts every row underneath while the operator is reading them, and it makes reaching a
 * submenu a click when the reference product needs none. The flyout is rendered through a
 * **portal to `document.body`**: the overlay panel sits inside a transformed ancestor, and a
 * `position: fixed` child of one of those is positioned against *that* box rather than the
 * viewport, which is how a panel ends up pinned to the wrong corner.
 *
 * Clicking a submenu row still opens it, for touch and for keyboard.
 */
export function DeviceFunctionList({
  nodes,
  platform,
  onRun,
  showSearch = true,
  autoFocusSearch = false,
  menuSemantics = false,
}: Props) {
  const [query, setQuery] = useState("");
  const [flyout, setFlyout] = useState<FlyoutState | null>(null);
  /// Rows fetched from the phone, per lazy submenu id: `undefined` = never opened,
  /// `null` = still asking, an array = the answer (empty included, which is itself news).
  const [loaded, setLoaded] = useState<Record<string, DeviceMenuNode[] | null>>({});
  const closeTimer = useRef<number | null>(null);

  const gated = useMemo(() => gateDeviceMenu(nodes, platform), [nodes, platform]);
  const shown = useMemo(() => filterDeviceMenu(gated, query), [gated, query]);

  const cancelClose = useCallback(() => {
    if (closeTimer.current !== null) {
      window.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
  }, []);

  const scheduleClose = useCallback(() => {
    cancelClose();
    closeTimer.current = window.setTimeout(() => setFlyout(null), FLYOUT_GRACE_MS);
  }, [cancelClose]);

  // A timer that outlives the component would call `setFlyout` on an unmounted one, which is
  // exactly what happens when a row's action closes the menu while the pointer is still on it.
  useEffect(() => cancelClose, [cancelClose]);

  const load = useCallback(
    (node: DeviceMenuNode) => {
      if (!node.loadChildren || loaded[node.id] !== undefined) return;
      setLoaded((current) => ({ ...current, [node.id]: null }));
      void node
        .loadChildren()
        .then((rows) => setLoaded((current) => ({ ...current, [node.id]: rows })))
        .catch((error) =>
          setLoaded((current) => ({
            ...current,
            // The phone's own sentence, as an unclickable row. A submenu that silently shows
            // nothing is indistinguishable from a phone that has nothing to show.
            [node.id]: [
              {
                id: `${node.id}-error`,
                // The phone's sentence, not "[object Object]" — a command rejects with an
                // object and `String` of one says nothing.
                label: describeError(error),
                disabled: true,
              } as DeviceMenuNode,
            ],
          })),
        );
    },
    [loaded],
  );

  /// Where the flyout for this row goes: beside it if there is room, flipped to the other
  /// side if not, and never off the bottom.
  const openFlyout = (node: DeviceMenuNode, row: HTMLElement, childCount: number) => {
    cancelClose();
    load(node);
    const rect = row.getBoundingClientRect();
    const rightRoom = window.innerWidth - rect.right - 8;
    const left =
      rightRoom >= FLYOUT_WIDTH ? rect.right + 2 : Math.max(8, rect.left - FLYOUT_WIDTH - 2);
    // 30 px a row plus padding, capped the same way the stylesheet caps it. An estimate is
    // enough: it only decides how far up to nudge, and the panel scrolls internally.
    const estimated = Math.min(window.innerHeight * 0.6, Math.max(1, childCount) * 30 + 16);
    const top = Math.max(8, Math.min(rect.top, window.innerHeight - estimated - 8));
    setFlyout({ id: node.id, left, top });
  };

  const runLeaf = (node: DeviceMenuNode) => {
    // Closed first, then run. The other order leaves a menu sitting over a confirm dialog
    // the action opened.
    setFlyout(null);
    onRun?.();
    node.run?.();
  };

  const renderLeaf = (node: DeviceMenuNode, inFlyout = false) => {
    const Icon = node.Icon;
    return (
      <button
        key={node.id}
        type="button"
        role={menuSemantics || inFlyout ? "menuitem" : undefined}
        className={`${node.danger ? "danger" : ""}${node.disabled ? " is-disabled" : ""}`}
        disabled={node.disabled}
        title={node.pathLabel ?? node.label}
        onClick={() => runLeaf(node)}
      >
        {Icon && <Icon size={16} />}
        <span>{node.pathLabel ?? node.label}</span>
      </button>
    );
  };

  const renderRow = (node: DeviceMenuNode) => {
    if (!isSubmenu(node)) return renderLeaf(node);
    const children = node.children ?? loaded[node.id] ?? [];
    const Icon = node.Icon;
    const isOpen = flyout?.id === node.id;
    return (
      <button
        key={node.id}
        type="button"
        role={menuSemantics ? "menuitem" : undefined}
        className={`device-menu-parent${isOpen ? " is-open" : ""}`}
        title={node.label}
        aria-expanded={isOpen}
        aria-haspopup="menu"
        onPointerEnter={(event) => openFlyout(node, event.currentTarget, children.length)}
        onPointerLeave={scheduleClose}
        // Click opens it too: a hover-only submenu is unreachable by keyboard and on a
        // touch screen, and the operator may click before the pointer settles.
        onClick={(event) => openFlyout(node, event.currentTarget, children.length)}
        onFocus={(event) => openFlyout(node, event.currentTarget, children.length)}
      >
        {Icon && <Icon size={16} />}
        <span>{node.label}</span>
        <span className="device-menu-chev">▸</span>
      </button>
    );
  };

  const openNode = flyout
    ? shown.find((node) => node.id === flyout.id) ??
      shown.flatMap((node) => node.children ?? []).find((node) => node.id === flyout.id)
    : undefined;
  const openChildren = openNode ? openNode.children ?? loaded[openNode.id] ?? [] : [];
  const openPending = openNode ? loaded[openNode.id] === null : false;

  return (
    <>
      {showSearch && (
        /* The reference product's 搜索菜单 box. With this many rows it stops being a nicety:
           scrolling to find "Reset DPI" is slower than typing "dpi". */
        <input
          className="device-menu-search"
          value={query}
          autoFocus={autoFocusSearch}
          placeholder="Tìm chức năng…"
          aria-label="Tìm chức năng"
          onChange={(event) => setQuery(event.target.value)}
        />
      )}
      {shown.length === 0 && <p className="device-menu-note">Không có chức năng nào khớp.</p>}
      {shown.map((node) => renderRow(node))}

      {flyout &&
        openNode &&
        createPortal(
          <div
            className="device-flyout"
            role="menu"
            aria-label={openNode.label}
            style={{ left: flyout.left, top: flyout.top }}
            onPointerEnter={cancelClose}
            onPointerLeave={scheduleClose}
          >
            {openPending && <p className="device-menu-note">Đang đọc từ máy…</p>}
            {!openPending && openChildren.length === 0 && (
              <p className="device-menu-note">Máy không trả về mục nào.</p>
            )}
            {openChildren.map((child) => renderLeaf(child, true))}
          </div>,
          document.body,
        )}
    </>
  );
}
