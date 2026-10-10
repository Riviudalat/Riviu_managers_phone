import { useEffect, useRef } from "react";

/**
 * Ctrl+K (⌘K on macOS) toggles the palette from anywhere in the window — except while
 * another modal dialog (a confirmation, say) owns the screen, which a palette stacked on top
 * would only confuse. The phone keyboard bridge never forwards Ctrl+K, so it reaches here.
 */
export function usePaletteShortcut(toggle: () => void) {
  const latest = useRef(toggle);
  useEffect(() => {
    latest.current = toggle;
  });
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.altKey || event.shiftKey) return;
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "k") return;
      const modal = document.querySelector('[aria-modal="true"]');
      if (modal && !modal.classList.contains("ui-palette")) return;
      event.preventDefault();
      latest.current();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
