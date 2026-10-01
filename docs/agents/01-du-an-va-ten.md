## 1. Dự án và tên gọi

Riviu Manager (Riviu Tech) là desktop Rust/Tauri/React quản lý Android/iOS qua một
control plane, có Nuôi, Tương tác, Publish, Flow và Điều phối. Xem [giới thiệu](../../README.md).
Không suy feature parity từ tên nền tảng hoặc enum SocialNetwork.

Tên desktop khác identity artifact điện thoại. Không đồng bộ tên bằng cách đổi
bundle iPhone, credential service name hoặc data directory: chúng gắn signing,
keyring và dữ liệu đã lưu. Manifest dưới `sidecars/wda` sở hữu artifact identity;
`state.rs` giải data directory, không theo productName. Không hardcode serial hoặc
tài khoản test trong recipe máy mới.

[Snapshot rename](../archive/technical-snapshots-2026-10-01/01-du-an-va-ten.md)
giữ lịch sử, không là danh sách thiết bị hiện hành.
