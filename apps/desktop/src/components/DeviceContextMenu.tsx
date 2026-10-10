import { useCallback, useMemo, useRef, useState } from "react";
import { describeError } from "../describeError";
import type { DeviceGroup, DeviceInfo } from "../types";
import {
  filterDeviceMenu,
  gateDeviceMenu,
  isSubmenu,
  type DeviceMenuNode,
} from "../deviceMenu";
import { ContextMenu, MenuGroup, MenuItem, MenuNote, MenuSubmenu } from "./ui/Menu";

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
 * The rows come from the same catalog, gating and search (`deviceMenu.ts`) as the focus
 * overlay's side panel (`DeviceFunctionList`), so zooming into a phone cannot lose a function
 * that the tile menu offers. The menu itself — roles, arrow keys, submenus, focus going back to
 * the tile — is the shared primitive in `ui/Menu.tsx`.
 *
 * Keyboard: Shift+F10 or the ContextMenu key on a tile opens it with the search box focused;
 * typing filters, ArrowDown enters the rows, ArrowRight opens a submenu, Enter runs a row,
 * Escape closes one level and then the menu.
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
  const [open, setOpen] = useState(true);
  const [query, setQuery] = useState("");
  /// Rows fetched from the phone, per lazy submenu id: `undefined` = never opened,
  /// `null` = still asking, an array = the answer (empty included, which is itself news).
  const [loaded, setLoaded] = useState<Record<string, DeviceMenuNode[] | null>>({});
  const searchRef = useRef<HTMLInputElement>(null);
  /// The chosen row's action, run once the menu has closed. The other order leaves a menu
  /// sitting over a confirm dialog the action opened, and hands focus back to the tile after
  /// that dialog took it.
  const chosen = useRef<(() => void) | null>(null);

  const gated = useMemo(() => gateDeviceMenu(nodes, device.platform), [nodes, device.platform]);
  const shown = useMemo(() => filterDeviceMenu(gated, query), [gated, query]);
  const searching = query.trim().length > 0;

  const choose = (action: (() => void) | undefined) => {
    chosen.current = action ?? null;
  };

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
            // nothing is indistinguishable from a phone that has nothing to show. The
            // sentence, not "[object Object]" — a command rejects with an object.
            [node.id]: [
              { id: `${node.id}-error`, label: describeError(error), disabled: true } as DeviceMenuNode,
            ],
          })),
        );
    },
    [loaded],
  );

  const renderLeaf = (node: DeviceMenuNode) => (
    <MenuItem
      key={node.id}
      icon={node.Icon}
      danger={node.danger}
      disabled={node.disabled}
      title={node.pathLabel ?? node.label}
      onSelect={() => choose(node.run)}
    >
      {node.pathLabel ?? node.label}
    </MenuItem>
  );

  const renderRow = (node: DeviceMenuNode) => {
    if (!isSubmenu(node)) return renderLeaf(node);
    const children = node.children ?? loaded[node.id] ?? [];
    const pending = loaded[node.id] === null;
    return (
      <MenuSubmenu
        key={node.id}
        label={node.label}
        icon={node.Icon}
        title={node.label}
        onOpenChange={(next) => {
          if (next) load(node);
        }}
      >
        {pending && <MenuNote>Đang đọc từ máy…</MenuNote>}
        {!pending && children.length === 0 && <MenuNote>Máy không trả về mục nào.</MenuNote>}
        {children.map(renderLeaf)}
      </MenuSubmenu>
    );
  };

  return (
    <ContextMenu
      open={open}
      point={{ x, y }}
      label={`Tác vụ cho ${device.name}`}
      onOpenChange={setOpen}
      onClosed={() => {
        const action = chosen.current;
        chosen.current = null;
        onClose();
        action?.();
      }}
      initialFocus={searchRef}
      header={
        <>
          <p className="device-menu-head" title={device.udid}>
            {device.name}
          </p>
          {/* The reference product's 搜索菜单 box. With this many rows it stops being a
              nicety: scrolling to find "Reset DPI" is slower than typing "dpi". */}
          <input
            ref={searchRef}
            className="device-menu-search"
            value={query}
            placeholder="Tìm chức năng…"
            aria-label="Tìm chức năng"
            onChange={(event) => setQuery(event.target.value)}
          />
        </>
      }
    >
      {shown.length === 0 && <MenuNote>Không có chức năng nào khớp.</MenuNote>}
      {shown.map(renderRow)}
      {/* Hidden while a search runs: the results are functions, and a group name matching
          the query would read as one. */}
      {groups.length > 0 && !searching && (
        <MenuGroup label="Thêm vào nhóm">
          {groups.map((group) => (
            <MenuItem
              key={group.id}
              marker={<span className="device-menu-dot" style={{ background: group.color }} />}
              onSelect={() => choose(() => onAddToGroup(group.id))}
            >
              {group.name}
            </MenuItem>
          ))}
        </MenuGroup>
      )}
    </ContextMenu>
  );
}
