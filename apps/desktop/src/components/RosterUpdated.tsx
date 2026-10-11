const twoDigits = (value: number) => String(value).padStart(2, "0");

/**
 * "Cập nhật hh:mm:ss" in the system bar: when the device list was last read from the backend.
 * It is the age of the roster, not of any phone's picture or status check, so it never stands in
 * for evidence about one phone. Before the first read it says "Chưa rõ", never a made-up time.
 */
export function RosterUpdated({ at }: { at: number | null }) {
  if (at === null) {
    return (
      <span className="roster-updated" title="Chưa đọc được danh sách máy lần nào">
        Cập nhật: Chưa rõ
      </span>
    );
  }
  const time = new Date(at);
  const clock = `${twoDigits(time.getHours())}:${twoDigits(time.getMinutes())}:${twoDigits(time.getSeconds())}`;
  return (
    <span className="roster-updated" title="Lần đọc danh sách máy gần nhất (giờ máy tính này)">
      Cập nhật <time dateTime={time.toISOString()}>{clock}</time>
    </span>
  );
}
