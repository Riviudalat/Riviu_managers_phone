# Hợp đồng UI và ma trận tham chiếu

Thiết kế hiện tại: giao diện sáng cố định kể cả khi hệ điều hành dùng dark mode;
shell trắng, nền trung tính `#F5F6F8`, primary
cam Riviu `#C2410C`; control 36 px, nút chính ≥ 40 px, nút icon dày đặc ≥ 32 px (luôn có tên
đọc được), body 14 px, nhãn chính và tên trang 16 px, **Máy N 18 px đậm**, chữ phụ 13 px,
chú thích tối thiểu 12 px. Noto Sans/Noto Sans Mono đóng gói cùng app. Control bo 6 px,
panel bo 8 px; khoảng cách theo thang 4/8/12/16/24 px; mọi con số (bộ đếm, bảng, giờ) dùng
`tabular-nums`. Giá trị lấy từ lớp token `--rv-*` (mục **Token** bên dưới). Giữ Tauri/React/Rust,
tile/canvas, mật độ và cử chỉ thiết bị. Thông báo ngắn có thể dùng toast đóng được; lỗi và trạng thái bền vẫn ở cạnh
hành động, monitor nguồn và ActivityCenter. Đây là tiêu chí triển khai, không tự xác nhận mọi
màn hình đã vượt cổng screenshot/accessibility.

Sidebar rộng 196 px (184 px từ 1100 px trở xuống), có nút thu gọn thành icon; khi mở,
tên **Riviu Manager** hiện đủ cạnh logo và nút thu gọn, không bị cắt;
dưới 960 px mặc định dùng thanh icon, vẫn mở rộng được bằng nút điều hướng;
tiêu đề nhóm ẩn/hiện các mục bên trong. Mục đang chọn dùng nền cam đặc, chữ trắng; hàng logo
cao bằng thanh hệ thống (48 px). Automation có ba trang Nuôi TikTok, Tương tác,
Đăng bài cùng My Apps, Lượt chạy, Tác vụ đã lưu và Flow thiết bị. Đầu trang là **thanh hệ
thống** (mục riêng bên dưới); máy thực hiện của mỗi workspace là một phạm vi riêng.
**Màu cam chỉ dùng cho hai việc:** một hành động chính của mỗi vùng, và mục đang chọn (máy
đã chọn, mục điều hướng đang mở). Tab, nút phân đoạn và bộ lọc đang bật là trung tính (chữ
đậm, gạch chân hoặc nền xám, không cam); hover trung tính; link và thao tác phụ trung tính;
cảnh báo màu hổ phách, lỗi màu đỏ. **Bảo trì → Sửa Riviu Agent** tách khỏi
các lệnh mở máy/đồng bộ. Quét thiết bị xuất hiện tại toolbar Thiết bị hoặc thanh hệ thống
của trang khác, không lặp trong cùng một màn hình.

