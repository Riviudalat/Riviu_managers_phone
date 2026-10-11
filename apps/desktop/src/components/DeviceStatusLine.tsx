import { Cable, Circle, CircleCheck, CircleDashed, CircleHelp, CirclePlay, CircleX, TriangleAlert, Unplug, type LucideIcon } from "lucide-react";
import { deviceStatusText, deviceStatusTitle } from "../deviceWork";
import type { DeviceOperationalView, DeviceStatusAxis } from "../deviceWork";
import { DEVICE_STATUS_ICONS } from "./deviceStatusIcons";
import "../styles/device-status.css";

/**
 * The condensed one-line status every surface shows for a phone (tile, list row, picker,
 * details drawer): icon + "Label: việc/lý do" + the state's token colour. The text truncates;
 * the tooltip carries the step and the full reason.
 */
export function DeviceStatusLine({ view, className }: { view: DeviceOperationalView; className?: string }) {
  const Icon = DEVICE_STATUS_ICONS[view.kind];
  return (
    <span className={`device-status is-${view.kind}${className ? ` ${className}` : ""}`} title={deviceStatusTitle(view)}>
      <Icon className="device-status-icon" size={13} strokeWidth={2.25} aria-hidden="true" />
      <span className="device-status-text">{deviceStatusText(view)}</span>
    </span>
  );
}

type AxisTone = "ok" | "neutral" | "info" | "warn" | "danger" | "muted" | "unchecked" | "unknown";

const AXIS_PRESENTATION: Record<string, { Icon: LucideIcon; tone: AxisTone }> = {
  "connection:connected": { Icon: Cable, tone: "neutral" },
  "connection:disconnected": { Icon: Unplug, tone: "muted" },
  "control:ready": { Icon: CircleCheck, tone: "ok" },
  "control:unchecked": { Icon: CircleDashed, tone: "unchecked" },
  "control:failed": { Icon: CircleX, tone: "danger" },
  "work:idle": { Icon: Circle, tone: "neutral" },
  "work:running": { Icon: CirclePlay, tone: "info" },
  "work:attention": { Icon: TriangleAlert, tone: "warn" },
};

function AxisRow({ name, axis, title }: { name: "connection" | "control" | "work"; axis: DeviceStatusAxis<string>; title: string }) {
  const { Icon, tone } = AXIS_PRESENTATION[`${name}:${axis.state}`] ?? { Icon: CircleHelp, tone: "unknown" as const };
  return (
    <div className="device-status-axis" data-axis={name}>
      <dt>{title}</dt>
      <dd>
        <span className={`device-status-axis-value tone-${tone}`}>
          <Icon size={14} strokeWidth={2.25} aria-hidden="true" />
          {deviceStatusText(axis)}
        </span>
        {axis.reason && <small>{axis.reason}</small>}
      </dd>
    </div>
  );
}

/** The three axes side by side, for the details drawer. */
export function DeviceStatusAxes({ view }: { view: DeviceOperationalView }) {
  return (
    <dl className="device-status-axes" aria-label="Ba trục trạng thái">
      <AxisRow name="connection" axis={view.axes.connection} title="Kết nối" />
      <AxisRow name="control" axis={view.axes.control} title="Điều khiển" />
      <AxisRow name="work" axis={view.axes.work} title="Công việc" />
    </dl>
  );
}

/**
 * The permanent explanation of the vocabulary (drawer and Trợ giúp), so its meaning never
 * lives only in a tooltip.
 */
export function DeviceStatusLegend() {
  return (
    <div className="device-status-legend">
      <p>Mỗi máy có ba trục, mỗi trục có <strong>Chưa rõ</strong> khi thiếu bằng chứng:</p>
      <ul>
        <li><strong>Kết nối</strong>: Đã kết nối · Mất kết nối · Chưa rõ. Máy còn trả lời qua USB/Wi-Fi hay không.</li>
        <li><strong>Điều khiển</strong>: Sẵn sàng · Chưa kiểm · Không đạt (kèm lý do) · Chưa rõ. Sẵn sàng chỉ khi có bằng chứng điều khiển còn hiệu lực cho kết nối hiện tại (Android: helper, phiên và xác thực; iPhone: WDA). Chưa kiểm là chưa có lần kiểm nào chứng minh, khác Không đạt.</li>
        <li><strong>Công việc</strong>: Rảnh · Đang chạy (kèm việc) · Cần xử lý (kèm lý do) · Chưa rõ. Máy đang chạy không nhận việc mới.</li>
      </ul>
      <p>Dòng trạng thái trên ô và bộ đếm gộp ba trục thành một từ, theo thứ tự: Mất kết nối → Đang chạy → Cần xử lý → Chưa rõ → Chưa kiểm → Sẵn sàng. Mỗi máy được đếm một lần, nên các số cộng lại bằng Tổng. Thiếu dữ liệu luôn là Chưa rõ, không bao giờ tự thành Sẵn sàng hay Mất kết nối.</p>
    </div>
  );
}
