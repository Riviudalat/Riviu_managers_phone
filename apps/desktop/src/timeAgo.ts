/**
 * How long ago something happened, in Vietnamese.
 *
 * The interaction campaign list had no time on it at all — `updatedAt` was on every summary
 * and rendered nowhere — so seven runs sorted newest-first looked like seven runs in no
 * order, and "the one I just launched" was a guess.
 *
 * Absolute past a day: "3 ngày trước" stops being useful for finding a specific run, and by
 * then the clock time is what an operator remembers.
 */
export function timeAgoVi(iso: string | null | undefined, now: Date = new Date()): string {
  if (!iso) return "";
  const then = new Date(iso);
  const at = then.getTime();
  if (Number.isNaN(at)) return "";

  const seconds = Math.round((now.getTime() - at) / 1000);
  // A clock skew between the database's timestamp and this machine's is not the future.
  if (seconds < 45) return "vừa xong";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} phút trước`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} giờ trước`;

  const pad = (value: number) => String(value).padStart(2, "0");
  return `${pad(then.getHours())}:${pad(then.getMinutes())} ${pad(then.getDate())}/${pad(
    then.getMonth() + 1,
  )}`;
}
