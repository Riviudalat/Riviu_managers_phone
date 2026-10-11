import { Smartphone, type LucideIcon } from "lucide-react";
import { DEVICE_STATUS_LABELS } from "../deviceWork";
import type { DeviceOperationalFilter, DeviceOperationalStatus, FleetCounts } from "../deviceWork";
import { DEVICE_STATUS_ICONS } from "./deviceStatusIcons";
import "../styles/device-status.css";

type CounterKey = Exclude<keyof FleetCounts, "total">;

const COUNTERS: { key: CounterKey; filter: DeviceOperationalStatus; title: string }[] = [
  { key: "ready", filter: "ready", title: "Máy rảnh có bằng chứng điều khiển còn hiệu lực cho kết nối hiện tại." },
  { key: "unchecked", filter: "unchecked", title: "Máy rảnh, đã kết nối nhưng chưa có lần kiểm nào chứng minh điều khiển được. Chưa kiểm không phải Không đạt." },
  { key: "busy", filter: "busy", title: "Một tác vụ đang giữ máy; máy không nhận việc mới." },
  { key: "attention", filter: "warning", title: "Máy cần người vận hành: điều khiển Không đạt, helper cần khôi phục, máy báo lỗi hoặc quá tải." },
  { key: "offline", filter: "offline", title: "Máy không còn trả lời qua USB/Wi-Fi." },
  { key: "unknown", filter: "unknown", title: "Thiếu bằng chứng: chưa đọc được tác vụ đang giữ máy hoặc bằng chứng điều khiển (chỉ đọc trong Trung tâm điều khiển). Không coi là Sẵn sàng." },
];

const AXES_NOTE = "Ba trục Kết nối, Điều khiển và Công việc chồng lên nhau; mỗi máy chỉ được đếm một lần theo dòng trạng thái gộp, nên các số cộng lại bằng Tổng.";
const UNKNOWN_VALUE = "Chưa rõ: một số máy chưa rõ có thể thuộc nhóm này.";

/**
 * Header fleet counters. They are a view of the wall's status filter, not a second filter:
 * pressing one sets `deviceStatusFilter` (the same state as the "Trạng thái thiết bị" select),
 * pressing it again or pressing Tổng clears it. Zero and unknown stay on screen so the row never
 * jumps; every count says its denominator.
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
  const total = counts.total;
  const totalText = total === null ? "?" : String(total);
  return (
    <div className="fleet-counters" role="group" aria-label="Lọc máy theo trạng thái">
      <button
        type="button"
        className="fleet-counter tone-total"
        data-counter="total"
        aria-pressed={filter === "all"}
        aria-label={`Tổng: ${total === null ? "chưa rõ" : `${total} máy`}`}
        title={`Mọi máy trong danh sách. ${AXES_NOTE}`}
        onClick={() => onFilter("all")}
      >
        <Smartphone className="fleet-counter-icon" size={13} aria-hidden="true" />
        <span className="fleet-counter-label">Tổng</span>
        <strong className={total === null ? "unknown" : undefined}>{totalText}</strong>
      </button>
      {COUNTERS.map(counter => {
        const value = counts[counter.key];
        const active = filter === counter.filter;
        const label = DEVICE_STATUS_LABELS[counter.filter];
        const Icon: LucideIcon = DEVICE_STATUS_ICONS[counter.filter];
        return (
          <button
            key={counter.key}
            type="button"
            className={`fleet-counter tone-${counter.filter}${active ? " active" : ""}`}
            data-counter={counter.key}
            aria-pressed={active}
            aria-label={`${label}: ${value === null ? "chưa rõ" : `${value} trên ${totalText} máy`}`}
            title={[counter.title, value === null ? UNKNOWN_VALUE : undefined, AXES_NOTE].filter(Boolean).join(" ")}
            onClick={() => onFilter(active ? "all" : counter.filter)}
          >
            <Icon className="fleet-counter-icon" size={13} aria-hidden="true" />
            <span className="fleet-counter-label">{label}</span>
            <span className="fleet-counter-value">
              <strong className={value === null ? "unknown" : undefined}>{value === null ? "?" : value}</strong>
              <span className="fleet-counter-of">/{totalText}</span>
            </span>
          </button>
        );
      })}
    </div>
  );
}
