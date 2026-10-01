# Snapshot tài liệu trước chuẩn hóa

Các file trong thư mục này lưu nội dung hướng dẫn cũ để không mất số đo, lý do và
checkpoint. Đây **không phải hướng dẫn thực thi hiện hành**. Những câu “hiện tại”,
“PASS”, “chưa implement” hoặc tọa độ/serial chỉ có nghĩa ở ngữ cảnh ban đầu.

Nguồn chuẩn: [README](../../../README.md), [vận hành](../../operator-guide.md),
[phát triển](../../developer-guide.md), [WDA safety](../../agents/02-wda-doc-truoc-khi-sua.md).

| Snapshot | Owner hiện hành |
|---|---|
| [README cũ](root-README.md) | README gốc và build/release runbook |
| [Operator](operator-guide.md) | docs/operator-guide.md và docs/operator/ |
| [Developer](developer-guide.md) | docs/developer-guide.md và docs/development/ |
| [Tên dự án](01-du-an-va-ten.md) | docs/agents/01-du-an-va-ten.md |
| [Kiến trúc](03-kien-truc.md) | developer guide, core/control-plane source |
| [Chạy/test](04-chay-va-test.md) | testing runbook |
| [Bình luận](05-trang-thai-binh-luan.md) | engine/verifier hiện hành |
| [Runtime](08-unified-agent-runtime.md) | agent_runtime/source và manifest |
| [Android](09-fleet-android.md) | Android driver/capability/catalog |
| [Thiết bị mới](10-thiet-bi-moi.md) | qualification/measurement contract |

Link tương đối đã được đổi vị trí để đọc được sau di chuyển; đây là snapshot tài
liệu, không là bản sao byte của artifact đã attested. Các IPA/manifest/patch/log và
rollback evidence có checksum ở nơi gốc không bị sửa. Dẫn chiếu §9.x cũ không còn
đủ địa chỉ để truy evidence; không suy chúng thành một báo cáo hiện đang có.
