import { useCallback, useEffect, useRef, useState } from "react";

/** Retain a dialog through its exit motion, then release focus and native resources. */
export function useClosingTransition(onClosed: () => void, duration = 180, identity?: string) {
  const [closing, setClosing] = useState(false);
  const callback = useRef(onClosed);
  callback.current = onClosed;
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => { if (timer.current) clearTimeout(timer.current); }, []);
  useEffect(() => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    setClosing(false);
  }, [identity]);
  const close = useCallback(() => {
    if (timer.current) return;
    if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) { callback.current(); return; }
    setClosing(true);
    timer.current = setTimeout(() => callback.current(), duration);
  }, [duration]);
  return { closing, close };
}
