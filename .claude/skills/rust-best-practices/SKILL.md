---
name: rust-best-practices
description: Review or implement Rust ownership, async cancellation, errors and tests in Riviu without changing its locked toolchain or device-effect contracts.
metadata:
  origin: adapted-upstream
---

# Rust thực hành cho Riviu

Bản chọn lọc/adapt từ Apollo GraphQL skills (MIT), không phải bản upstream nguyên
trạng. [Provenance](UPSTREAM.json) và [license](LICENSE) đi cùng skill. Các chapter
upstream đã được đọc; phần ví dụ phụ thuộc Rust mới hơn pin, lệnh install/update,
all-features và ví dụ unsafe thiếu precondition không được đưa vào bản local.

## Quy trình

1. Xác định contract và owner module; đọc Cargo/lock/toolchain, không suy MSRV từ
   phiên bản latest. Tài liệu repo thắng ví dụ chung.
2. Dùng borrowing khi caller giữ ownership; chuyển giá trị khi lifetime đi theo task.
   Không clone Arc/string chỉ để tránh hiểu lifetime. Đo trước khi tối ưu copy.
3. Tách error typed cho policy/retry; anyhow cho context ở boundary, không parse câu
   lỗi hiển thị để quyết định retry. Không log credential/request nhạy cảm.
4. Async: xác định ai sở hữu task, permit, cancellation và cleanup. Future bị drop
   không chứng minh effect ngoài tiến trình chưa chạy. Device effect đã dispatch phải
   drain và đối soát qua control plane, không bọc timeout để bỏ nó giữa chừng.
5. DB blocking đi StorageExecutor; transaction không giữ qua await HTTP/device.
   Backpressure trước spawn, không tạo hàng đợi task vô hạn rồi chờ semaphore bên trong.
6. Regression đi qua production path với fake boundary; kiểm failure/cancel/restart
   và negative case. Expected độc lập với helper dưới test; zero tests không PASS.
7. Chạy gate đúng phạm vi theo [testing](../../../docs/development/testing.md).
   Không rustup update, cargo update/install, all-features, clean hoặc nới lint để xanh.

## Ownership và đồng bộ

[Reference chọn lọc](references/review-checklist.md) giữ các điểm dễ nhầm về Send/Sync,
lock qua await, dispatch và unsafe. Dùng rustc/Clippy hiện hành để kiểm bounds thật;
không suy một type luôn Send/Sync chỉ từ tên wrapper.

Không đổi stack, dependency hay API công khai chỉ để hợp một pattern. Large-file
refactor phải giữ public contract, evidence và source-based tests liên quan.
