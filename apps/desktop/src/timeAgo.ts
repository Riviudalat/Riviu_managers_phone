/**
 * How long ago something happened, in Vietnamese.
 *
 * The interaction campaign list had no time on it at all — `updatedAt` was on every summary
 * and rendered nowhere — so seven runs sorted newest-first looked like seven runs in no
 * order, and "the one I just launched" was a guess.
 *
 * Absolute past a day: "3 ngày trước" stops being useful for finding a specific run, and by
 * then the clock time is what an operator remembers.
 *
 * **`getHours()` on a `+00:00` timestamp is deliberate.** `db/interaction.rs` writes
 * `Utc::now().to_rfc3339()`, `new Date(iso)` parses that as UTC, and `getHours()` converts to
 * the operator's local wall clock — which is exactly what "the clock time an operator
 * remembers" means. Anyone "fixing" this to `getUTCHours()` would break it by seven hours in
 * Vietnam, so `timeAgo.test.ts` now asserts the conversion with an offset-bearing fixture
 * instead of the offset-less local strings it used to use, which asserted nothing about it.
 */
export function timeAgoVi(iso: string | null | undefined, now: Date = new Date()): string {
  if (!iso) return "";
  const then = new Date(iso);
  const at = then.getTime();
  if (Number.isNaN(at)) return "";

  const seconds = Math.round((now.getTime() - at) / 1000);
  // A clock skew between the database's timestamp and this machine's is not the future.
  if (seconds < 45) return "vừa xong";
  // Floored, not rounded. Rounding said "2 giờ trước" ninety minutes in — reading the clock
  // forward is worse than reading it short, because the operator is looking for a run they
  // remember starting and an overstated age sends them past it.
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} phút trước`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} giờ trước`;

  const pad = (value: number) => String(value).padStart(2, "0");
  const stamp = `${pad(then.getHours())}:${pad(then.getMinutes())} ${pad(then.getDate())}/${pad(
    then.getMonth() + 1,
  )}`;
  // The year, once it stops being this one. Without it a run from 2025-08-21 and one from
  // 2026-08-21 render identically, which is the one thing an absolute stamp exists to prevent.
  return then.getFullYear() === now.getFullYear()
    ? stamp
    : `${stamp}/${then.getFullYear()}`;
}
