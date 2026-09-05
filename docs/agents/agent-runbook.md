# Runbook tiếp nhận agent

## Thứ tự đọc

1. Root `AGENTS.md`, sau đó [chỉ mục](README.md) và năm nhật ký mới nhất phù hợp phạm vi.
2. Toàn bộ [§2](02-wda-doc-truoc-khi-sua.md) trước mọi thay đổi WDA/iOS.
3. [Kiến trúc](03-kien-truc.md), [runtime](08-unified-agent-runtime.md), [developer guide](../developer-guide.md).
4. [Operator guide](../operator-guide.md) và [UI contract](../ui-reference-matrix.md) khi chạm giao diện.

## Hợp đồng làm việc

- Đọc `git status --short --untracked-files=all`; không reset/revert thay đổi không thuộc mình.
- Root AGENTS giữ vai trò cửa vào, dưới cổng 120 dòng. Nội dung mới đi vào file đúng chủ đề.
- Số § là định danh vĩnh viễn; §9/§10 và §9.43/§9.44/§9.45 cần thêm file/ngày khi nhập nhằng.
- Không lấy file lịch sử làm trạng thái hiện tại, không lấy số mục lớn hơn làm bằng chứng thay mọi mục trước.
- Không thay manifest/device identity, reinstall, tap, Post hoặc retry ngoài phạm vi đang kiểm.
- Giữ stock/RT-MMO profile riêng, deadline trên request, session trước stream và helper/hash pin.
- `src/api.ts` vẫn là biên IPC; control plane chịu admission/ownership cho cả UI lẫn API/MCP.
- Nhật ký phải ghi phần đã xác minh và phần còn chờ. Các cổng xanh không tự chứng minh Windows sạch hoặc effect công khai.

## Dọn repository

Đọc [manifest dọn tài liệu](../archive/cleanup-2026-09-06.md). File không có reference
chỉ là ứng viên, không phải bằng chứng vô giá trị. Phân biệt asset đang bundle, fixture,
snapshot, log nghiệm thu, file phụ tái tạo được và cache compiler.

`.superpowers/baseline-*` có thể là Git worktree; không xoá bằng recursive cleanup.
`target/` chứa evidence và rollback ngoài cache. Không đụng stash hay dữ liệu vận hành
để làm `git status` đẹp hơn. Di chuyển tài liệu có ngày phải có old/new map; patch/diff
lịch sử giữ byte gốc và được đọc cùng map.

## Skill và công cụ

Nguồn skill repo là `.claude/skills/run-riviu-managers-phone/`; `.agents/` là runtime
copy ignored. Khi sao chép, giữ đường dẫn chuẩn trong nội dung thay vì đổi thành `.Codex`.
Dùng symbol thay số dòng khi mô tả code. Snapshot native và web mock phải được dán nhãn
khác nhau trong report.

## Bàn giao

Ghi mục tiêu, file đã sửa, contract giữ nguyên/đổi, lệnh và exit status, bằng chứng có
đường dẫn, phần chưa chạy và bước kế tiếp. Mở lại mọi artifact được công bố. Chỉ dùng
các lệnh Git theo phạm vi đã thống nhất; không commit/push khi chưa được yêu cầu.

Prompt dùng lại:

```text
Đọc AGENTS.md và docs/agents/README.md; xác định § mới nhất cho phạm vi này.
Kiểm tra worktree, symbol sở hữu và test trước khi sửa. Giữ device identity,
admission/ownership, deadline, persistence và uncertainty semantics.
Viết regression, sửa hẹp, chạy focused/full gates theo rủi ro; phân biệt mock/live.
Cập nhật file tài liệu đúng chủ đề và mục nhật ký mới; sinh lại chỉ mục.
Bàn giao đường dẫn, kết quả lệnh thật và phần còn chờ; không tự commit/push.
```