Modal và drawer giữ focus, Escape đóng lớp đang thao tác, đóng trả focus về nút mở.
Các cửa sổ điện thoại là non-modal: không phủ tối, không giữ Tab bên trong;
chọn được máy và trang khác khi cửa sổ đang mở. Một cửa sổ phóng to dùng lại khi đổi
máy, trả phiên cũ trước khi mở phiên mới. Menu/confirm vẫn ở trên. Chất lượng và FPS
Android chỉnh trực tiếp tại rail; hover bung bảng nổi, ghim giữ bảng cạnh lưới, nhóm
bung/thu số máy bên trong. Dropdown dùng chung nền lớp nổi, viền trung tính, option
hover/đang trỏ nền xám, dấu chọn và menu cuộn, chuyển động 180–200 ms.
My Apps dùng bảng ứng dụng gọn. Mở ứng dụng vào trang vận hành cũ; Thêm Flow mở editor
riêng với thư viện hành động bên trái, canvas chính, cấu hình và thiết bị bên phải.
Tên, model, tài khoản và trạng thái máy nằm trên nền sáng riêng phía trên preview;
không che hình điện thoại. Preview trong lưới giữ tỉ lệ 1:2; thẻ có thêm phần thông tin
cao 88 px. Dấu chọn trên thẻ và bộ đếm dưới thanh lọc dùng cùng selection hiện có.
Dải thông tin gồm số máy/tên, @nick · model, chip trạng thái theo `deviceWork` và bước đang
chạy hoặc lý do cần xử lý; chữ dài bị cắt nhưng vẫn đọc đủ qua tên truy cập và tooltip.
Máy cần xử lý có viền hổ phách; máy đã chọn có viền cam và dấu tích. Rail **Hiển thị** giữ thanh
trượt và Ctrl + lăn chuột, thêm ba cỡ: **Tổng quan** (ô tối thiểu 100 px), **Vừa** (140 px),
**Lớn** (190 px). Kiểu dải theo dải độ rộng hiện tại, không theo nút đã bấm.
**Ngoại lệ có tên — dải Tổng quan:** chỉ ở cỡ Tổng quan, dải cao khoảng 64 px và chữ phụ
11,5 px (`--text-tile-compact`), dưới mức tối thiểu 12 px, để 20 ô trọn vẹn vừa màn
1366×768 khi sidebar mở; riêng **Máy N** vẫn 18 px đậm và đọc đủ. Vừa và Lớn dùng dải 88 px
với chữ ≥ 12 px; nút trong dải vẫn ≥ 12 px. Chip trạng thái trên ô theo bảng màu trạng thái:
Sẵn sàng xanh lá, Đã kết nối (chưa kiểm điều khiển) xanh xám, Bận xanh dương, Cần xem hổ
phách, Ngoại tuyến xám.
Chế độ **Danh sách** của Thiết bị là bảng `<table>` gọn: mỗi máy một dòng 34 px, chữ 13 px,
đủ ≥ 15 dòng ở 1366×768. Cột: Máy (số · tên), Tài khoản (@nick), Việc đang chạy (chip
`deviceWork` + bước), Cần xử lý (chỉ máy cần xử lý; nút Khôi phục helper khi có), Bằng chứng
gần nhất, Kết nối (ẩn dưới 1100 px). Ô dài bị cắt một dòng, còn đủ qua tooltip và tên truy cập
của dòng. Nút **Chọn cột** dùng chung với `ResponsiveTable`, nhớ trên máy tính này; cột Máy luôn hiện.
Bằng chứng gần nhất là mốc mới nhất backend ghi cho máy (cập nhật lượt chạy hoặc mẫu đo tải),
không lấy từ toast hay lần UI đọc: không có thì ghi **Chưa có**, cũ hơn 3 phút ghi **Cũ · …**.
Chưa dùng TanStack Table/Virtual: 50 dòng không cần.
Monitor tiến trình vẫn không modal và hoạt động xuyên trang. **Thay đổi hợp đồng (10/2026):**
từ 1500 px, monitor mở rộng ghim thành cột bên phải, làm hẹp lưới chứ không phủ lên;
khi thu gọn hoặc màn hẹp hơn, nó là một nút gọn trong header. Trạng thái thu gọn của cột được
nhớ trên máy. Cột ghim và nút header không kéo được nữa; chỉ cửa sổ nổi (mở từ nút header dưới
1500 px) còn kéo đổi vị trí. Đổi chỗ ghim/thu gọn không dựng lại canvas điện thoại. Toast không
che lệnh: nằm trên thanh chọn máy và bên trái cột monitor.
Khi có máy được chọn, mọi lệnh trên các máy đó nằm ở một **thanh chọn máy** dính dưới lưới:
Mở màn hình, Đồng bộ thao tác, Chạy Flow…, Công cụ nhóm…, Thêm vào nhóm, Bỏ chọn. Toolbar chỉ
giữ lệnh toàn fleet (Mở toàn bộ và Công cụ khi chưa chọn máy, Nhóm, Bảo trì, quét lại). Khi bộ
lọc đang ẩn máy đã chọn, thanh ghi rõ "N máy đã chọn đang bị bộ lọc ẩn", vì lệnh vẫn đến các máy đó.
Tab Thiết lập/Hẹn giờ/Theo dõi, tab con (Phiên nuôi/Hành vi/AI), bước Tương tác và Đăng bài dùng
chung kiểu chọn trung tính: chữ đậm màu chữ chính và gạch chân cùng màu. Bảng có cuộn riêng;
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

