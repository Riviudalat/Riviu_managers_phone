import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type MouseEvent, type PointerEvent } from "react";

type Position = { left: number; top: number };
export function clampMonitorPosition(position: Position, width: number, height: number, viewportWidth: number, viewportHeight: number): Position {
  return { left: Math.max(8, Math.min(position.left, viewportWidth - width - 8)),
    top: Math.max(8, Math.min(position.top, viewportHeight - height - 8)) };
}

export function useFloatingMonitor(expanded: boolean, maximized = false) {
  const ref = useRef<HTMLElement>(null);
  const drag = useRef<{ pointer: number; x: number; y: number; origin: Position; moved: boolean; target: HTMLElement } | null>(null);
  const suppressClick = useRef(false);
  const [position, setPosition] = useState<Position | null>(null);
  const finishDrag = useCallback((pointer?: number) => {
    const held = drag.current;
    if (!held || (pointer !== undefined && held.pointer !== pointer)) return;
    drag.current = null;
    if (held.target.hasPointerCapture?.(held.pointer)) held.target.releasePointerCapture(held.pointer);
  }, []);
  useEffect(() => {
    // A release can happen outside the header before the drag threshold captures it.
    const release = (event: globalThis.PointerEvent) => finishDrag(event.pointerId);
    const blur = () => finishDrag();
    window.addEventListener("pointerup", release, true);
    window.addEventListener("pointercancel", release, true);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("pointerup", release, true);
      window.removeEventListener("pointercancel", release, true);
      window.removeEventListener("blur", blur);
      finishDrag();
    };
  }, [finishDrag]);
  const constrain = (point: Position) => {
    const rect = ref.current?.getBoundingClientRect();
    return rect ? clampMonitorPosition(point, rect.width, rect.height, window.innerWidth, window.innerHeight) : point;
  };
  useLayoutEffect(() => {
    finishDrag();
    if (maximized) return;
    const resize = () => setPosition((current) => current ? constrain(current) : null);
    resize();
    window.addEventListener("resize", resize);
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(resize);
    if (ref.current) observer?.observe(ref.current);
    return () => { window.removeEventListener("resize", resize); observer?.disconnect(); };
  }, [expanded, maximized, finishDrag]);
  const excluded = (target: EventTarget) => target instanceof Element && target.closest("[data-monitor-no-drag]") !== null;
  const onPointerDown = (event: PointerEvent<HTMLElement>) => {
    if (event.button !== 0 || event.isPrimary === false || drag.current) return;
    suppressClick.current = false;
    if (maximized || excluded(event.target)) return;
    const rect = ref.current?.getBoundingClientRect();
    if (!rect) return;
    drag.current = { pointer: event.pointerId, x: event.clientX, y: event.clientY, origin: { left: rect.left, top: rect.top }, moved: false, target: event.currentTarget };
  };
  const onPointerMove = (event: PointerEvent<HTMLElement>) => {
    const held = drag.current;
    if (maximized || !held || held.pointer !== event.pointerId) return;
    if (!held.moved) {
      if (Math.hypot(event.clientX - held.x, event.clientY - held.y) < 4) return;
      held.moved = true;
      // Capture only once dragging starts; capturing the header on pointerdown would
      // steal an ordinary click from the title button nested inside it.
      event.currentTarget.setPointerCapture(event.pointerId);
    }
    suppressClick.current = true;
    setPosition(constrain({ left: held.origin.left + event.clientX - held.x, top: held.origin.top + event.clientY - held.y }));
  };
  const finishPointer = (event: PointerEvent<HTMLElement>) => {
    finishDrag(event.pointerId);
  };
  const onClickCapture = (event: MouseEvent<HTMLElement>) => {
    if (!suppressClick.current) return;
    suppressClick.current = false;
    if (event.detail === 0) return;
    event.preventDefault();
    event.stopPropagation();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (maximized || excluded(event.target)) return;
    const directions: Record<string, Position> = { ArrowLeft: { left: -24, top: 0 }, ArrowRight: { left: 24, top: 0 },
      ArrowUp: { left: 0, top: -24 }, ArrowDown: { left: 0, top: 24 } };
    const delta = directions[event.key];
    const rect = ref.current?.getBoundingClientRect();
    if (!delta || !rect) return;
    event.preventDefault();
    setPosition(constrain({ left: rect.left + delta.left, top: rect.top + delta.top }));
  };
  return { ref, style: !maximized && position ? { ...position, right: "auto", bottom: "auto" } : undefined,
    handle: { onPointerDown, onPointerMove, onPointerUp: finishPointer,
      onPointerCancel: finishPointer, onLostPointerCapture: finishPointer, onClickCapture, onKeyDown } };
}
