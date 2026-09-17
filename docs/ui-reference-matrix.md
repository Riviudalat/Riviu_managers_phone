# Hợp đồng UI và ma trận tham chiếu

Thiết kế hiện tại: giao diện sáng cố định kể cả khi hệ điều hành dùng dark mode;
shell trắng, nền trung tính `#F5F6F8`, primary
cam Riviu `#C2410C`; control 36 px, body 14 px, chữ phụ 13 px, chú thích tối thiểu 12 px,
heading trang 20 px. Noto Sans/Noto Sans Mono đóng gói cùng app. Control bo 6 px,
panel bo 8 px; khoảng cách theo thang 4/8/12/16/24 px. Giữ Tauri/React/Rust,
tile/canvas, mật độ và cử chỉ thiết bị. Không toast nổi; trạng thái ở cạnh hành động,
monitor nguồn và ActivityCenter. Đây là tiêu chí triển khai, không tự xác nhận mọi
màn hình đã vượt cổng screenshot/accessibility.

Sidebar cố định 196 px (184 px trên màn hẹp); tiêu đề nhóm ẩn/hiện các mục bên trong,
không thu toàn sidebar thành icon. Mục đang chọn dùng cam đặc/chữ trắng ngay khi
đổi trang, không chờ fade mới đủ tương phản. Automation có ba trang Nuôi TikTok, Tương tác,
Đăng bài cùng My Apps, Lượt chạy, Tác vụ đã lưu và Flow thiết bị. Header cao tối thiểu 56 px, trạng thái có nhãn **Toàn hệ thống**;
máy thực hiện của mỗi workspace là một phạm vi riêng. Mỗi vùng có một hành động
chính màu cam, thao tác phụ trung tính. **Bảo trì → Sửa Riviu Agent** tách khỏi
các lệnh mở máy/đồng bộ. Quét thiết bị xuất hiện tại toolbar Thiết bị hoặc header
của trang khác, không lặp trong cùng một màn hình.

Modal và drawer giữ focus, Escape đóng lớp đang thao tác, đóng trả focus về nút mở.
Các cửa sổ điện thoại là non-modal: không phủ tối, không giữ Tab bên trong;
chọn được máy và trang khác khi cửa sổ đang mở. Một cửa sổ phóng to dùng lại khi đổi
máy, trả phiên cũ trước khi mở phiên mới. Menu/confirm vẫn ở trên. Chất lượng và FPS
Android chỉnh trực tiếp tại rail; hover bung bảng nổi, ghim giữ bảng cạnh lưới, nhóm
bung/thu số máy bên trong. Dropdown dùng chung nền trắng, viền trung tính, option
hover/đã chọn màu cam nhạt, dấu chọn và menu cuộn, chuyển động 180–200 ms.
My Apps dùng bảng ứng dụng gọn. Mở ứng dụng vào trang vận hành cũ; Thêm Flow mở editor
riêng với thư viện hành động bên trái, canvas chính, cấu hình và thiết bị bên phải.
Monitor tiến trình vẫn không modal và hoạt động xuyên trang. Bảng có cuộn riêng;
thanh hành động và nút chạy nằm trong viewport ở kích thước laptop. Chuyển màu nhẹ
120–180 ms, không hiệu ứng lặp trang trí; hỗ trợ giảm chuyển động.

Khi ghi Macro, hộp Công cụ nhóm được ẩn và ngừng giữ focus. Chỉ một thanh ghi hiển thị:
trong menu điện thoại nếu đang mở, hoặc dưới header ứng dụng. Dừng ghi mở lại tab Macro,
giữ bản nháp và phạm vi UDID đã chốt; không đóng điện thoại hoặc tự chạy Macro.
Phần tài khoản Tương tác cuộn dọc trong khung máy trên cửa sổ hẹp. Điều phối chuyển
thành một cột ở 1100 px; toolbar tự xuống hàng nhưng nhãn Lưu/Chạy vẫn một dòng.

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
| Đăng bài | ba khung chọn bài / bài↔máy / thiết bị; caption dialog | nguồn/caption/nhạc/Sheet/máy | preflight, Post/URL/Sheet/cleanup; retry phạm vi thiếu | không đăng lại Partial; active bị lọc ẩn, confirm stale, focus caption, target-bound digest |
| Flow | editor mở, mode/device/fleet, execution detail | graph/node/target/revision | run/node history; mở lỗi | Save/Archive/import identity, guard, node effects |
| Lượt chạy | bảng dense, filters, detail | source/status/time | total/page/source link | bài khác máy; active cũ; pagination |
| My Apps | thư viện tích hợp + quy trình đã lưu | tìm, nhập JSON, tạo/chỉnh | mở chức năng hoặc editor | tải quy trình không giả thành rỗng; import bàn phím |
| Tác vụ đã lưu | bảng và form cấu hình | ứng dụng, revision, scope | chạy/lịch có xác nhận | pending/error không báo 0; giữ revision và target |
| Quản lý tài khoản | bảng và form tài khoản | tên, handle, nền tảng, máy | bản ghi đã lưu/đối chiếu | response kind cũ không ghi đè; không coi handle nhập là login proof |
| Lịch chạy | bảng lịch và cấu hình | thời gian local, trạng thái | lần chạy kế tiếp/kết quả | lưu/bật lịch không chạy ngay; giữ revision |
| Kho nội dung | bảng metadata, bulk toolbar | artifact và target | ledger từng máy | restore monitor, cancel queued, no uncertain retry |
| Trung tâm ứng dụng | bảng package, contextual action | package/version/target | batch/item result | artifact snapshot, restart uncertainty |
| Dữ liệu | năng lực, tác vụ 24 giờ, nhật ký gần nhất | tìm kiếm trong tối đa 200 log đã tải | số liệu theo phạm vi; tra cứu sâu tại Tác vụ | hiển thị giới hạn và phạm vi lọc/xuất |
| API | listener status, config section | địa chỉ/credential | actual bind/restart | config khác listener; lỗi bind hiển thị |
| Cài đặt | section rõ, lưu từng vùng | form/credential | persisted readback | stale response, draft guard, restart indication |
| Trợ giúp | hướng dẫn theo nhiệm vụ | lối vào theo việc cần làm | điều hướng tới màn thật | không phát tác vụ khi bấm lối tắt |

Sidebar có 16 page; editor My Apps, Điều phối và Macro là subview. Dữ liệu và
Mạng/Proxy còn nhánh render nhưng chưa có lối vào sidebar; không tự mở thêm menu.
Danh sách phải tách initial loading, refresh, lỗi có retry, rỗng thật và không khớp
bộ lọc. Refresh lỗi giữ dữ liệu cũ khi còn hợp lệ; response cũ không được đổi view mới.

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
