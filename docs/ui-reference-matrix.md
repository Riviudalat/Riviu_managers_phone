# Hợp đồng UI và ma trận tham chiếu

Thiết kế hiện tại: giao diện sáng cố định kể cả khi hệ điều hành dùng dark mode;
shell trắng, nền trung tính `#F5F6F8`, primary
cam Riviu `#C2410C`; control 36 px, body 14 px, chữ phụ 13 px, chú thích tối thiểu 12 px,
heading trang 20 px. Noto Sans/Noto Sans Mono đóng gói cùng app. Control bo 6 px,
panel bo 8 px; khoảng cách theo thang 4/8/12/16/24 px. Giữ Tauri/React/Rust,
tile/canvas, mật độ và cử chỉ thiết bị. Thông báo ngắn có thể dùng toast đóng được; lỗi và trạng thái bền vẫn ở cạnh
hành động, monitor nguồn và ActivityCenter. Đây là tiêu chí triển khai, không tự xác nhận mọi
màn hình đã vượt cổng screenshot/accessibility.

Sidebar rộng 224 px (200 px trên màn hẹp), có nút thu gọn thành icon;
dưới 960 px mặc định dùng thanh icon, vẫn mở rộng được bằng nút điều hướng;
tiêu đề nhóm ẩn/hiện các mục bên trong. Mục đang chọn dùng nền cam nhạt,
chữ cam đậm và vạch cam bên trái. Automation có ba trang Nuôi TikTok, Tương tác,
Đăng bài cùng My Apps, Lượt chạy, Tác vụ đã lưu và Flow thiết bị. Header cao tối thiểu 68 px, trạng thái có nhãn **Toàn hệ thống**;
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
Tên, model, tài khoản và trạng thái máy nằm trên nền sáng riêng phía trên preview;
không che hình điện thoại. Preview trong lưới giữ tỉ lệ 1:2; thẻ có thêm phần thông tin
cao 88 px. Dấu chọn trên thẻ và bộ đếm dưới thanh lọc dùng cùng selection hiện có.
Dải thông tin gồm số máy/tên, @nick · model, chip trạng thái theo `deviceWork` và bước đang
chạy hoặc lý do cần xử lý; chữ dài bị cắt nhưng vẫn đọc đủ qua tên truy cập và tooltip.
Máy cần xử lý có viền đỏ; máy đã chọn có viền cam và dấu tích. Rail **Hiển thị** giữ thanh
trượt và Ctrl + lăn chuột, thêm ba cỡ: **Tổng quan** (ô tối thiểu 100 px), **Vừa** (140 px),
**Lớn** (190 px). Kiểu dải theo dải độ rộng hiện tại, không theo nút đã bấm.
**Ngoại lệ có tên — dải Tổng quan:** chỉ ở cỡ Tổng quan, dải cao khoảng 64 px và chữ
11,5 px (`--text-tile-compact`), dưới mức tối thiểu 12 px, để 20 ô trọn vẹn vừa màn
1366×768 khi sidebar mở. Vừa và Lớn dùng dải 88 px với chữ ≥ 12 px; nút trong dải vẫn ≥ 12 px.
Monitor tiến trình vẫn không modal và hoạt động xuyên trang, mặc định nằm góc phải phía trên; có thể kéo đổi vị trí.
Tab Thiết lập/Hẹn giờ/Theo dõi dùng chung nền trắng và mục chọn cam nhạt. Bảng có cuộn riêng;
thanh hành động và nút chạy nằm trong viewport ở kích thước laptop. Chuyển màu nhẹ
120–180 ms, không hiệu ứng lặp trang trí; hỗ trợ giảm chuyển động.

Khi ghi Macro, hộp Công cụ nhóm được ẩn và ngừng giữ focus. Chỉ một thanh ghi hiển thị:
trong menu điện thoại nếu đang mở, hoặc dưới header ứng dụng. Dừng ghi mở lại tab Macro,
giữ bản nháp và phạm vi UDID đã chốt; không đóng điện thoại hoặc tự chạy Macro.
Phần tài khoản Tương tác cuộn dọc trong khung máy trên cửa sổ hẹp. Điều phối chuyển
thành một cột ở 1100 px; toolbar tự xuống hàng nhưng nhãn Lưu/Chạy vẫn một dòng.

