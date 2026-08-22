import type { NurtureSettings } from "../../types";
import { InfoDot as Info } from "../InfoDot";

/** When sessions start by themselves, and on which phones. */
export function NurtureScheduleTab({ settings, patch }: {
  settings: NurtureSettings;
  patch: <K extends keyof NurtureSettings>(key: K, value: NurtureSettings[K]) => void;
}) {
  return (
    <div className="nurture-sect nurture-sched" role="tabpanel">
      <label className="check">
        <input
          type="checkbox"
          checked={settings.scheduleEnabled}
          onChange={(e) => patch("scheduleEnabled", e.target.checked)}
        />
        <span className="nu-inline">
          Lịch tự chạy
          <Info
            of="Lịch tự chạy"
            what="Tự khởi động phiên theo chu kỳ, không cần bấm Bắt đầu. Chỉ chạy trên những máy đã chọn khi lưu."
          />
        </span>
      </label>
      <div className="nurture-row">
        <label>
          <span className="nu-inline">
            Mỗi (phút)
            <Info of="Mỗi (phút)" what="Khoảng cách giữa hai lần tự khởi động." />
          </span>
          <input
            type="number"
            min={15}
            max={1440}
            value={settings.scheduleEveryMinutes}
            onChange={(e) => patch("scheduleEveryMinutes", Number(e.target.value) || 240)}
          />
        </label>
        <label>
          <span className="nu-inline">
            Thời lượng (phút)
            <Info
              of="Thời lượng (phút)"
              what="Phiên theo lịch chạy tối đa bấy nhiêu phút. Phiên bấm tay không dùng số này — nó được gán một trần 2–3 giờ ngẫu nhiên, nên hai máy bấm cùng lúc không dừng cùng lúc."
            />
          </span>
          <input
            type="number"
            min={15}
            max={360}
            value={settings.scheduleDurationMinutes}
            onChange={(e) => patch("scheduleDurationMinutes", Number(e.target.value) || 150)}
          />
        </label>
      </div>
    </div>
  );
}
