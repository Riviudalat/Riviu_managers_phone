import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react";

import { applyBoxSelection, isDragMeaningful, normalizeBox, tilesInBox } from "./boxSelect";
import type { Rect, TileRect } from "./boxSelect";
import type { DeviceInfo } from "./types";

/**
 * Selecting phones on the grid: click, shift-click, marquee, and Ctrl/Cmd+A.
 *
 * The arithmetic already lived apart in `boxSelect.ts` and was already tested; what stayed
 * in `App.tsx` was the state and the two window-level listeners around it. Those are the
 * parts that were unreachable from a test, and they are the parts that carry the rules —
 * which selector finds a tile, when a drag counts, and what Ctrl+A must not steal.
 */
export interface BoxSelection {
  selected: string[];
  setSelected: React.Dispatch<React.SetStateAction<string[]>>;
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
): BoxSelection {
  const [selected, setSelected] = useState<string[]>([]);
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
      const hits = tilesInBox(box, tiles);
      setSelected((prev) => applyBoxSelection(prev, hits, origin.additive));
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [dragging]);

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
  }, [gridIsUp, visibleDevices]);

  // Memoised as it was in `App.tsx`: this is handed to children as a prop, so a fresh
  // array every render would be a re-render of the grid on every keystroke elsewhere.
  const selectedDevices = useMemo(
    () => devices.filter((d) => selected.includes(d.udid)),
    [devices, selected],
  );

  const onSelect = (udid: string, additive: boolean) => {
    setSelected((prev) => {
      if (additive) {
        return prev.includes(udid) ? prev.filter((x) => x !== udid) : [...prev, udid];
      }
      return prev.includes(udid) && prev.length === 1 ? [] : [udid];
    });
  };

  return { selected, setSelected, selectedDevices, onSelect, canvasRef, onCanvasMouseDown, band };
}
