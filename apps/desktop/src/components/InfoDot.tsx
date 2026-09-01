import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { CircleHelp } from "lucide-react";

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
  const [tip, setTip] = useState<{ left: number; top: number } | null>(null);
  const [pinned, setPinned] = useState(false);
  const tooltipId = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!pinned) return;

    const dismiss = () => {
      setPinned(false);
      setTip(null);
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

  const open = (element: HTMLElement) => {
    const rect = element.getBoundingClientRect();
    setTip({ left: Math.round(rect.left + rect.width / 2), top: Math.round(rect.top) });
  };

  return (
    <button
      ref={triggerRef}
      type="button"
      className="nu-info"
      aria-label={`Giải thích ${of}`}
      aria-describedby={tip ? tooltipId : undefined}
      data-info={of}
      data-tip={what}
      onMouseEnter={(event) => open(event.currentTarget)}
      onMouseLeave={(event) => {
        if (!pinned && document.activeElement !== event.currentTarget) setTip(null);
      }}
      onFocus={(event) => open(event.currentTarget)}
      onBlur={() => {
        if (!pinned) setTip(null);
      }}
      onClick={(event) => {
        const element = event.currentTarget;
        setPinned((current) => {
          if (current) setTip(null);
          else open(element);
          return !current;
        });
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          setPinned(false);
          setTip(null);
        }
      }}
    >
      <CircleHelp size={14} strokeWidth={1.8} aria-hidden="true" />
      {tip &&
        createPortal(
          <span
            id={tooltipId}
            className="nu-tip"
            role="tooltip"
            style={{ left: tip.left, top: tip.top }}
          >
            {what}
          </span>,
          document.body,
        )}
    </button>
  );
}