**Hoạt động** dùng một `toastStore` duy nhất và ghi rõ đây là nhật ký tạm trong RAM, mất khi
đóng app, không phải bằng chứng lưu bền. Mỗi mục mang metadata có kiểu do nơi tạo gán
(`cause`, `udid`/`udids`, `runId`); `toastError` lấy `code` có tên, `udid`, `runId` từ lỗi
qua `errorFacts`, fan-out lấy đúng các máy lỗi qua `fanOutActivity`. Panel gom các mục cùng
`cause` thành một dòng có số mục và "N máy", mở ra xem từng mục, và nút "Chọn N máy này" thay
lựa chọn bằng đúng các ID đó rồi về Thiết bị. Câu cho người đọc đứng trước; mã nguyên nhân,
mã lượt chạy và mã thiết bị nằm trong **Chi tiết**. Mục không có `cause` không bị gom theo chữ.
Lời toast theo nguyên tắc ACK chưa phải bằng chứng: "Đã gửi lệnh…", không "Thành công" cho
tác động chưa được xác minh.

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
| Cài đặt máy | chuẩn cài đặt (checkbox + tự áp dụng), phạm vi, bảng kết quả theo máy | danh mục cài đặt, máy/nhóm/toàn bộ | trạng thái đọc từng cài đặt; kết quả applied/alreadyOk/needsManual/refusedBusy/failed | đọc lỗi là “Chưa rõ”, máy bận bị từ chối không chiếm quyền, iOS “Chưa hỗ trợ” không gọi máy, áp dụng chỉ sau khi đã lưu |
| Chẩn đoán | bảng điều kiện, detail bằng chứng | máy/phạm vi | readiness/lỗi; sửa đúng điều kiện | không tự repair từ health false-negative |
| Nuôi | Thiết lập/Hẹn giờ/Theo dõi | scope, nhịp, effect, lịch | phiên/máy/effect; đọc bằng chứng | credential riêng, draft/readiness, target isolation |
| Tương tác | Thiết lập/Theo dõi, assignment | URL hiện tại, actors, nội dung | campaign/outcome; source retry | URL parse stale, profile identity, uncertain |
| Đăng bài | ba khung chọn bài / bài↔máy / thiết bị; caption dialog | nguồn/caption/nhạc/Sheet/máy | preflight, Post/URL/Sheet/cleanup; retry phạm vi thiếu | không đăng lại Partial; active bị lọc ẩn, confirm stale, focus caption, target-bound digest |
| Flow | editor mở, mode/device/fleet, execution detail | graph/node/target/revision | run/node history; mở lỗi | Save/Archive/import identity, guard, node effects |
| Lượt chạy | bảng dense, filters, detail | source/status/time | total/page/source link | bài khác máy; active cũ; pagination |
| My Apps | thư viện tích hợp + quy trình đã lưu | tìm, nhập JSON, tạo/chỉnh | mở chức năng hoặc editor | tải quy trình không giả thành rỗng; import bàn phím |
| Tác vụ đã lưu | bảng và form cấu hình | ứng dụng, revision, scope | chạy/lịch có xác nhận | pending/error không báo 0; giữ revision và target |
| Quản lý tài khoản | bảng và form tài khoản | tên, handle, nền tảng, máy | bản ghi đã lưu/đối chiếu | response kind cũ không ghi đè; không coi handle nhập là login proof |
| Lịch chạy | bảng lịch và cấu hình | thời gian local, trạng thái | lần chạy kế tiếp/kết quả | lưu/bật lịch không chạy ngay; giữ revision |
| Kho nội dung | bảng metadata, bulk toolbar | artifact và target | ledger từng máy | restore monitor, cancel queued, no uncertain retry |
| Trung tâm ứng dụng | bảng package, contextual action | package/version/target | batch/item result; gỡ/cài lại theo máy | artifact snapshot, restart uncertainty; gỡ có confirm mất dữ liệu/tài khoản, máy bận bị từ chối, không tự gửi lại |
| Dữ liệu | năng lực, tác vụ 24 giờ, nhật ký gần nhất | tìm kiếm trong tối đa 200 log đã tải | số liệu theo phạm vi; tra cứu sâu tại Tác vụ | hiển thị giới hạn và phạm vi lọc/xuất |
| API | listener status, config section | địa chỉ/credential | actual bind/restart | config khác listener; lỗi bind hiển thị |
| Cài đặt | section rõ, lưu từng vùng | form/credential | persisted readback | stale response, draft guard, restart indication |
| Trợ giúp | hướng dẫn theo nhiệm vụ | lối vào theo việc cần làm | điều hướng tới màn thật | không phát tác vụ khi bấm lối tắt |

