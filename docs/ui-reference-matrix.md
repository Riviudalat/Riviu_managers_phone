# Hợp đồng UI và ma trận tham chiếu

Thiết kế được chốt cho đợt 06/09/2026: shell trắng, nền trung tính `#F5F6F8`, primary
cam `#C2410C`; control 36 px, body 13–14 px, heading 18–20 px. Giữ Tauri/React/Rust,
tile/canvas, mật độ và cử chỉ thiết bị. Không toast nổi; trạng thái ở cạnh hành động,
monitor nguồn và ActivityCenter. Đây là tiêu chí triển khai, không tự xác nhận mọi
màn hình đã vượt cổng screenshot/accessibility.

## Nguồn và cách áp dụng

| Nguồn | Mẫu tham khảo | Áp dụng trong Riviu | Không sao chép |
|---|---|---|---|
| GenFarmer, khảo sát repo | grid, lựa chọn và quan sát fleet | Thiết bị, nhóm, preview, trạng thái máy | thuật toán điều khiển hoặc retry không có bằng chứng |
| Xiaowei, khảo sát provenance | thao tác fleet có phạm vi | action bar, menu ngữ cảnh, kết quả từng máy | nguồn runtime ngoài ma trận provenance |
| [RoxyBrowser](https://roxybrowser.com/) | bộ lọc và tác vụ theo lựa chọn | danh sách Thiết bị, thư viện | định danh/profile browser thay device identity |
| [AdsPower](https://www.adspower.com/) | cột gọn và contextual bulk action | thư viện và Tác vụ | thao tác bulk thiếu review target |
| [MoreLogin](https://www.morelogin.com/) | danh sách profile và trạng thái quét nhanh | hồ sơ automation | lưu hồ sơ đồng nghĩa chạy |
| [Dolphin Anty](https://dolphin-anty.com/) | quản lý hàng/cột và scope chọn | bảng dữ liệu và thư viện | hành động công khai tự retry |
| [AirDroid Business](https://www.airdroid.com/business/) | chi tiết thiết bị trong drawer | drawer Thiết bị/Chẩn đoán | drawer chiếm preview hay làm đổi selection |
| [n8n](https://docs.n8n.io/workflows/executions/all-executions/) | execution history, detail của từng bước | Flow/Tác vụ và liên kết về nguồn | phát lại mù effect hoặc coi workflow là idempotent |

GenFarmer: [khảo sát trong repo](re/genfarmer/README.md). Xiaowei:
[nguồn và parity](provenance/xiaowei-safe-parity.md). Các nhãn công cụ ở bảng là nguồn
tham khảo thiết kế trong kế hoạch đã duyệt, không phải dependency hay code được nhập.
Các nguồn chính thức được mở lại ngày 06/09/2026; cách áp dụng ở hai cột cuối là
quyết định thiết kế Riviu, không phải tuyên bố parity toàn bộ với sản phẩm nguồn.

## Ma trận trang

| Trang | Bố cục chính | Bộ lọc/đầu vào | Đầu ra và đường đi tiếp | Kiểm tra bắt buộc |
|---|---|---|---|---|
| Thiết bị | toolbar, group tabs, grid/table, drawer | nhóm, trạng thái, tìm kiếm, máy chọn | trạng thái máy; mở máy hoặc Chẩn đoán | tập lọc giống grid/table; tile/canvas không đổi |
| Chẩn đoán | bảng điều kiện, detail bằng chứng | máy/phạm vi | readiness/lỗi; sửa đúng điều kiện | không tự repair từ health false-negative |
| Nuôi | Thiết lập/Theo dõi, hồ sơ | scope, nhịp, effect, lịch | phiên/máy/effect; đọc bằng chứng | credential riêng, draft/readiness, target isolation |
| Tương tác | Thiết lập/Theo dõi, assignment | URL hiện tại, actors, nội dung | campaign/outcome; source retry | URL parse stale, profile identity, uncertain |
| Đăng bài | input/assignment, preflight, monitor | media/caption/nhạc/Sheet/máy | Post/URL/Sheet/cleanup; retry phạm vi thiếu | không đăng lại Partial, target-bound digest |
| Flow | editor mở, mode/device/fleet, execution detail | graph/node/target/revision | run/node history; mở lỗi | Save/Archive/import identity, guard, node effects |
| Tác vụ | bảng dense, filters, detail | source/status/time | total/page/source link | bài khác máy; active cũ; pagination |
| Kho nội dung | bảng metadata, bulk toolbar | artifact và target | ledger từng máy | restore monitor, cancel queued, no uncertain retry |
| Trung tâm ứng dụng | bảng package, contextual action | package/version/target | batch/item result | artifact snapshot, restart uncertainty |
| Dữ liệu | bảng, filters, detail | source/time | lịch sử bền, artifact | tổng trước trang; không silent truncation |
| API | listener status, config section | địa chỉ/credential | actual bind/restart | config khác listener; lỗi bind hiển thị |
| Cài đặt | section rõ, lưu từng vùng | form/credential | persisted readback | stale response, draft guard, restart indication |

## Quy tắc thành phần

- Nút công cụ dùng icon quen thuộc, tooltip và accessible name. Lệnh nghiệp vụ dùng icon+kèm nhãn khi cần.
- Chế độ dùng segmented control; binary dùng checkbox/toggle; số dùng input/stepper/slider; option dùng menu.
- Không card lồng card, không hero marketing. Toolbar/bảng/section có kích thước ổn định.
- Không resize control theo label/hover/loading. Chữ wrap có chủ đích; không tràn, đè hàng sau hoặc scale theo viewport.
- Dùng khoảng cách/màu/icon nhất quán; disabled/loading/error/empty phải phân biệt và có hành động tiếp theo.
- Desktop/laptop và viewport hẹp giữ nút chạy, filter, status đọc được; overflow thuộc bảng/canvas phù hợp.
- Kiểm tra contrast, focus-visible, keyboard/dialog, tooltip gần mép, reload và chuyển trang khi chạy/bản nháp.

Test entrypoints: `apps/desktop/e2e/pages.spec.ts`, `typography.spec.ts`,
`flow-workspace.spec.ts`, unit của workspace và token tests. Snapshot mock chỉ xác nhận
layout/interaction fixture; smoke Tauri xác nhận renderer/backend thật ở phạm vi đã chạy.
