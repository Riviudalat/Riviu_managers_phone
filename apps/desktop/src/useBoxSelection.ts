import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";

import { applyBoxSelection, isDragMeaningful, normalizeBox, tilesInBox } from "./boxSelect";
import type { Rect, TileRect } from "./boxSelect";
import type { DeviceInfo } from "./types";
import { excludeBlocked, type ExcludedDevice, type SelectionBlock } from "./selectionEligibility";

export type { ExcludedDevice, SelectionBlock };
/** A phone an explicit action tried to add and could not, and why. */
export interface RefusedDevice { udid: string; reason: string }

/**
 * Selecting phones on the grid: click, shift-click, marquee, and Ctrl/Cmd+A.
 *
 * The arithmetic already lived apart in `boxSelect.ts` and was already tested; what stayed
 * in `App.tsx` was the state and the two window-level listeners around it. Those are the
 * parts that were unreachable from a test, and they are the parts that carry the rules —
 * which selector finds a tile, when a drag counts, and what Ctrl+A must not steal.
 *
 * The selection is the set of phones the next command reaches, keyed by udid, so it never
 * follows a grid position. Two rules guard it (PLAN §3): a phone that cannot take new work
 * (busy, disconnected, gone) is never *added*, because every path that adds goes through one
 * check; and a selected phone that becomes so *leaves* the selection for `excluded`, with its
 * reason. Nothing puts it back when it frees up: only the operator selects it again.
 */
export interface BoxSelectionOptions {
  blockedReason?: SelectionBlock;
  /// Called with the phones an explicit action tried to add and could not.
  onRefused?: (refused: RefusedDevice[]) => void;
}

export interface BoxSelection {
  selected: string[];
  /// Replaces the selection; phones that cannot take new work are refused, not added.
  setSelected: (next: string[]) => void;
  /// Empties the selection and the "Bị loại" list together.
  clearSelection: () => void;
  excluded: ExcludedDevice[];
  dismissExcluded: () => void;
  selectedDevices: DeviceInfo[];
  /// Click a tile: plain click replaces the selection, modified click toggles.
  onSelect: (udid: string, additive: boolean) => void;
  /// Put on the grid container; also the element the marquee measures tiles inside.
  canvasRef: React.RefObject<HTMLDivElement | null>;
  onCanvasMouseDown: (event: ReactMouseEvent<HTMLDivElement>) => void;
  /// The live marquee in client coordinates, or null when no drag is in progress.
  band: Rect | null;
}

