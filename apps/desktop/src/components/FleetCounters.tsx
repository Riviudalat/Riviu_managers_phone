import type { DeviceOperationalFilter, FleetCounts } from "../deviceWork";

type CounterFilter = Exclude<DeviceOperationalFilter, "all" | "ready" | "connected">;

const COUNTERS: { key: keyof FleetCounts; filter: CounterFilter; label: string; tone: string; title: string }[] = [
  { key: "idle", filter: "idle", label: "Rảnh", tone: "ok", title: "Máy Sẵn sàng hoặc Đã kết nối, không có tác vụ nào giữ máy. Rảnh chưa phải là đã chứng minh điều khiển được." },
  { key: "busy", filter: "busy", label: "Đang chạy", tone: "info", title: "Máy Bận: một tác vụ đang giữ máy." },
  { key: "attention", filter: "warning", label: "Cần xử lý", tone: "danger", title: "Máy Cần xem: helper/Agent lỗi, máy quá tải hoặc chưa đọc được tác vụ." },
  { key: "offline", filter: "offline", label: "Mất kết nối", tone: "muted", title: "Máy Ngoại tuyến: không còn trả lời qua USB/Wi-Fi." },
];

const UNKNOWN_TITLE = "Chưa rõ: chưa đọc được tác vụ đang giữ máy hoặc bằng chứng helper Android (chỉ đọc trong Màn hình máy).";

/**
 * Header fleet counters. They are a view of the wall's status filter, not a second filter:
 * pressing one sets `deviceStatusFilter` (the same state as the "Trạng thái thiết bị" select)
 * and pressing it again clears it. Zero and unknown stay on screen so the row never jumps.
 */
export function FleetCounters({
  counts,
  filter,
  onFilter,
}: {
  counts: FleetCounts;
  filter: DeviceOperationalFilter;
  onFilter: (next: DeviceOperationalFilter) => void;
}) {
  return (
    <div className="fleet-counters" role="group" aria-label="Lọc máy theo trạng thái">
      {COUNTERS.map(counter => {
        const value = counts[counter.key];
        const active = filter === counter.filter;
        return (
          <button
            key={counter.key}
            type="button"
            className={`fleet-counter tone-${counter.tone}${active ? " active" : ""}`}
            data-counter={counter.key}
            aria-pressed={active}
            title={value === null ? `${counter.title} ${UNKNOWN_TITLE}` : counter.title}
            onClick={() => onFilter(active ? "all" : counter.filter)}
          >
            <span className="fleet-counter-dot" aria-hidden="true" />
            <span className="fleet-counter-label">{counter.label}</span>
            <strong className={value === null ? "unknown" : undefined}>{value === null ? "Chưa rõ" : value}</strong>
          </button>
        );
      })}
    </div>
  );
}