**Lý do bị khóa nằm cạnh nút.** Nút chính bị tắt luôn có lý do ngay bên cạnh (`BlockedReason`,
`aria-describedby` trỏ vào lý do) và nút sửa đưa focus tới đúng trường: Nuôi (phạm vi, thiết lập),
Tương tác (link, Chọn máy, trường bước 2), Đăng bài (thư mục, Chọn nhanh, máy nhận, caption),
Flow (Lưu bản, Xem lỗi), Điều phối (tên, thêm bước), Công cụ nhóm (văn bản, regex, tệp). Lý do
quy trình không gom vào banner và không ẩn. Dòng chức năng bị tắt trong cửa sổ điện thoại nêu lý
do ở tooltip. **Thông báo hệ thống** cấp app (backend, số máy, nhánh iOS/Android, bộ công cụ) gộp
thành một dải: thông báo trùng chữ hiện một lần kèm (×N), thông báo nặng nhất hiện đủ cùng hành
động, phần còn lại sau nút "+N thông báo hệ thống". **Tràn:** tab Công cụ nhóm xuống hàng, không
cuộn ngang ẩn; toolbar Flow tách hai hàng từ 1400 px trở xuống; tên trong tiêu đề cửa sổ điện thoại,
chọn Flow và "Gán:" của Đăng bài cắt chữ nhưng có tooltip. Mọi lệnh của các vùng này vẫn bấm được
ở 1366×768 và 820×560.

## Thanh hệ thống

Thay tiêu đề lớn và các viên trạng thái: một thanh cố định cao 48 px trên cùng cột chính, không cuộn
theo nội dung. Thứ tự: **tên trang** (16 px) · **phạm vi** "Laptop này · N máy" xếp trên **"Cập nhật
hh:mm:ss"** · bốn bộ đếm fleet · pill theo dõi tác vụ (khi có) · **Tìm máy, trang…** (Ctrl+K) ·
**Inspector** có chữ (tooltip ghi chỉ máy Android) · chỗ dành cho công tắc Sáng/Tối (trống, không chiếm
chỗ khi chưa có) · chuông **Hoạt động** · nút quét lại ở các trang khác Thiết bị. N đếm máy trong danh
sách đọc được; trước lần đọc đầu tiên ghi **Chưa rõ**, không ghi 0. "Cập nhật" là giờ máy tính lúc danh
sách máy được đọc gần nhất (`listDevices` hoặc sự kiện `devicesUpdated`); lần đọc lỗi giữ giờ cũ; chưa
đọc lần nào thì ghi **Cập nhật: Chưa rõ**. Đây là tuổi của danh sách, không phải tuổi hình hay kiểm
trạng thái của từng máy.

Một hàng 48 px từ 1040 px, kể cả khi có lượt chạy, 40 máy và bộ đếm **Chưa rõ**: từ 1280 px phạm vi/giờ
cập nhật là một cột cạnh bộ đếm; từ 1024 đến 1279 px phạm vi và giờ cập nhật thành một dòng chú thích
nằm trên bộ đếm trong cùng 48 px, nhường chỗ cho tên trang, pill theo dõi, Ctrl+K, Inspector có chữ,
chuông và nút quét lại. Tên trang tối đa hai dòng; pill theo dõi co tới 120 px và cắt chữ, câu đầy đủ ở
tooltip. Dưới 1024 px (và 1024–1039 px khi có pill) thanh xuống hai hàng: tên trang và các nút ở hàng
trên, phạm vi, bộ đếm và giờ cập nhật ở hàng dưới. Không cuộn ngang ở 820 px.

## Token

Mọi màu, khoảng cách, bo góc, cỡ chữ và chiều cao lấy từ lớp `--rv-*` trong
`apps/desktop/src/index.css`; tên cũ (`--primary`, `--line`, `--text-2`, `--warn`, …) là alias của lớp
này. Theme khác chỉ định nghĩa lại các `--rv-*` (Tối sẽ nằm dưới `:root[data-theme="dark"]`), nên
stylesheet không được viết cứng màu sáng: `shell.css`, `workspace.css`, `controls.css`, `grid.css`,
`control-center.css` và `components/ui/` đã đọc thẳng `--rv-*` cho bề mặt; các file khác còn đọc alias.

