---
name: systematic-debugging
description: Investigate Riviu failures using reproducible evidence, boundary isolation and one hypothesis at a time before changing code. Includes no automatic device actions, installation or agent fan-out.
metadata:
  origin: adapted-upstream
---

# Điều tra lỗi có hệ thống

Bản prose chọn lọc/adapt từ Superpowers (MIT). [Provenance](UPSTREAM.json),
[license](LICENSE). Không cài plugin/hooks, không yêu cầu tool ngoài hoặc spawning
agent tự động. Contract quyền/thiết bị của Riviu luôn đứng trước ví dụ debugging.

## 1. Xác định lỗi trước sửa

- Ghi expected/actual, phiên bản source và executable, thời điểm và ID của lần lỗi.
- Đọc lỗi đầy đủ, phân biệt provider/transport/session/ownership/perception/input/
  verifier/storage. ACK khác success; không có dữ liệu khác dữ liệu rỗng.
- Tái hiện bằng fixture/fake boundary hoặc trace replay trước. Không tái hiện bằng
  Post/Comment/Like/Follow hoặc replay effect uncertain khi chưa được phép.
- Xem thay đổi liên quan và trường hợp chạy đúng. Không reset/stash code người khác,
  kill process/ADB, xóa DB hoặc nới verifier để làm triệu chứng biến mất.

## 2. Theo dấu ranh giới

Với mỗi boundary, ghi input identity (không nội dung riêng), output status/error,
revision/session epoch, thời gian và owner. Dữ liệu allowlisted/redacted; không dump
environment, token, request body, clipboard hoặc DB. Không chạy signing/install như
một phép kiểm read-only.

[Root cause và phòng vệ](references/evidence-debugging.md) có checklist. Instrument
nhỏ có budget, không blocking I/O làm thay deadline điều khiển; gỡ instrumentation
thử nếu không còn cần, giữ diagnostic có giá trị theo convention repo.

## 3. Một giả thuyết, một phép phân biệt

Viết “nếu nguyên nhân X đúng thì quan sát Y; nếu sai thì Z”. Chọn test nhỏ nhất phân
biệt được hai trường hợp. Không gộp nhiều fix suy đoán rồi xem lỗi còn không.
Sau vài giả thuyết sai, xem lại model/boundary hoặc nhờ reviewer; không mở rộng agent
fan-out, quyền hay retry. Timeout không chứng minh công việc chưa thực hiện.

## 4. Regression, sửa và kiểm

- Test hành vi qua production seam, expected độc lập. Không xóa code hiện có để làm
  theo TDD máy móc. Baseline/mutant/rollback chỉ trên bản sao có bảo toàn dữ liệu.
- Sửa đúng owner, không thêm bypass controller hoặc policy lỗi bằng string matching.
- Chạy gate hẹp rồi rộng theo blast radius; kiểm negative/cancel/restart khi liên quan.
- Báo command/exit và phạm vi; test zero cases hoặc compile failure không là behavior proof.
- Nếu chặn môi trường/quyền, báo nguyên nhân, không đổi client để lách.

Dùng [testing runbook](../../../docs/development/testing.md) thay plugin TDD/verification
không được expose. Kết quả người/agent nói không thay output thật. Không tự commit,
push, tạo issue hay gửi dữ liệu lên dịch vụ ngoài chỉ vì phát hiện lỗi.
