import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { CircleHelp } from "lucide-react";
import { autoUpdate, flip, hide, offset, shift, useFloating } from "@floating-ui/react-dom";

/**
 * The help icon after a control's name: what that control does, shown instantly on hover.
 *
 * Replaces paragraphs of hint text under each field — the information without the permanent
 * shelf space that made a settings form read as documentation. One icon per control, free
 * until pointed at. Shared so every panel's help reads and looks the same.
 *
 * The tooltip renders through a **portal to `document.body`**: a floating panel positions
 * itself with `transform`, which makes it the containing block for `position: fixed`, so a
 * tooltip left inside the panel is measured against the panel and clipped by its
 * `overflow: hidden` — it never appears. At body level it is measured against the viewport,
 * where the `getBoundingClientRect` coordinates are correct and nothing clips it.
 *
 * The trigger has its own accessible name so it does not become part of a surrounding label.
 * Hover, focus and click all reveal the same description; click pins it for touch users.
 */
export function InfoDot({ of, what }: { of: string; what: string }) {
  const [tip, setTip] = useState(false);
  const [pinned, setPinned] = useState(false);
  const tooltipId = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const { refs, floatingStyles, middlewareData } = useFloating({
    open: tip,
    placement: "top",
    strategy: "fixed",
    whileElementsMounted: autoUpdate,
    middleware: [offset(8), flip({ padding: 8 }), shift({ padding: 8 }), hide()],
  });

  useEffect(() => {
    if (!pinned) return;

    const dismiss = () => {
      setPinned(false);
      setTip(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") dismiss();
    };
    const onPointerDown = (event: PointerEvent) => {
      if (!triggerRef.current?.contains(event.target as Node)) dismiss();
    };

    document.addEventListener("keydown", onKeyDown);
    document.addEventListener("pointerdown", onPointerDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("pointerdown", onPointerDown);
    };
  }, [pinned]);

  return (
    <button
      ref={(element) => {
        triggerRef.current = element;
        refs.setReference(element);
      }}
      type="button"
      className="nu-info"
      aria-label={`Giải thích ${of}`}
      aria-describedby={tip ? tooltipId : undefined}
      data-info={of}
      data-tip={what}
      onMouseEnter={() => setTip(true)}
      onMouseLeave={(event) => {
        if (!pinned && document.activeElement !== event.currentTarget) setTip(false);
      }}
      onFocus={() => setTip(true)}
      onBlur={() => {
        if (!pinned) setTip(false);
      }}
      onClick={() => {
        setTip(!pinned);
        setPinned(!pinned);
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          setPinned(false);
          setTip(false);
        }
      }}
    >
      <CircleHelp size={14} strokeWidth={1.8} aria-hidden="true" />
      {tip &&
        createPortal(
          <span
            ref={refs.setFloating}
            id={tooltipId}
            className="nu-tip"
            role="tooltip"
            style={{ ...floatingStyles, visibility: middlewareData.hide?.referenceHidden ? "hidden" : undefined }}
          >
            {what}
          </span>,
          document.body,
        )}
    </button>
  );
}
