# Hướng dẫn cho agent tiếp nhận dự án

> **Luôn cập nhật tài liệu này.** Sửa gì ảnh hưởng tới kiến trúc, ràng buộc thiết bị,
> hay danh sách "đừng làm lại" thì cập nhật ngay trong cùng lần thay đổi đó — vào
> **file đúng chủ đề** trong [`docs/agents/`](docs/agents/README.md), không vào file này.
> Đây là thứ đầu tiên agent sau đọc.
>
> **Cập nhật lần cuối:** 27/08/2026.

---

## File này là cửa vào

Nội dung thật nằm ở **[`docs/agents/`](docs/agents/README.md)**. Bắt đầu từ bản mục lục:
nó nói mục §x nào ở file nào.

Tới 27/08/2026 file này dài **10.385 dòng, 754 KB, không mục lục**, và số mục **không
theo thứ tự file** — §9.43 nằm giữa §9.20 và §9.21; §9.73 sửa §9.68 từ 150 dòng *bên
trên* nó. Nên quy tắc người ta tưởng là đúng, "đọc xuống, mục sau đè mục trước", **sai**.
Nó đã lừa chính người viết nó nhiều lần trong một tuần: lấy `BROADCAST_CAP = 2048` từ
§9.68 khi §9.73 đã ghi con số thật là 128, và tin "helper APK chưa pin" khi nó đã pin và
đã cài lên 19/19 máy.

Việc chia file **không đổi một chữ nào** của nội dung. Phép chia được kiểm bằng cách dựng
lại toàn bộ và so từng dòng với bản gốc: 10.376 dòng thân, khớp tuyệt đối.

## Đọc gì trước

**Nếu bạn sắp chạm vào WDA hoặc thiết bị iOS: đọc
[§2](docs/agents/02-wda-doc-truoc-khi-sua.md) trước, hết mục.** Đó là mục duy nhất trong
tài liệu này mà bỏ qua có thể làm hỏng thiết bị thật, và nó ngắn.

| § | Chủ đề | File |
|---|---|---|
| §1 | Dự án này là gì | [`01-du-an-va-ten.md`](docs/agents/01-du-an-va-ten.md) |
| §2 | **Đọc TRƯỚC KHI sửa bất cứ thứ gì liên quan tới WDA** | [`02-wda-doc-truoc-khi-sua.md`](docs/agents/02-wda-doc-truoc-khi-sua.md) |
| §3 | Kiến trúc | [`03-kien-truc.md`](docs/agents/03-kien-truc.md) |
| §4 | Chạy và test | [`04-chay-va-test.md`](docs/agents/04-chay-va-test.md) |
| §5 | Trạng thái bình luận | [`05-trang-thai-binh-luan.md`](docs/agents/05-trang-thai-binh-luan.md) |
| §6 | Cách hiệu chỉnh detector | [`06-hieu-chinh-detector.md`](docs/agents/06-hieu-chinh-detector.md) |
| §7 | Nguyên tắc khi sửa code này | [`07-nguyen-tac.md`](docs/agents/07-nguyen-tac.md) |
| §8 | Unified Agent Runtime | [`08-unified-agent-runtime.md`](docs/agents/08-unified-agent-runtime.md) |
| §9 | Fleet Android | [`09-fleet-android.md`](docs/agents/09-fleet-android.md) |
| §10 | Mở đường cho thiết bị mới | [`10-thiet-bi-moi.md`](docs/agents/10-thiet-bi-moi.md) |
| §9.1–§9.119 | Nhật ký: 129 mục, 6 file | [`diary/`](docs/agents/README.md#nhật-ký-9x) |

**Trạng thái hiện tại của dự án** — cái gì chạy, cái gì chưa — ở nhật ký §9, đọc từ mục
**mới nhất** trở lên. Đó là nơi duy nhất được cập nhật theo từng đợt.

## Số mục là định danh vĩnh viễn

Mã nguồn trích tài liệu này **261 chỗ** (`AGENTS.md §9.5`, `AGENTS.md mục 6`, …). Những
trích dẫn đó vẫn đúng: **số mục không đổi khi file bị chia.** Tra số ở
[bản mục lục](docs/agents/README.md).

Ba quy tắc đi kèm, và cả ba đều từ lỗi đã xảy ra thật:

1. **Đừng trích theo số dòng.** Cả sáu chỗ làm thế trong repo đều đã trỏ lệch 29–33 dòng
   *trước* khi có ai chia file, và một trong ba cái đã rơi sang đoạn nói chuyện khác hẳn.
   Đã đổi hết sang số mục vào 27/08/2026. Tên symbol và số mục sống qua refactor.
2. **`§9.43`, `§9.44`, `§9.45` mỗi số có HAI mục khác nhau, khác ngày.** Một trích dẫn
   trần tới ba số đó không xác định được mục nào — mục lục liệt kê cả hai bên. (`§9.105`
   và `§9.115` cũng trùng số, nhưng đó là các mục "tiếp" cùng chủ đề, cố ý.)
3. **Thêm mục mới thì thêm số mới**, và cập nhật mục lục. Có cổng CI kiểm việc này.

## Cổng

Mọi §9.x kết thúc bằng một dòng "Cổng" phát biểu bằng lệnh cụ thể. Danh sách lệnh đầy đủ
ở [`README.md` mục "Chạy cổng"](README.md#chạy-cổng). Hai cổng giữ chính tài liệu này:

- **`every_agents_citation_resolves`** — mọi `AGENTS.md §x` trong mã và trong docs phải
  trỏ tới một mục thật trong `docs/agents/`; sai một số là CI đỏ.
- **`agents_md_stays_a_door`** — file này phải ở lại ngắn. Nó dài 10.385 dòng đúng vì
  không có gì chặn nó dài ra.