export function useBoxSelection(
  devices: DeviceInfo[],
  visibleDevices: DeviceInfo[],
  /// Ctrl/Cmd+A only applies while the grid is actually on screen.
  gridIsUp: boolean,
  options: BoxSelectionOptions = {},
): BoxSelection {
  const [selected, setRawSelected] = useState<string[]>([]);
  const [excluded, setExcluded] = useState<ExcludedDevice[]>([]);
  const { blockedReason } = options;
  // Handlers read the latest render's selection and rules; a stale closure would test a phone
  // against the busy set of a render from before it went busy. A layout effect, so no input
  // event can land between a commit and this update.
  const latest = useRef({ selected, options });
  useLayoutEffect(() => {
    latest.current = { selected, options };
  });
  // Splits the phones an action would add into those that may take new work and those refused.
  // Phones already selected pass: taking them out is the exclusion effect's job, not this one's.
  const admit = useCallback((udids: string[]) => {
    const { selected: current, options: { blockedReason: block } } = latest.current;
    const allowed: string[] = [];
    const refused: RefusedDevice[] = [];
    for (const udid of new Set(udids)) {
      const reason = current.includes(udid) ? null : block?.(udid) ?? null;
      if (reason) refused.push({ udid, reason });
      else allowed.push(udid);
    }
    return { allowed, refused };
  }, []);
  // Selecting an excluded phone again is the operator's decision, so it leaves "Bị loại".
  const forgive = useCallback((udids: string[]) => {
    if (!udids.length) return;
    setExcluded((prev) => prev.some((entry) => udids.includes(entry.udid))
      ? prev.filter((entry) => !udids.includes(entry.udid))
      : prev);
  }, []);
  const setSelected = useCallback((next: string[]) => {
    const { allowed, refused } = admit(next);
    if (refused.length) latest.current.options.onRefused?.(refused);
    // An action whose every new phone was refused changes nothing; it never empties the set.
    if (next.length && !allowed.length) return;
    forgive(allowed);
    setRawSelected(allowed);
  }, [admit, forgive]);
  const canvasRef = useRef<HTMLDivElement | null>(null);
  // Rubber-band (box) selection over the window grid (A7). `bandOrigin` holds the mousedown
  // point and modifier; `band` is the live rectangle in client coords, non-null only while
  // dragging (which is also the effect's on/off signal).
  const bandOrigin = useRef<{ x: number; y: number; additive: boolean } | null>(null);
  const [band, setBand] = useState<Rect | null>(null);

  // Start a marquee only from empty canvas space with the left button; a mousedown that
  // lands on a tile is that tile's own click, not a selection box.
  const onCanvasMouseDown = (event: ReactMouseEvent<HTMLDivElement>) => {
    if (event.button !== 0 || event.target !== event.currentTarget) return;
    // Stops the browser starting a *text* selection under the marquee. Without it, dragging a
    // box across the grid highlighted the captions it passed over — the tile number, model and
    // OS came back blue — so the gesture that selects phones also looked like it was selecting
    // words. `preventDefault` here covers the whole drag; `user-select: none` on the tile
    // covers a stray drag that begins inside one.
    event.preventDefault();
    bandOrigin.current = {
      x: event.clientX,
      y: event.clientY,
      additive: event.shiftKey || event.ctrlKey || event.metaKey,
    };
    setBand(normalizeBox(event.clientX, event.clientY, event.clientX, event.clientY));
  };

  // While a marquee is live, track the pointer on `window` (not the canvas) so a drag that
  // leaves the grid still updates and still commits on release. Attaches once per drag.
  const dragging = band !== null;
  useEffect(() => {
    if (!dragging) return;
    const onMove = (event: MouseEvent) => {
      const origin = bandOrigin.current;
      if (origin) setBand(normalizeBox(origin.x, origin.y, event.clientX, event.clientY));
    };
    const onUp = (event: MouseEvent) => {
      const origin = bandOrigin.current;
      bandOrigin.current = null;
      setBand(null);
      const canvas = canvasRef.current;
      if (!origin || !canvas) return;
      if (!isDragMeaningful(origin.x, origin.y, event.clientX, event.clientY)) return;
      const box = normalizeBox(origin.x, origin.y, event.clientX, event.clientY);
      // `.dev-phone[data-udid]`, not `[data-udid]`: a tile carries that attribute on three
      // elements — the article, `PhoneCanvas`'s host div, and the canvas once a stream
      // attaches — so the bare selector returned the same phone two or three times and the
      // selection held duplicates. Measured on the 20-phone fleet: a box over three tiles
      // gave the toolbar 3 and the sidebar 6. `tilesInBox` de-duplicates as well, because a
      // duplicated udid reaches `group_input` and sends every group action to that phone twice.
      const tiles: TileRect[] = Array.from(
        canvas.querySelectorAll<HTMLElement>(".dev-phone[data-udid]"),
      ).map((el) => {
        const r = el.getBoundingClientRect();
        return {
          udid: el.dataset.udid ?? "",
          rect: { left: r.left, top: r.top, right: r.right, bottom: r.bottom },
        };
      });
      const { allowed, refused } = admit(tilesInBox(box, tiles));
      if (refused.length) latest.current.options.onRefused?.(refused);
      // A box over nothing but busy phones leaves the selection as it was.
      if (refused.length && !allowed.length) return;
      forgive(allowed);
      setRawSelected((prev) => applyBoxSelection(prev, allowed, origin.additive));
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [dragging, admit, forgive]);

  // Ctrl/Cmd+A selects every phone in the current tab while the grid is up — the farm
  // shortcut from xiaowei. Ignored while typing in a field so it never steals the browser's
  // own select-all inside an input.
  useEffect(() => {
    if (!gridIsUp) return;
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "a") return;
      const target = event.target as HTMLElement | null;
      const tag = target?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || target?.isContentEditable) return;
      if (target?.closest?.(".automation-host")) return;
      event.preventDefault();
      setSelected(visibleDevices.map((device) => device.udid));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [gridIsUp, visibleDevices, setSelected]);

  // Memoised as it was in `App.tsx`: this is handed to children as a prop, so a fresh
  // array every render would be a re-render of the grid on every keystroke elsewhere.
  const selectedDevices = useMemo(
    () => devices.filter((d) => selected.includes(d.udid)),
    [devices, selected],
  );

  // A selected phone that can no longer take new work leaves the receiving set and says why.
  useEffect(() => {
    if (!blockedReason || !selected.length) return;
    const { excluded: leaving } = excludeBlocked(selected, blockedReason);
    if (!leaving.length) return;
    const gone = new Set(leaving.map((entry) => entry.udid));
    setRawSelected((prev) => prev.filter((udid) => !gone.has(udid)));
    setExcluded((prev) => [...prev.filter((entry) => !gone.has(entry.udid)), ...leaving]);
  }, [blockedReason, selected]);

  const onSelect = (udid: string, additive: boolean) => {
    const prev = latest.current.selected;
    if (prev.includes(udid)) {
      if (additive) setRawSelected(prev.filter((x) => x !== udid));
      else setRawSelected(prev.length === 1 ? [] : [udid]);
      return;
    }
    const { refused } = admit([udid]);
    if (refused.length) {
      latest.current.options.onRefused?.(refused);
      return;
    }
    forgive([udid]);
    setRawSelected(additive ? [...prev, udid] : [udid]);
  };

  const clearSelection = useCallback(() => {
    setRawSelected([]);
    setExcluded([]);
  }, []);
  const dismissExcluded = useCallback(() => setExcluded([]), []);

  return {
    selected,
    setSelected,
    clearSelection,
    excluded,
    dismissExcluded,
    selectedDevices,
    onSelect,
    canvasRef,
    onCanvasMouseDown,
    band,
  };
}

/**
 * Keeps a layout snapshot for as long as a pointer button stays down (PLAN §3).
 *
 * Pass what decides the layout (which phones show, in what order, and the notices above them);
 * while held, `shown` is the value from the moment of the press. Only the pointer ends a hold:
 * its own release (applied after the click that release produces, so the click lands on the
 * pressed tile), a cancel, the window losing focus mid-gesture, or, for a release the window
 * never heard, the next pointer event proving no button is down. Elapsed time never does: a
 * press held for a minute keeps its layout for that minute.
 */
export function usePointerHold<T>(live: T): {
  shown: T;
  held: boolean;
  onPointerDownCapture: (event: ReactPointerEvent<HTMLElement>) => void;
} {
  const [hold, setHold] = useState<{ pointerId: number; snapshot: T } | null>(null);
  const latestLive = useRef(live);
  latestLive.current = live;
  const onPointerDownCapture = useCallback((event: ReactPointerEvent<HTMLElement>) => {
    const { pointerId } = event;
    setHold((current) => current ?? { pointerId, snapshot: latestLive.current });
  }, []);
  useEffect(() => {
    if (!hold) return;
    const { pointerId } = hold;
    let afterClick = 0;
    const end = () => setHold(null);
    // The click a release produces is dispatched in the same input task as the release;
    // ending on the next task orders the end after that click. The release is the authority.
    const released = (event: PointerEvent) => {
      if (event.pointerId !== pointerId) return;
      window.clearTimeout(afterClick);
      afterClick = window.setTimeout(end, 0);
    };
    const cancelled = (event: PointerEvent) => {
      if (event.pointerId === pointerId) end();
    };
    // A release outside the window that was never delivered: a later pointer event with no
    // button down proves the press is over. One with the button still down keeps the hold.
    const proved = (event: PointerEvent) => {
      if (event.buttons === 0) end();
    };
    window.addEventListener("pointerup", released, true);
    window.addEventListener("pointercancel", cancelled, true);
    window.addEventListener("pointermove", proved, true);
    window.addEventListener("pointerover", proved, true);
    window.addEventListener("blur", end);
    return () => {
      window.clearTimeout(afterClick);
      window.removeEventListener("pointerup", released, true);
      window.removeEventListener("pointercancel", cancelled, true);
      window.removeEventListener("pointermove", proved, true);
      window.removeEventListener("pointerover", proved, true);
      window.removeEventListener("blur", end);
    };
  }, [hold]);
  return { shown: hold ? hold.snapshot : live, held: hold !== null, onPointerDownCapture };
}
