# Kho lịch sử

Các file ở đây ghi thiết kế, kế hoạch hoặc số đo tại ngày của chúng. Chúng không được
cập nhật để mô tả trạng thái sản phẩm hiện tại. Đọc [hướng dẫn hiện tại](../README.md)
và [nhật ký agent](../agents/README.md) trước khi dùng một kết luận để sửa code.

| Nhóm | Nội dung |
|---|---|
| `reports/` | báo cáo đo/đối chiếu có ngày, chuyển từ gốc docs |
| `plans/` | kế hoạch 28–31/07/2026, giữ nguyên lý do và checkpoint |
| `specs/` | thiết kế có ngày; không tự có nghĩa đã triển khai đủ |
| [Dọn tài liệu 06/09/2026](cleanup-2026-09-06.md) | phạm vi, kiểm chứng và danh sách xoá |
| [Bản đồ đường dẫn](path-map.json) | old/new mapping cho link và patch lịch sử |

`docs/re/`, `docs/verification/` và `docs/fixtures/` giữ vị trí vì có consumer riêng.
Patch/diff/JSON chứng cứ giữ nguyên byte, kể cả khi nội dung nhắc đường dẫn cũ; tra map
trước khi diễn giải. Không chạy rollback lịch sử lên cây hiện tại mà không review.
