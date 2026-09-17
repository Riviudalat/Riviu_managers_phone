import { ArrowRight, Cable, ClipboardList, Code2, FolderOpen, HelpCircle } from "lucide-react";
import { StatusNotice } from "../components/States";
import type { PageId } from "../types";

const TOPICS = [
  { page: "diagnostics", title: "Kết nối và kiểm tra máy", description: "Xem trạng thái kết nối, điều khiển và luồng hình trước khi chạy tác vụ.", label: "Kiểm tra thiết bị", Icon: Cable },
  { page: "myApps", title: "Thiết lập quy trình", description: "Mở Nuôi TikTok, Tương tác hoặc Đăng bài; chọn phạm vi riêng cho từng chức năng.", label: "Mở My Apps", Icon: ClipboardList },
  { page: "publish", title: "Ghép bài và thiết bị", description: "Chọn nguồn, ghép mỗi bài với một máy rồi kiểm tra. Chọn nhanh không tự đăng bài.", label: "Thiết lập Đăng bài", Icon: FolderOpen },
  { page: "jobs", title: "Đọc kết quả và xử lý phần còn thiếu", description: "Xem đúng lượt chạy và bằng chứng từng máy; phân biệt thành công, còn thiếu và chưa rõ.", label: "Xem lượt chạy", Icon: HelpCircle },
] satisfies { page: PageId; title: string; description: string; label: string; Icon: typeof Cable }[];

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
      <StatusNotice tone="info">Khi kết quả chưa rõ, giữ bằng chứng để đối chiếu. Không đăng lại bài hoặc gửi lại bình luận chỉ vì thiếu link hay báo cáo.</StatusNotice>
      <section className="operator-help-api">
        <div><h3>Dành cho tích hợp</h3><p>Tham chiếu lệnh và kiểm tra trạng thái API cục bộ; không đưa token vào đường dẫn chia sẻ.</p></div>
        <button type="button" onClick={() => onOpenPage("api")}><Code2 size={16} aria-hidden="true" />Tham chiếu API</button>
      </section>
    </section>
  );
}
