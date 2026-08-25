/**
 * The float panels' progress bar.
 *
 * It began life inside `nurture/NurtureProgress.tsx` because nurture was the first thing in
 * this app with a bounded run to draw. Interaction has one too — a campaign is N messages
 * over M links — and copying a `role="progressbar"` is how two bars end up disagreeing about
 * what an unknown fraction looks like. Both panels import it from here — the re-export left
 * behind in `NurtureProgress` for "every existing importer" turned out to have none.
 *
 * The `nu-` class prefix stays: these are the float-panel styles both popups share, the same
 * way both use `nu-field` and `nu-switch`.
 */
export type BarProps = {
  /** `0..1`, or `null` for "not known yet" — which draws a hatched track, not an empty one. */
  fraction: number | null;
  /** Share of the track that ended badly, drawn as a red tail. */
  failedFraction?: number;
  /** Colours the fill: running, finished cleanly, or failed. */
  tone?: "run" | "done" | "failed" | "idle";
  /** Accessible name; the visible label sits outside the bar. */
  label: string;
  className?: string;
};

/**
 * The bar itself.
 *
 * `role="progressbar"` with real `aria-value*` attributes rather than a styled div: this is a
 * measurement, and a screen reader that reads "63%" is reading the same thing the operator
 * sees. An unknown fraction sets `aria-valuetext` instead of a number, because
 * `aria-valuenow="0"` would claim a measurement nobody has.
 */
export function ProgressBar({
  fraction,
  failedFraction = 0,
  tone = "run",
  label,
  className,
}: BarProps) {
  const known = fraction !== null;
  const pct = Math.round((fraction ?? 0) * 100);
  const failedPct = Math.round(failedFraction * 100);
  return (
    <div
      className={`nu-bar tone-${tone}${known ? "" : " is-unknown"}${className ? ` ${className}` : ""}`}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      {...(known ? { "aria-valuenow": pct } : { "aria-valuetext": "chưa rõ" })}
      // Fed inline, so every `var()` reading these carries a fallback — a stylesheet cannot
      // see a style attribute, and the rule still has to render without one.
      style={{ "--fill": `${pct}%`, "--failed": `${failedPct}%` } as React.CSSProperties}
    >
      <span className="nu-bar-fill" />
      {failedPct > 0 && <span className="nu-bar-failed" />}
    </div>
  );
}
