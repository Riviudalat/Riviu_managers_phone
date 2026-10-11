---
name: riviu-fix-cycle
description: >
  Dùng khi coordinator nhận một yêu cầu sửa Riviu và phải đưa nó tới "OK" trọn vòng: spec
  worker → worker Orca → merge và cổng tích hợp trên main → build native → đóng controller cũ,
  mở controller mới → preflight từng máy → chạy Nuôi/Tương tác/Đăng bài và theo dõi → kiểm UI
  thật → báo cáo tiếng Việt. Có ngân sách thời gian từng bước, điểm quyết định cần máy thật hay
  bằng chứng mock là đủ, và điều kiện dừng. Không push/release khi người dùng chưa duyệt.
metadata:
  version: "1.0.0"
  origin: local
  project: riviu-manager
---

# Vòng sửa lỗi Riviu từ yêu cầu tới OK

Skill này là trình tự và điểm quyết định; hợp đồng nằm ở owner docs, lệnh nằm ở
[scripts/device-acceptance](../../../scripts/device-acceptance/README.md). Ngân sách, quyết
định và điều kiện dừng chi tiết: [các bước](references/steps.md).

| # | Bước | Ngân sách | Công cụ |
|---|---|---|---|
| 1 | Yêu cầu → spec worker | 10 phút | `riviu-orca-coordinator` (mẫu spec) |
| 2 | Worker → merge → dọn | theo worker; 10 phút coordinator | `riviu-orca-coordinator` |
| 3 | Cổng tích hợp trên main | 15–30 phút | [Cổng theo thay đổi](../../../docs/development/testing.md#cổng-theo-thay-đổi) |
| 4 | Build native | 3–12 phút | `build-native.py` |
| 5 | Đóng controller cũ, mở mới | 5 phút | `close.mjs`, `launch.py` |
| 6 | Preflight từng máy | 5–10 phút | `preflight.mjs` |
| 7 | Chạy chế độ + theo dõi | Nuôi ~25 phút, Đăng bài ≤ 90 phút | `run-*`, `pubwatch.py`, `rotwatch.sh`, `status.mjs` |
| 8 | Kiểm UI thật | 5 phút | `native-ui-shots.mjs` |
| 9 | Báo cáo | 10 phút | tiếng Việt, kèm đường dẫn bằng chứng |

## Quyết định đầu tiên: có cần vòng máy thật không

Đi vòng máy (bước 4–8) khi thay đổi chạm đường chạy trên điện thoại: driver, selector/
perception, engine Nuôi/Tương tác/Đăng bài, verifier, lease/admission, helper. Thay đổi chỉ
UI, docs, tooling hay logic thuần đã có fixture thì dừng ở bước 3 với bằng chứng đúng tầng
([phân tầng bằng chứng](../../../docs/development/testing.md#phân-tầng-bằng-chứng)), và nói
rõ tầng nào chưa kiểm.

## Dừng và hỏi người dùng khi

- còn Post/Send/Sheet `uncertain` trên máy cần dùng: chỉ observe, không chạy vòng mới trên máy đó;
- controller cũ không thoát sau `close.mjs` (không kill);
- cổng tích hợp đỏ vì lỗi không thuộc thay đổi này;
- cùng một lỗi sau hai vòng sửa: chuyển `systematic-debugging` / `riviu-device-diagnostics`
  thay vì vòng thứ ba;
- bước tiếp theo là push, release, hay tương tác công khai ngoài phạm vi đã duyệt.

"OK" chỉ khi mọi nghĩa vụ (máy × việc) có một trạng thái `verified`/`failed`/`skipped`/
`uncertain` theo [riviu-device-acceptance](../riviu-device-acceptance/SKILL.md#6-ghi-kết-quả)
và người dùng đã đọc báo cáo.