| Nhóm | Token | Ghi chú |
|---|---|---|
| Bề mặt 3 tầng | `--rv-surface-workspace` · `--rv-surface-panel` · `--rv-surface-overlay` (thêm `-sunken`, `-subtle` cho giếng trong panel) | nền làm việc / vùng (sidebar, thanh hệ thống, panel, bảng) / lớp nổi (menu, hộp thoại, Ctrl+K, toast) |
| Sân khấu điện thoại | `--rv-surface-stage`, `--rv-stage-*` | luôn tối ở mọi theme |
| Viền, chữ | `--rv-border`, `--rv-border-strong`, `--rv-text-1/2/3`, `--rv-text-inverse`, `--rv-link` | link trung tính, có gạch chân |
| Cam | `--rv-primary`, `--rv-primary-hover`, `--rv-primary-text`, `--rv-primary-soft`, `--rv-focus-ring` | chỉ hành động chính và mục đang chọn |
| Chọn trung tính | `--rv-selected-text`, `--rv-selected-line`, `--rv-selected-fill` | tab, phân đoạn, bộ lọc đang bật |
| Trạng thái | `--rv-status-{ready,unchecked,running,attention,error,offline,unknown}-{text,fill,line,solid}` | Sẵn sàng xanh lá · Chưa kiểm xanh xám · Đang chạy xanh dương · Cần xử lý hổ phách · Lỗi đỏ · Mất kết nối xám · Chưa rõ tím xám; chữ đạt AA trên trắng, trên nền của nó và trên nền làm việc; luôn kèm chữ và icon |
| Khoảng cách | `--rv-space-1/2/3/4/6` = 4/8/12/16/24 px | |
| Bo góc | `--rv-radius-control` 6 px, `--rv-radius-panel` 8 px, `--rv-radius-pill` | |
| Chữ | `--rv-font-caption` 12 · `-secondary` 13 · `-body` 14 · `-label` 16 · `-device` 18 · `-device-lg` 20 · `-title` 20 px; `--rv-numeric` | |
| Chiều cao | `--rv-bar-height` 48 · `--rv-toolbar-height` 40 · `--rv-control-primary` 40 · `--rv-control` 36 · `--rv-control-dense` 32 px | |

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
| Thiết bị | toolbar, group tabs, grid/table, drawer | nhóm, trạng thái, tìm kiếm, máy chọn | trạng thái máy; mở máy hoặc Chẩn đoán | tập lọc giống grid/table; tile/canvas không đổi; chưa có máy: bốn bước cắm USB → cho phép gỡ lỗi USB/Tin cậy → Làm mới → Chẩn đoán; trống vì lọc: nêu bộ lọc + Bỏ lọc |
| Cài đặt máy | chuẩn cài đặt (checkbox + tự áp dụng), phạm vi, bảng kết quả theo máy | danh mục cài đặt, máy/nhóm/toàn bộ | trạng thái đọc từng cài đặt; kết quả applied/alreadyOk/needsManual/refusedBusy/failed | đọc lỗi là “Chưa rõ”, máy bận bị từ chối không chiếm quyền, iOS “Chưa hỗ trợ” không gọi máy, áp dụng chỉ sau khi đã lưu |
| Chẩn đoán | bảng điều kiện, detail bằng chứng | máy/phạm vi | readiness/lỗi; sửa đúng điều kiện | không tự repair từ health false-negative |
| Nuôi | Thiết lập/Hẹn giờ/Theo dõi | scope, nhịp, effect, lịch | phiên/máy/effect; đọc bằng chứng | credential riêng, draft/readiness, target isolation |
| Tương tác | Thiết lập/Theo dõi, assignment | URL hiện tại, actors, nội dung | campaign/outcome; source retry | URL parse stale, profile identity, uncertain |
| Đăng bài | ba khung chọn bài / bài↔máy / thiết bị; caption dialog | nguồn/caption/nhạc/Sheet/máy | preflight, Post/URL/Sheet/cleanup; retry phạm vi thiếu | không đăng lại Partial; active bị lọc ẩn, confirm stale, focus caption, target-bound digest |
| Flow | editor mở, mode/device/fleet, execution detail | graph/node/target/revision | run/node history; mở lỗi | Save/Archive/import identity, guard, node effects |
| Lượt chạy | bảng dense, filters, detail; **Nâng cao** chứa Chạy JSON nâng cao | source/status/time | total/page/source link | bài khác máy; active cũ; pagination; rỗng tách: lọc (Bỏ lọc) · khoảng thời gian (Xem toàn bộ lịch sử) · chưa có lượt (Mở Nuôi TikTok) |
| My Apps | thư viện tích hợp + quy trình đã lưu | tìm, tạo/chỉnh; nhập JSON trong **Nâng cao** | mở chức năng hoặc editor | tải quy trình không giả thành rỗng; import bàn phím qua menu Nâng cao; rỗng có Tạo quy trình đầu tiên |
| Tác vụ đã lưu | bảng và form cấu hình | ứng dụng, revision, scope | chạy/lịch có xác nhận | pending/error không báo 0; giữ revision và target; lập lịch duyệt đúng danh sách máy đã lưu, hộp xác nhận nêu máy và 5 lần chạy đầu; rỗng có Tạo tác vụ đầu tiên |
| Quản lý tài khoản | bảng và form tài khoản; **Nâng cao** chứa nhập/xuất JSON | tên, handle, nền tảng, máy | bản ghi đã lưu/đối chiếu | response kind cũ không ghi đè; không coi handle nhập là login proof; rỗng gọi đúng tên nút Thêm tài khoản và có Thêm tài khoản đầu tiên |
| Lịch chạy | bảng lịch và cấu hình (xếp chồng khi ≤1200 px, gồm 1093 px của laptop 125%) | thời gian local (nêu múi giờ), trạng thái, máy đã duyệt | 5 lần chạy tới; kết quả gần nhất bằng lời | **Lưu** và **Lưu & bật** tách riêng, không chạy ngay; giữ revision; giữ danh sách máy đã duyệt, phạm vi đổi thì hiện chênh lệch, không bật được và bị từ chối tới khi **Duyệt lại**; rỗng: chưa có cấu hình → Tạo cấu hình để hẹn giờ (mở Nuôi TikTok › Hẹn giờ), có cấu hình → Tạo lịch từ cấu hình |
| Kho nội dung | bảng metadata, bulk toolbar | artifact và target | ledger từng máy | restore monitor, cancel queued, no uncertain retry; rỗng có Thêm nội dung đầu tiên; tìm không khớp có Xóa tìm kiếm |
| Trung tâm ứng dụng | bảng package, contextual action | package/version/target | batch/item result; gỡ/cài lại theo máy | artifact snapshot, restart uncertainty; gỡ có confirm mất dữ liệu/tài khoản, máy bận bị từ chối, không tự gửi lại |
| Dữ liệu | năng lực, tác vụ 24 giờ, nhật ký gần nhất | tìm kiếm trong tối đa 200 log đã tải | số liệu theo phạm vi; tra cứu sâu tại Tác vụ | hiển thị giới hạn và phạm vi lọc/xuất |
| API | listener status, config section | địa chỉ/credential | actual bind/restart | config khác listener; lỗi bind hiển thị |
| Cài đặt | section rõ, lưu từng vùng | form/credential | persisted readback | stale response, draft guard, restart indication |
| Trợ giúp | hướng dẫn theo nhiệm vụ | lối vào theo việc cần làm | điều hướng tới màn thật | không phát tác vụ khi bấm lối tắt |

