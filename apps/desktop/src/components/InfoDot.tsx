import { useState } from "react";
import { createPortal } from "react-dom";

/**
 * The `!` glyph after a control's name: what that control does, shown instantly on hover.
 *
 * Replaces paragraphs of hint text under each field — the information without the permanent
 * shelf space that made a settings form read as documentation. One glyph per control, free
 * until pointed at. Shared so every panel's help reads and looks the same.
 *
 * The tooltip renders through a **portal to `document.body`**: a floating panel positions
 * itself with `transform`, which makes it the containing block for `position: fixed`, so a
 * tooltip left inside the panel is measured against the panel and clipped by its
 * `overflow: hidden` — it never appears. At body level it is measured against the viewport,
 * where the `getBoundingClientRect` coordinates are correct and nothing clips it.
 *
 * `aria-hidden`: a `<label>`'s accessible name is its whole text content, so a glyph visible
 * to the accessibility tree would rename every field it sits beside. `data-info` lets a test
 * find a specific one; `data-tip` exposes the text without needing a hover to read it.
 */
export function InfoDot({ of, what }: { of: string; what: string }) {
  const [tip, setTip] = useState<{ left: number; top: number } | null>(null);
  return (
    <span
      className="nu-info"
      aria-hidden="true"
      data-info={of}
      data-tip={what}
      onMouseEnter={(event) => {
        const rect = event.currentTarget.getBoundingClientRect();
        setTip({ left: Math.round(rect.left + rect.width / 2), top: Math.round(rect.top) });
      }}
      onMouseLeave={() => setTip(null)}
    >
      !
      {tip &&
        createPortal(
          <span className="nu-tip" role="tooltip" style={{ left: tip.left, top: tip.top }}>
            {what}
          </span>,
          document.body,
        )}
    </span>
  );
}
