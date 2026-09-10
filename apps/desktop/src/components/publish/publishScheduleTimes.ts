export function localDateTime(value: Date) {
  const part = (n: number) => String(n).padStart(2, "0");
  return `${value.getFullYear()}-${part(value.getMonth()+1)}-${part(value.getDate())}T${part(value.getHours())}:${part(value.getMinutes())}`;
}
export function distributeTimes(first: string, count: number, minutes: number): string[] {
  if (!/^\d{2}:\d{2}$/.test(first) || !Number.isInteger(minutes) || minutes < 1) return [];
  const [hour, minute] = first.split(":").map(Number);
  if (hour > 23 || minute > 59 || hour*60+minute+(count-1)*minutes >= 1440) return [];
  return Array.from({length: count}, (_, i) => {
    const total = hour*60+minute+i*minutes;
    return `${String(Math.floor(total/60)).padStart(2,"0")}:${String(total%60).padStart(2,"0")}`;
  });
}

/** Reject calendar normalization (e.g. Feb 30) and DST gaps before calling native preflight. */
export function scheduleDateIssue(date: string, time: string, now: number): string {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date) || !/^\d{2}:\d{2}$/.test(time)) return "Chọn ngày và giờ đăng";
  const parsed = new Date(`${date}T${time}`);
  if (!Number.isFinite(parsed.getTime()) || localDateTime(parsed) !== `${date}T${time}`) return "Ngày hoặc giờ đăng không hợp lệ";
  return parsed.getTime() <= now ? "Giờ đăng phải ở tương lai" : "";
}
