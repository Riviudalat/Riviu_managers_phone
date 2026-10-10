import type { ReactNode } from "react";

/** One reason a primary action is disabled, and optionally the way to the field that fixes it. */
export interface Blocker {
  message: ReactNode;
  fixLabel?: string;
  onFix?: () => void;
}

/** The `aria-describedby` value for a CTA blocked by `blockers` rendered under `id`. */
export function blockerDescribedBy(id: string, blockers: readonly Blocker[]): string | undefined {
  return blockers.length ? blockers.map((_, index) => `${id}-${index}`).join(" ") : undefined;
}

/** Moves focus to the first visible match, for a blocker's fix link. Returns whether it found one. */
export function focusBlockingField(root: ParentNode | null | undefined, selector: string): boolean {
  const target = Array.from(root?.querySelectorAll<HTMLElement>(selector) ?? []).find(
    (element) => !element.closest("[hidden]"),
  );
  if (!target) return false;
  target.scrollIntoView?.({ block: "nearest" });
  target.focus();
  return true;
}
