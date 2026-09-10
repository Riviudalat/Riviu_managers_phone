import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

export type ScheduleDropTarget = { machine?: string };
type DragView = { ids: string[]; originId: string; x: number; y: number; target: ScheduleDropTarget | null };

/** Pointer capture keeps fast successive drops local; all assignment decisions use the latest owner state. */
export function useScheduleDrag({ disabled, getIds, onDrop }: {
  disabled: boolean; getIds: (id: string) => string[]; onDrop: (ids: string[], target: ScheduleDropTarget) => void;
}) {
  const root = useRef<HTMLElement>(null);
  const current = useRef({ disabled, getIds, onDrop });
  current.current = { disabled, getIds, onDrop };
  const drag = useRef<{ pointer: number; startX: number; startY: number; x: number; y: number; active: boolean; ids: string[]; originId: string } | null>(null);
  const frame = useRef<number | null>(null);
  const [view, setView] = useState<DragView | null>(null);
  const suppressClick = useRef(false);
  const hit = (x: number, y: number): ScheduleDropTarget | null => {
    const node = document.elementFromPoint(x, y);
    if (!node || !root.current?.contains(node)) return null;
    const machine = node.closest<HTMLElement>("[data-schedule-device]");
    if (machine) return { machine: machine.dataset.scheduleDevice };
    return node.closest("[data-schedule-drop]") ? {} : null;
  };
  const cancel = () => {
    const d = drag.current;
    drag.current = null;
    if (frame.current !== null) cancelAnimationFrame(frame.current);
    frame.current = null;
    if (d && root.current?.hasPointerCapture?.(d.pointer)) root.current.releasePointerCapture(d.pointer);
    setView(null);
  };
  useEffect(() => {
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape" && drag.current) { event.preventDefault(); cancel(); } };
    window.addEventListener("keydown", escape);
    window.addEventListener("blur", cancel);
    return () => { window.removeEventListener("keydown", escape); window.removeEventListener("blur", cancel); cancel(); };
  }, []);
  useEffect(() => { if (disabled) cancel(); }, [disabled]);
  const tick = () => {
    const d = drag.current;
    if (!d?.active) return;
    const node = document.elementFromPoint(d.x, d.y);
    const scroll = node?.closest<HTMLElement>("[data-schedule-scroll]");
    if (scroll && root.current?.contains(scroll)) {
      const box = scroll.getBoundingClientRect();
      const step = d.y < box.top + 35 ? -12 : d.y > box.bottom - 35 ? 12 : 0;
      if (step) scroll.scrollTop += step;
    }
    const target = hit(d.x, d.y);
    setView({ ids: target?.machine ? [d.originId] : d.ids, originId: d.originId, x: d.x, y: d.y, target });
    frame.current = requestAnimationFrame(tick);
  };
  return { root, view, cancel, bindings: {
    onPointerDown(event: ReactPointerEvent<HTMLElement>) {
      if (current.current.disabled || event.button !== 0) return;
      const source = (event.target as HTMLElement).closest<HTMLElement>("[data-schedule-drag]");
      if (!source) return;
      cancel(); suppressClick.current = false;
      drag.current = { pointer: event.pointerId, startX: event.clientX, startY: event.clientY, x: event.clientX, y: event.clientY, active: false, ids: current.current.getIds(source.dataset.scheduleDrag!), originId: source.dataset.scheduleDrag! };
    },
    onPointerMove(event: ReactPointerEvent<HTMLElement>) {
      const d = drag.current;
      if (!d || d.pointer !== event.pointerId || current.current.disabled) return;
      d.x = event.clientX; d.y = event.clientY;
      if (!d.active && Math.hypot(d.x - d.startX, d.y - d.startY) < 7) return;
      event.preventDefault();
      if (!d.active) { d.active = true; root.current?.setPointerCapture(event.pointerId); tick(); }
    },
    onPointerUp(event: ReactPointerEvent<HTMLElement>) {
      const d = drag.current;
      if (!d || d.pointer !== event.pointerId) return;
      const target = d.active ? hit(event.clientX, event.clientY) : null;
      const ids = target?.machine ? [d.originId] : d.ids;
      suppressClick.current = d.active;
      cancel();
      if (target && !current.current.disabled) current.current.onDrop(ids, target);
    },
    onPointerCancel: cancel,
    onLostPointerCapture: cancel,
    onClickCapture(event: React.MouseEvent<HTMLElement>) { if (suppressClick.current) { suppressClick.current = false; event.preventDefault(); event.stopPropagation(); } },
  } };
}