Sidebar theo danh sách MENU trong Sidebar.tsx; editor My Apps, Điều phối và Macro là subview. Dữ liệu và
Mạng/Proxy còn nhánh render nhưng chưa có lối vào sidebar; không tự mở thêm menu.
Danh sách phải tách initial loading, refresh, lỗi có retry, rỗng thật và không khớp
bộ lọc. Refresh lỗi giữ dữ liệu cũ khi còn hợp lệ; response cũ không được đổi view mới.

**Trạng thái rỗng** dùng `EmptyState` trong `components/States.tsx` và luôn có đúng một nút
bước tiếp. "Trống vì bộ lọc" nêu bộ lọc đang bật và có **Bỏ lọc**/**Xóa tìm kiếm**; "chưa có dữ
liệu" có nút tạo dữ liệu đầu tiên. Khi cần nhiều bước (cắm máy), dùng `steps` của `EmptyState`.
Lời gợi ý gọi đúng tên nút đang hiện trên trang.

**Nâng cao.** Thao tác JSON thô (Nhập/Xuất JSON bản ghi, Nhập ứng dụng từ JSON, Chạy JSON nâng
cao) nằm trong menu **Nâng cao** (`ButtonMenu` của `components/ui/Menu.tsx`) ở cuối thanh công
cụ; nút dùng hằng ngày đứng trước. Mở bằng Enter/Space, chọn dòng bằng Enter/Space, đóng trả
focus về nút.

**Lịch.** Mọi trình sửa lịch lặp (Lịch chạy, Hẹn giờ của Nuôi/Tương tác, lịch mỗi giờ của Tác vụ
đã lưu) hiện 5 lần chạy tới kèm múi giờ máy tính, tách **Lưu** (lưu, đang tắt) với **Lưu & bật**,
và ghi danh sách máy được duyệt lúc lưu. Hẹn giờ khung giờ của Nuôi dùng **Lưu** (giữ bật/tắt),
**Lưu & bật**, **Tắt hẹn giờ**, ghi danh sách tường minh cho từng khung và hiện 5 lần chạy dự
kiến; danh sách rỗng bị từ chối, không bao giờ thành mọi máy đang kết nối. Làm mới lịch đọc lại
thành viên nhóm; đang đọc hoặc đọc lỗi là "chưa biết", không phải "đã duyệt". Phạm vi sau đó khác danh sách đã duyệt (nhóm thêm/bớt
máy, máy mới cắm vào lịch "Toàn bộ") thì lịch hiện chênh lệch từng máy, không bật được và lần
chạy đến hạn bị từ chối trước khi gửi lệnh tới khi người vận hành **Duyệt lại**; lịch không tự mở
rộng phạm vi. Máy đã duyệt nhưng đang mất kết nối chỉ bị bỏ qua ở lần chạy đó và được ghi lại.
Lịch đăng bài là lịch một lần theo từng bài, mỗi dòng gắn một máy và qua hộp xem lại riêng.

## Quy tắc thành phần

- Nút công cụ dùng icon quen thuộc, tooltip và accessible name. Lệnh nghiệp vụ dùng icon+kèm nhãn khi cần.
- Chế độ dùng segmented control; binary dùng checkbox/toggle; số dùng input/stepper/slider; option dùng menu.
- Menu chức năng của máy (chuột phải trên thẻ hoặc trong cửa sổ điện thoại; Shift+F10 hay phím
  ContextMenu trên thẻ) dùng primitive `components/ui/Menu.tsx`: Base UI, CSS thuần, alias riêng
  `--ui-hover`/`--ui-border`/`--ui-ring`/`--primary-foreground`, không đổi `--accent`. Tên máy (cùng tên
  với thẻ: alias nếu có, không thì **Máy N**; model và udid ở tooltip) và ô **Tìm chức năng** nằm ngoài phần tử `menu`. Mở bằng bàn phím thì focus vào ô tìm; ↓ vào danh sách,
  → hoặc Enter mở menu con, ← hoặc Escape đóng một cấp, Escape nữa đóng menu và trả focus về thẻ đã mở.
  Menu không modal: không khóa cuộn, không làm inert/ẩn cửa sổ điện thoại phía sau; bấm ra ngoài thì
  đóng. Dòng được chọn chạy sau khi menu đóng hẳn. Hover chỉ đổi màu, không lấy focus khỏi ô tìm.
  **Ngoại lệ có tên — focus guard của Base UI:** span `data-base-ui-focus-guard` (aria-hidden,
  tabindex 0) chỉ chuyển focus; kiểm axe menu loại trừ đúng selector này.
- **Bảng lệnh** (Ctrl+K, ⌘K trên macOS, hoặc nút **Tìm máy, trang…** trên thanh hệ thống; dưới
  1600 px, và dưới 1760 px khi cột theo dõi đang ghim, chỉ còn icon có tooltip ghi phím tắt) dùng primitive `components/ui/CommandPalette.tsx`: Base UI Dialog + Autocomplete inline, CSS
  thuần, cùng alias với menu. Lọc không phân biệt dấu ("may 3" khớp "Máy 3"). Giai đoạn 1 chỉ điều
  hướng và chọn: đi tới trang (các trang của sidebar và Điều phối), đi tới **Máy N** (về Thiết bị,
  chỉ chọn máy đó, focus thẻ/dòng của nó; nếu bộ lọc đang ẩn máy thì bỏ lọc), và **Chọn N máy cần
  xử lý** (đúng tập của bộ đếm Cần xử lý trên header; khi bộ đếm đó chưa biết thì dòng này tắt, không
  chọn cả đội máy). **Chạy tác vụ đã lưu…** chỉ mở trang Tác vụ đã lưu; nút Chạy ở đó vẫn hỏi xác
  nhận. Bảng lệnh không chạy tác vụ trên máy và không thay menu chức năng của máy. Bảng lệnh là
  dialog modal khi mở; Escape trả focus về nơi đã mở (kể cả cửa sổ điện thoại), đóng xong cửa sổ
  điện thoại vẫn non-modal. Dòng được chọn chạy sau khi bảng đóng hẳn; đi tới trang thì focus
  chuyển sang mục sidebar của trang đó (hoặc tab Điều phối). Ctrl+K không mở khi đang có một dialog
  modal khác (ví dụ hộp xác nhận).
- Không card lồng card, không hero marketing: tối đa hai tầng khung lồng nhau, viền chỉ ở ranh
  giới vùng. Bảng (`ResponsiveTable`) và thanh bước (`WorkflowStepper`) nằm trong panel nên chỉ kẻ
  đường trên/dưới, không tự vẽ khung; toolbar Thiết bị là một hàng trên nền làm việc với một
  đường kẻ dưới, không phải thẻ. Toolbar/bảng/section có kích thước ổn định.
- Toolbar: một hàng 40 px từ 1024 px (Thiết bị: Mở toàn bộ, Nhóm, Công cụ, Bảo trì, quét lại, rồi tìm,
  lọc trạng thái, Chọn tất cả, chế độ xem). Ô tìm không bao giờ hẹp hơn 176 px để đọc được từ đã gõ; chỗ
  cho nó lấy từ thao tác phụ: từ 1280 px có chữ; 1180–1279 px Nhóm/Công cụ/Bảo trì chỉ còn icon (giữ tên
  đọc được và tooltip); 1024–1179 px Nhóm, Công cụ, Bảo trì và Quét lại vào menu **⋯ Thêm thao tác thiết
  bị** có chữ (Escape đóng và trả focus về ⋯; hộp thoại mở từ menu trả focus về ⋯). Khi bảng Hiển thị đang
  ghim, các mốc này lùi 180 px (icon 1360–1599, menu 1024–1359); nếu vẫn không đủ, phần lọc xuống hàng thứ
  hai chứ không co ô tìm. Nút chính giữ chữ. Toolbar không lặp số máy (đã ở thanh hệ thống). Dưới 1024 px
  phần lọc xuống hàng thứ hai; thông báo luôn chiếm một hàng riêng. Nút chính ≥ 40 px được giữ ở cả các
  footer/toolbar riêng (Nuôi, Đăng bài, My Apps, Tác vụ đã lưu, Quản lý tài khoản).
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
**Chưa rõ**, không hiện 0.

Thanh hệ thống luôn có bốn **bộ đếm fleet**: **Rảnh** (`kind` là `ready` hoặc `connected`), **Đang chạy**
(`busy`), **Cần xử lý** (`warning`, gồm cả máy quá tải theo sức khỏe máy) và **Mất kết nối**
(`offline`). Số 0 vẫn hiện; số chưa biết hiện **Chưa rõ** theo đúng quy tắc của số Sẵn sàng (Mất kết
nối chỉ chưa rõ trước khi danh sách máy tải xong). Rảnh không có nghĩa là Sẵn sàng. Bấm một bộ đếm
đặt đúng state lọc của select **Trạng thái thiết bị** trên lưới (select có thêm mục **Rảnh**), bấm lại
thì bỏ lọc; từ trang khác thì chuyển về Control Center. Bộ đếm và bộ lọc dùng chung một state và
cùng điều kiện của thẻ máy, không có bản sao thứ hai. Bộ đếm là dòng đọc trạng thái (chấm màu ·
nhãn · số), không viền; bộ đếm đang lọc có nền xám và chữ đậm, không cam. Dưới 1600 px (dưới 1760 px
khi cột theo dõi tác vụ đang ghim) bộ đếm thu về cỡ chú thích. Mở và đóng **Chọn thiết bị**
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
