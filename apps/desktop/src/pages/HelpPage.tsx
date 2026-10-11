import { ArrowRight, Cable, ClipboardList, Code2, FolderOpen, HelpCircle } from "lucide-react";
import { StatusNotice } from "../components/States";
import { DeviceStatusLegend } from "../components/DeviceStatusLine";
import { PostOutcomeLegend } from "../features/operations/PostOutcome";
import type { PageId } from "../types";

const TOPICS = [
  { page: "diagnostics", title: "Kết nối và kiểm tra máy", description: "Xem trạng thái kết nối, điều khiển và luồng hình trước khi chạy tác vụ.", label: "Kiểm tra thiết bị", Icon: Cable },
  { page: "myApps", title: "Thiết lập quy trình", description: "Mở Nuôi TikTok, Tương tác hoặc Đăng bài; chọn phạm vi riêng cho từng chức năng.", label: "Mở Quy trình ứng dụng", Icon: ClipboardList },
  { page: "publish", title: "Ghép bài và thiết bị", description: "Chọn nguồn, ghép mỗi bài với một máy rồi kiểm tra. Chọn nhanh không tự đăng bài.", label: "Thiết lập Đăng bài", Icon: FolderOpen },
  { page: "jobs", title: "Đọc kết quả và xử lý phần còn thiếu", description: "Xem đúng lượt chạy và bằng chứng từng máy; phân biệt thành công, còn thiếu và chưa rõ.", label: "Xem lượt chạy", Icon: HelpCircle },
] satisfies { page: PageId; title: string; description: string; label: string; Icon: typeof Cable }[];

/** Where a GenFarmer habit lands in Riviu, with the limits stated rather than implied. */
const FROM_GENFARMER = [
  { from: "Control Center", to: "Trung tâm điều khiển", note: "Lưới máy, chọn máy, mở màn lớn. Lối tắt Trung tâm ở đầu thanh bên." },
  { from: "My Apps", to: "Ứng dụng › Quy trình ứng dụng", note: "Nuôi TikTok, Tương tác, Đăng bài và quy trình đã lưu. Cài APK nằm ở thẻ Gói cài đặt bên cạnh." },
  { from: "Automation", to: "Nhóm Công việc", note: "Nuôi TikTok, Tương tác, Đăng bài, Lượt chạy, Lịch chạy, Tác vụ đã lưu, Flow thiết bị." },
  { from: "Runs", to: "Lượt chạy", note: "Kết quả và bằng chứng từng máy. Lượt Chưa rõ chỉ theo dõi, không chạy lại." },
  { from: "Tasks", to: "Tác vụ đã lưu", note: "Chạy luôn hỏi xác nhận trước khi gửi lệnh." },
  { from: "Schedules", to: "Lịch chạy", note: "Lịch giữ danh sách máy đã duyệt; nhóm đổi thì phải Duyệt lại, lịch không tự mở rộng phạm vi." },
  { from: "Inspector", to: "Bắt thuộc tính (nút Inspector trên thanh hệ thống)", note: "Chỉ máy Android. iPhone chưa có Inspector." },
  { from: "Groups", to: "Box/Nhóm (nút Nhóm trong Trung tâm điều khiển)", note: "Nhóm là danh sách máy được gán, không suy từ số máy." },
  { from: "Settings / stream", to: "Nút Hiển thị cạnh lưới", note: "Cỡ ô, cỡ cửa sổ điều khiển, chất lượng hình và FPS. Chất lượng và FPS chỉ áp dụng cho máy Android." },
];

export function HelpPage({ onOpenPage }: { onOpenPage: (page: PageId) => void }) {
  return (
    <section className="operator-help" aria-label="Hướng dẫn vận hành">
      <header className="operator-help-heading">
        <h2>Bạn cần làm gì?</h2>
        <p>Đi tới đúng nơi để chuẩn bị thiết bị, thiết lập công việc hoặc xem kết quả.</p>
      </header>
      <div className="operator-help-topics">
        {TOPICS.map(({ page, title, description, label, Icon }) => (
          <section key={page} className="operator-help-topic">
            <Icon size={20} aria-hidden="true" />
            <div><h3>{title}</h3><p>{description}</p></div>
            <button type="button" onClick={() => onOpenPage(page)}>{label}<ArrowRight size={15} aria-hidden="true" /></button>
          </section>
        ))}
      </div>
      <section className="operator-help-vocabulary" aria-labelledby="help-device-status">
        <h3 id="help-device-status">Đọc trạng thái máy</h3>
        <DeviceStatusLegend />
      </section>
      <section className="operator-help-vocabulary" aria-labelledby="help-post-outcome">
        <h3 id="help-post-outcome">Đọc kết quả bài đăng</h3>
        <PostOutcomeLegend />
      </section>
      <StatusNotice tone="info">Khi kết quả chưa rõ, giữ bằng chứng để đối chiếu. Không đăng lại bài hoặc gửi lại bình luận chỉ vì thiếu link hay báo cáo.</StatusNotice>
      <section className="operator-help-api">
        <div><h3>Dành cho tích hợp</h3><p>Tham chiếu lệnh và kiểm tra trạng thái API cục bộ; không đưa token vào đường dẫn chia sẻ.</p></div>
        <button type="button" onClick={() => onOpenPage("api")}><Code2 size={16} aria-hidden="true" />Tham chiếu API</button>
      </section>
      <section className="operator-help-migration" aria-labelledby="help-from-genfarmer">
        <h3 id="help-from-genfarmer">Từ GenFarmer sang Riviu</h3>
        <p>Gõ tên cũ vào Ctrl+K (ví dụ Runs, My Apps, Inspector) để tới đúng chỗ.</p>
        <table>
          <thead><tr><th scope="col">GenFarmer</th><th scope="col">Riviu</th><th scope="col">Ghi chú</th></tr></thead>
          <tbody>
            {FROM_GENFARMER.map(({ from, to, note }) => (
              <tr key={from}><th scope="row">{from}</th><td>{to}</td><td>{note}</td></tr>
            ))}
          </tbody>
        </table>
      </section>
    </section>
  );
}
