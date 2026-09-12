# Runbook tiếp nhận agent

## Thứ tự đọc

1. Root [`AGENTS.md`](../../AGENTS.md) và [`README.md`](../../README.md).
2. Toàn bộ [WDA safety](02-wda-doc-truoc-khi-sua.md) trước mọi thay đổi WDA/iOS.
3. [Kiến trúc](03-kien-truc.md), [runtime](08-unified-agent-runtime.md), [developer guide](../developer-guide.md).
4. [Operator guide](../operator-guide.md) và [UI contract](../ui-reference-matrix.md) khi chạm giao diện.
5. Các file còn lại trong [docs/agents](README.md) khi phạm vi đụng tới.

## Hợp đồng làm việc

- Sau mỗi thay đổi UI/Rust liên quan desktop: chạy Tauri dev và kiểm tra trước khi bàn giao.
  Chỉ đóng gói release/Setup khi cần giao bản cài. Lệnh dev nằm trong developer guide.
- Đọc `git status --short --untracked-files=all`; không reset/revert thay đổi không thuộc mình.
- Root AGENTS chỉ là cửa ngắn. Nội dung mới về hành vi người dùng đi vào README hoặc hướng dẫn sản phẩm; ràng buộc kỹ thuật đứng yên đi vào file đúng chủ đề dưới `docs/agents/`.
- Không ghi nhật ký thay đổi theo đợt vào kho tài liệu.
- Không lấy file lịch sử làm trạng thái hiện tại.
- Không thay manifest/device identity, reinstall, tap, Post hoặc retry ngoài phạm vi đang kiểm.
- Giữ stock/RT-MMO profile riêng, deadline trên request, session trước stream và helper/hash pin.
- `src/api.ts` vẫn là biên IPC; control plane chịu admission/ownership cho cả UI lẫn API/MCP.

## Dọn repository

Đọc [manifest dọn tài liệu](../archive/cleanup-2026-09-06.md). File không có reference
chỉ là ứng viên, không phải bằng chứng vô giá trị. Phân biệt asset đang bundle, fixture,
snapshot, log nghiệm thu, file phụ tái tạo được và cache compiler.

`.superpowers/baseline-*` có thể là Git worktree; không xoá bằng recursive cleanup.
`target/` chứa evidence và rollback ngoài cache. Không đụng stash hay dữ liệu vận hành
để làm `git status` đẹp hơn.
