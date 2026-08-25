/**
 * The per-phone function menu, as a data structure (xiaowei 功能 menu).
 *
 * The reference product's phone menu is thirty-odd rows with two submenus and a search box
 * over the top, and the whole reason this file is pure is that all three of those are
 * *logic*: what a search matches, what a submenu contains, what a platform is not offered.
 * Getting any of them wrong is a menu row that lies, and none of it needs a DOM to be
 * proven.
 *
 * The rule the menu itself obeys has not changed and is stated in `DeviceContextMenu`: a row
 * exists only if a command behind it exists. This file cannot enforce that — a `run` is a
 * closure — but it is where the rows are shaped, so it is where the rule is repeated.
 */

import type { ReactElement } from "react";

export interface DeviceMenuNode {
  id: string;
  label: string;
  /**
   * Drawn beside the label where the surface has room for one — the focus overlay's panel
   * does, the right-click menu does not. Optional because the *rule* is that a row exists
   * when its command exists; an icon is decoration and must never gate a row.
   */
  Icon?: (props: { size?: number }) => ReactElement;
  /**
   * Extra words a search should match, for rows an operator would look for by a name the
   * label does not contain: `wifi` for "Bật/tắt Wi-Fi", `dpi` for "Đặt lại mật độ điểm",
   * `adb` for the whole submenu. Without these, a Vietnamese-labelled menu is unsearchable
   * by the English terms the reference product taught its operators.
   */
  keywords?: string;
  danger?: boolean;
  /** Not offered on iOS. Gated by `gateDeviceMenu`, never by the renderer. */
  androidOnly?: boolean;
  disabled?: boolean;
  /** A non-empty list makes this row a submenu, and `run` is then ignored. */
  children?: DeviceMenuNode[];
  /**
   * A submenu whose rows have to be asked of the phone — the installed keyboards, the
   * installed apps. The renderer calls this when the row is opened and shows what it
   * returns; a failure becomes one unclickable row carrying the phone's own reason.
   *
   * Kept out of `filterDeviceMenu`'s reach on purpose: search must not fire twenty adb
   * calls because somebody typed a letter, so an unopened lazy row matches by its own label
   * only. Its rows become searchable once it has been opened.
   */
  loadChildren?: () => Promise<DeviceMenuNode[]>;
  run?: () => void;
  /**
   * Set by `filterDeviceMenu` on a row it lifted out of a submenu, so the flattened result
   * can still say where the row lives ("ADB › Tắt Wi-Fi"). Never set by the caller.
   */
  pathLabel?: string;
}

/**
 * Strip diacritics for matching, so `cai dat` finds "Cài đặt".
 *
 * Every label in this app is Vietnamese and every keyboard an operator uses is not: typing
 * the accents to find a menu row is a tax nobody pays, they just conclude search is broken.
 * NFD splits a letter from its marks and the property escape removes the marks.
 */
export function foldAccents(text: string): string {
  return text
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    // đ/Đ has no combining form — it is its own codepoint — so NFD leaves it alone and a
    // search for "dat" would miss "đặt".
    .replace(/đ/g, "d")
    .replace(/Đ/g, "D")
    .toLowerCase();
}

/** Does this row's own text match? Children are not consulted. */
export function menuNodeMatches(node: DeviceMenuNode, query: string): boolean {
  const needle = foldAccents(query.trim());
  if (!needle) return true;
  const haystack = foldAccents(`${node.label} ${node.keywords ?? ""}`);
  return haystack.includes(needle);
}

/**
 * Apply the search box.
 *
 * An empty query returns the tree untouched — that is the ordinary menu. A query returns
 * **leaves**, lifted out of their submenus and labelled with where they came from, because
 * a search result the operator has to go hunting for inside a collapsed submenu is not a
 * search result. A parent that matches by its own name keeps its whole subtree, so typing
 * `adb` still offers everything under ADB.
 */
export function filterDeviceMenu(nodes: DeviceMenuNode[], query: string): DeviceMenuNode[] {
  if (!query.trim()) return nodes;
  const out: DeviceMenuNode[] = [];
  const walk = (list: DeviceMenuNode[], trail: string[]) => {
    for (const node of list) {
      const children = node.children ?? [];
      const label = trail.length > 0 ? `${trail.join(" › ")} › ${node.label}` : node.label;
      if (children.length === 0) {
        if (menuNodeMatches(node, query)) out.push({ ...node, pathLabel: label });
        continue;
      }
      if (menuNodeMatches(node, query)) {
        // The submenu itself is what matched, so it survives whole rather than flattened.
        out.push({ ...node, pathLabel: label });
        continue;
      }
      walk(children, [...trail, node.label]);
    }
  };
  walk(nodes, []);
  return out;
}

/**
 * Drop rows this platform cannot do, and then drop submenus that emptied out.
 *
 * The second half is the part that is easy to forget and impossible to miss once it
 * happens: an ADB submenu whose every row is Android-only must not appear on an iPhone as a
 * chevron that opens nothing.
 */
export function gateDeviceMenu(
  nodes: DeviceMenuNode[],
  platform: "android" | "ios",
): DeviceMenuNode[] {
  const out: DeviceMenuNode[] = [];
  for (const node of nodes) {
    if (node.androidOnly && platform !== "android") continue;
    if (node.children && node.children.length > 0) {
      const children = gateDeviceMenu(node.children, platform);
      if (children.length === 0) continue;
      out.push({ ...node, children });
      continue;
    }
    out.push(node);
  }
  return out;
}

/**
 * Drop rows by id, at any depth, and then drop submenus that emptied out.
 *
 * For a surface that already offers a row its own way. The focus overlay is the case: it has
 * an inline app panel, an inline keyboard picker and an inline adb console, all of which are
 * better there than the menu's versions — so it takes the shared catalog *minus* those, and
 * the operator sees one of each rather than two.
 *
 * By id and not by label on purpose: a label is copy and gets reworded, an id is a contract.
 */
export function withoutMenuIds(nodes: DeviceMenuNode[], drop: string[]): DeviceMenuNode[] {
  const unwanted = new Set(drop);
  const walk = (list: DeviceMenuNode[]): DeviceMenuNode[] => {
    const out: DeviceMenuNode[] = [];
    for (const node of list) {
      if (unwanted.has(node.id)) continue;
      if (node.children && node.children.length > 0) {
        const children = walk(node.children);
        // A submenu whose every row was dropped must go too, or it renders as a chevron
        // that opens onto nothing — the same trap `gateDeviceMenu` exists to avoid.
        if (children.length === 0) continue;
        out.push({ ...node, children });
        continue;
      }
      out.push(node);
    }
    return out;
  };
  return walk(nodes);
}

/** Does this row open onto more rows, whether it already has them or must go and ask? */
export function isSubmenu(node: DeviceMenuNode): boolean {
  return (node.children?.length ?? 0) > 0 || node.loadChildren !== undefined;
}

/** Every leaf in the tree, for a test or a keyboard walk. Submenu rows are not leaves. */
export function menuLeaves(nodes: DeviceMenuNode[]): DeviceMenuNode[] {
  const out: DeviceMenuNode[] = [];
  for (const node of nodes) {
    if (node.children && node.children.length > 0) out.push(...menuLeaves(node.children));
    else out.push(node);
  }
  return out;
}
