# Tài liệu Riviu Manager

Tài liệu hiện tại phân biệt hợp đồng sản phẩm với bằng chứng nghiệm thu có ngày.
Điểm tiếp nhận agent ngắn: [AGENTS.md](../AGENTS.md).

| Nhu cầu | Điểm vào |
|---|---|
| Vận hành 12 trang, hiểu đầu vào và kết quả | [Hướng dẫn vận hành](operator-guide.md) |
| Sửa code, chọn cổng và chạy ứng dụng | [Hướng dẫn phát triển](developer-guide.md) |
| Tiếp nhận thay đổi, bảo toàn bằng chứng | [Runbook agent](agents/agent-runbook.md) |
| Ràng buộc kỹ thuật (WDA, kiến trúc, fleet) | [docs/agents](agents/README.md) |
| Bố cục, trạng thái và nguồn tham chiếu UI | [Hợp đồng UI](ui-reference-matrix.md) |
| Lý do thiết kế và kế hoạch có ngày | [Kho lịch sử](archive/README.md) |
| Kiểm toán đường dẫn và xoá dữ liệu trùng | [Hồ sơ dọn tài liệu](archive/cleanup-2026-09-06.md) |
| Khảo sát giao thức và công cụ | [GenFarmer](re/genfarmer/README.md), [Riviu Agent](re/riviu-agent/README.md), [RT-MMO](re/rtmmo-agent/README.md) |
| Giới hạn nguồn và ma trận parity | [Provenance](provenance/xiaowei-safe-parity.md) |

## Bằng chứng và tài nguyên

`verification/` giữ log nghiệm thu, checksum và rollback có ngày. `re/` giữ khảo sát
và bằng chứng liên quan; chúng không phải nguồn hướng dẫn vận hành hiện tại.
`fixtures/` chứa dữ liệu và rollback fixture đang được tài liệu/test tham chiếu.
`apps-script/publish-sheet.gs` là mã triển khai Sheet, không phải tệp tạm.

Ảnh trong `apps/desktop/e2e/*-snapshots/` là chuẩn so sánh UI; ảnh trong
`crates/core/tests/fixtures/` là đầu vào detector. Không xoá bằng bộ lọc đuôi ảnh.
`target/` chứa cả build cache và hồ sơ nghiệm thu; phải phân loại trước khi dọn.

## Quy ước

- Tên tài liệu mới dùng lowercase-kebab-case; báo cáo có ngày dùng `YYYY-MM-DD-topic.md`.
- `README.md`, `AGENTS.md`, `SKILL.md` là tên điểm vào của công cụ, giữ nguyên chữ hoa.
- Không ghi nhật ký thay đổi theo đợt vào `docs/agents/`. Hành vi người dùng đổi thì cập nhật hướng dẫn sản phẩm hoặc README gốc.
- Tài liệu hiện tại không ghim số test dễ lỗi thời.

## Kiểm tra

Chạy tại gốc repository sau khi đưa file mới vào Git:

```powershell
python -m unittest scripts.test_check_docs -v
python scripts/check_docs.py
```

Bộ quét chỉ đọc file Git theo dõi và xác minh file/anchor nội bộ. URL ngoài là nguồn
tham khảo, không được coi là đã nghiệm thu chỉ vì liên kết tồn tại.