Sidebar theo danh sách MENU trong Sidebar.tsx; editor My Apps, Điều phối và Macro là subview. Dữ liệu và
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

## Thuật ngữ trạng thái máy và phạm vi

Bốn trạng thái khác nhau; nhãn và số đếm đứng cạnh nhãn phải dùng cùng một điều kiện.

| Từ trên UI | Nghĩa | Điều kiện |
|---|---|---|
| **Đã kết nối** | Máy có trong danh sách và trả lời, nhưng chưa chứng minh điều khiển được | `deviceOperationalView(...).kind === "connected"` |
| **Bận** (hoặc nhãn hoạt động như "Đang nuôi TikTok") | Một tác vụ đang giữ máy | `kind === "busy"`: có chủ sở hữu hiện tại hoặc roster `busy` |
| **Chọn được** (Nuôi, Tương tác) / **có thể gán** (Đăng bài) | Máy được phép đưa vào phạm vi, danh sách máy thực hiện hoặc phân công. Máy bận vẫn chọn được, vì tác vụ cũ dừng khi lượt mới bắt đầu. Đây không phải lời khẳng định đã sẵn sàng. | roster `ready`, `busy` hoặc `connected`; lịch Đăng bài còn đòi roster `ready` và không vướng bài đang chờ |
| **Sẵn sàng** | Đã chứng minh điều khiển được: Android có helper, phiên và xác thực sẵn sàng; iOS ở trạng thái `ready` hoặc WDA sẵn sàng | chỉ `deviceOperationalView(...).kind === "ready"` |

Không dùng "sẵn sàng" cho tập rộng hơn `kind === "ready"`. Thẻ chọn máy chỉ có trạng thái roster,
nên roster `ready` hiện là **Đã chuẩn bị**, còn `connected` là **Chưa chuẩn bị**. Số **Sẵn sàng**
ở header và sidebar đếm đúng điều kiện của thẻ máy. Khi chưa đọc được chủ sở hữu tác vụ hoặc
bằng chứng helper Android (bộ nhớ đệm này chỉ được đọc trong Control Center), số đó hiện
**Chưa rõ**, không hiện 0. Mở và đóng **Chọn thiết bị**
mà không sửa thì giữ nguyên phạm vi Toàn bộ, Nhóm hoặc Máy cụ thể. Phạm vi chỉ thành Máy
cụ thể khi người vận hành sửa trong drawer. Phạm vi rỗng không bao giờ thành Toàn bộ;
nút chạy hiện lý do bị khóa.

Phạm vi, máy thực hiện và phân công là ba số khác nhau, mỗi số có nhãn riêng:

| Khái niệm | Nghĩa | Nhãn trên UI |
|---|---|---|
| **Phạm vi máy** | Toàn bộ, Nhóm hoặc Máy cụ thể; giới hạn máy nào được dùng | "N máy trong phạm vi" (Nuôi, Tương tác), "N trong phạm vi" (Đăng bài) |
| **Máy thực hiện** | Máy sẽ chạy lượt Tương tác: máy chọn trong phạm vi cộng máy được thêm bởi tag tài khoản | "N máy · K từ tag", "N máy thực hiện" |
| **Phân công** | Đăng bài ghép từng bài với một máy có thể gán; **Chọn nhanh** và **Hoàn tác** nằm trong nhóm "Phân công", không đổi phạm vi | "N/M bài đã gán" |

Nuôi chỉ có một trình sửa phạm vi: select **Phạm vi thiết bị** chọn kiểu, drawer **Chọn máy**
sửa danh sách; Nuôi không có drawer **Chọn thiết bị** thứ hai. Tương tác và Đăng bài giữ
**Chọn thiết bị** cho phạm vi, còn **Chọn máy** của Tương tác sửa máy thực hiện và tài khoản.

Test entrypoints: `apps/desktop/e2e/pages.spec.ts`, `typography.spec.ts`,
`flow-workspace.spec.ts`, unit của workspace và token tests. Snapshot mock chỉ xác nhận
layout/interaction fixture; smoke Tauri xác nhận renderer/backend thật ở phạm vi đã chạy.
