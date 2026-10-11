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

Sidebar rộng 196 px (184 px từ 1100 px trở xuống), có nút thu gọn; khi mở,
tên **Riviu Manager** hiện đủ cạnh logo và nút thu gọn, không bị cắt. Mục đang chọn dùng nền
cam đặc, chữ trắng; hàng logo cao bằng thanh hệ thống (48 px). Đầu trang là **thanh hệ
thống** (mục riêng bên dưới); máy thực hiện của mỗi workspace là một phạm vi riêng.

**Điều hướng** (`components/navigationMenu.ts` là nguồn duy nhất của sidebar, tiêu đề trang và
Ctrl+K). Tên thuần Việt; tiêu đề trang trùng tên mục điều hướng.
- **Vùng ghim** trên cùng: Trung tâm (Trung tâm điều khiển) · Nuôi TikTok · Tương tác · Đăng bài ·
  Lượt chạy · Trợ giúp. Mỗi mục có chữ, cao 40 px, không cuộn, nên luôn thấy ở 1093×614 và 820×560.
- Bốn nhóm cuộn riêng bên dưới: **Dàn máy** (Cài đặt máy) · **Công việc** (Lịch chạy · Tác vụ đã
  lưu · Flow thiết bị) · **Nội dung** (Kho nội dung · Quản lý tài khoản · Ứng dụng) · **Hệ thống**
  (Chẩn đoán · API · Cài đặt). Mỗi trang chỉ có một chỗ: trang đã ghim không lặp trong nhóm của nó
  (nhóm gốc vẫn là Dàn máy/Công việc/Hệ thống, Ctrl+K ghi nhóm đó).
- Cửa sổ thấp hơn 900 px gập mọi nhóm người vận hành chưa tự mở/gập; màn cao hơn mở sẵn; lựa chọn
  mở/gập được nhớ trên máy. Tới một trang trong nhóm đang gập từ ngoài sidebar (Ctrl+K, Trợ giúp)
  thì nhóm đó tự mở để thấy mục đang chọn; đổi trang không bao giờ gập nhóm. Nhóm đang gập chứa
  trang hiện tại có tiêu đề cam.
- Thanh thu gọn rộng 76 px giữ icon và tên ngắn dưới icon (Trung tâm, Tài khoản, …), không bao giờ
  chỉ có icon; editor Quy trình ứng dụng tự dùng thanh này.
- **Ứng dụng** là hai trang riêng, chuyển bằng hàng thẻ trên đầu: **Quy trình ứng dụng** (trước là
  My Apps) và **Gói cài đặt** (trước là Trung tâm ứng dụng: cài/gỡ APK, IPA). Mục sidebar mở thẻ
  đầu; bấm lại khi đang ở Gói cài đặt thì giữ nguyên thẻ.
- Trợ giúp có bảng **Từ GenFarmer sang Riviu** (Control Center, My Apps, Automation, Runs, Tasks,
  Schedules, Inspector, Groups, Settings/stream), ghi rõ giới hạn: Inspector chỉ máy Android.

**Màu cam chỉ dùng cho hai việc:** một hành động chính của mỗi vùng, và mục đang chọn (máy
đã chọn, mục điều hướng đang mở). Tab, nút phân đoạn và bộ lọc đang bật là trung tính (chữ
đậm, gạch chân hoặc nền xám, không cam); hover trung tính; link và thao tác phụ trung tính;
cảnh báo màu hổ phách, lỗi màu đỏ. **Bảo trì → Sửa Riviu Agent** tách khỏi
các lệnh mở máy/đồng bộ. Quét thiết bị xuất hiện tại toolbar Thiết bị hoặc thanh hệ thống
của trang khác, không lặp trong cùng một màn hình.

Modal và drawer giữ focus, Escape đóng lớp đang thao tác, đóng trả focus về nút mở.
Các cửa sổ điện thoại là non-modal: không phủ tối, không giữ Tab bên trong;
chọn được máy và trang khác khi cửa sổ đang mở. Một cửa sổ phóng to dùng lại khi đổi
máy, trả phiên cũ trước khi mở phiên mới. Menu/confirm vẫn ở trên. Không còn rail dọc:
nút **Hiển thị** trên thanh công cụ mở/đóng một bảng ngang dưới thanh (chỉ khi bấm, không theo
hover): cỡ ô (Nhỏ/Vừa/Lớn + thanh trượt), cỡ màn hình lớn, chất lượng lưới, chất lượng cửa sổ
lớn, FPS (áp dụng cho mọi máy Android, kèm câu "sẽ khởi động lại luồng hình Android"), lọc kết
nối, Xoay máy đã chọn, Cài đặt. Chỉ ghi "Đã áp dụng" sau khi lệnh lưu trả kết quả. Dropdown dùng chung nền lớp nổi, viền trung tính, option
hover/đang trỏ nền xám, dấu chọn và menu cuộn, chuyển động 180–200 ms.
Quy trình ứng dụng dùng bảng ứng dụng gọn. Mở ứng dụng vào trang vận hành cũ; Thêm Flow mở editor
riêng với thư viện hành động bên trái, canvas chính, cấu hình và thiết bị bên phải.
Tên, model, tài khoản và trạng thái máy nằm trên nền sáng riêng phía trên preview;
không che hình điện thoại. Preview trong lưới giữ tỉ lệ 1:2; thẻ có thêm phần thông tin
cao 88 px. Dấu chọn trên thẻ và bộ đếm dưới thanh lọc dùng cùng selection hiện có.
Dải thông tin gồm số máy/tên, @nick · model, dòng trạng thái gộp (`DeviceStatusLine`: biểu
tượng + "Nhãn: việc/lý do" + màu trạng thái) và bước đang chạy hoặc lý do cần xử lý; chữ dài
bị cắt nhưng vẫn đọc đủ qua tên truy cập và tooltip.
Máy cần xử lý có viền hổ phách; máy đã chọn có viền cam và dấu tích.

**Ba chế độ xem** (nút có chữ trên thanh công cụ, nhớ theo người dùng, tập chọn giữ nguyên khi
đổi): **Tổng quan** (mặc định cho người mới), **Màn hình** (preview lớn như cũ) và **Danh sách**.
Ô **Tổng quan** là ô ngắn ~82 px: chữ bên trái, preview 40 px (36 px khi cửa sổ thấp hơn
700 px) bên phải để định vị máy; Máy N 20 px đậm không bao giờ bị cắt; @tài khoản 14 px rút
gọn trên ô và hiện đủ khi ô được chọn hoặc có focus (kể cả bàn phím); một dòng trạng thái 14 px
có lý do/bước. Nút mở màn hình hiện khi hover/focus/chọn; nhấp đúp vẫn mở. Màn hình giữ thanh
trượt và Ctrl + lăn chuột với ba cỡ **Nhỏ** (100 px), **Vừa** (140 px), **Lớn** (190 px); kiểu
dải theo dải độ rộng hiện tại. Ở cỡ Nhỏ, dải cao khoảng 64 px với chữ phụ 11,5 px
(`--text-tile-compact`, ngoại lệ có tên); Vừa và Lớn dùng dải 88 px với chữ ≥ 12 px.

| Cỡ cửa sổ (CSS px) | Tổng quan, trọn vẹn ở đầu trang | Máy N / trạng thái | Cuộn ngang |
|---|---|---|---|
| 1920×1080 | 40/40 ô | 20 / 14 px | không |
| 1366×768 | 34 ô (≥ 20) | 20 / 14 px | không |
| 1093×614 (Windows 125%) | 20 ô | 20 / 14 px | không |
| 820×560 | 6 ô | 20 / 14 px | không |

Đo bằng mock trình duyệt, `fleetSize: 40`, hai box 20 + 19 máy không theo dải số, sidebar mở.

**Box là danh sách máy được gán**, không suy từ Máy N: mỗi nhóm đã lưu là một box. Lưới Tổng
quan và Màn hình chia theo box, mỗi phần có tiêu đề "tên · Laptop này · tổng · sẵn sàng · đang
chạy · cần xử lý"; máy không thuộc nhóm nào nằm ở phần **Chưa thuộc box** cuối lưới; máy thuộc
hai nhóm hiện một lần ở nhóm đầu. Số trên tiêu đề box đếm **toàn bộ máy đã gán** (đang có mặt);
tìm kiếm và lọc trạng thái/kết nối chỉ đổi ô được vẽ và ghi riêng "đang lọc: thấy N/T", không bao
giờ thay mẫu số. Chip box dưới thanh công cụ lọc lưới về một box, nhãn lọc chung "Đang lọc: … ·
Bỏ lọc"; **+ Box** mở quản lý nhóm. Thứ tự trong box là thứ tự số máy, không đổi khi trạng thái
đổi. Ô có focus nằm trên ô bên cạnh để @tài khoản đầy đủ không bị che; trên ô ngắn, tuổi hình
hiện dạng gọn "65 giây" cạnh Máy N (đủ câu trong tooltip và tên truy cập). Bảng Hiển thị khi đóng
vẫn giữ trạng thái đang áp dụng (ẩn, không gỡ khỏi trang).

**Hoãn sang đợt sau (V4, đã ghi nhận):** cột **Chủ giữ** và **Cập nhật** của Danh sách; ngăn Nhóm
mở theo yêu cầu và ghim được ở ≥ 1600 px (hiện vẫn là hộp quản lý nhóm); cỡ ô trong Hiển thị chỉ
có tác dụng ở chế độ Màn hình (cần ghi rõ hoặc khóa ở Tổng quan); đoạn mô tả cột Danh sách cũ ngay
dưới đây còn tên cột trước V4 (Trạng thái, Cần xử lý) và sẽ được thay bằng một hợp đồng cột duy nhất.

**Độ mới hình**: ô ghi "hình N giây trước" khi khung hình gần nhất cũ hơn 20 giây và phủ
**Hình cũ** lên preview khi quá 60 giây (`frameFreshness.ts`). Đây là tuổi hình, không phải
tuổi lần kiểm trạng thái hay kết nối; Làm mới không đặt lại nó. Chip trạng thái trên ô theo bảng màu trạng thái:
Sẵn sàng xanh lá, Đã kết nối (chưa kiểm điều khiển) xanh xám, Bận xanh dương, Cần xem hổ
phách, Ngoại tuyến xám.
Cột Danh sách: Máy · Box · Tài khoản · Công việc (trục việc) · Điều khiển (trục điều khiển) ·
Lý do · Bằng chứng · Kết nối. Chế độ **Danh sách** của Thiết bị là bảng `<table>` gọn: mỗi máy một dòng 34 px, chữ 13 px,
đủ ≥ 15 dòng ở 1366×768. Cột: Máy (số · tên), Tài khoản (@nick), Trạng thái (dòng
trạng thái gộp + bước), Cần xử lý (chỉ máy cần xử lý; nút Khôi phục helper khi có), Bằng chứng
gần nhất, Kết nối (ẩn dưới 1100 px). Ô dài bị cắt một dòng, còn đủ qua tooltip và tên truy cập
của dòng. Nút **Chọn cột** dùng chung với `ResponsiveTable`, nhớ trên máy tính này; cột Máy luôn hiện.
Bằng chứng gần nhất là mốc mới nhất backend ghi cho máy (cập nhật lượt chạy hoặc mẫu đo tải),
không lấy từ toast hay lần UI đọc: không có thì ghi **Chưa có**, cũ hơn 3 phút ghi **Cũ · …**.
Chưa dùng TanStack Table/Virtual: 50 dòng không cần.
**Hàng đợi Cần xử lý (PLAN §5) thay panel "Theo dõi tác vụ" (10/2026).** Không gì ngoài thao tác
của người dùng được mở hàng đợi hay đổi diện tích lưới, ở mọi cỡ màn: không còn cột ghim từ 1500 px,
không còn cửa sổ nổi kéo được, lượt đăng mới không tự mở. Thanh hệ thống có chip **Cần xử lý N**
(hổ phách khi N > 0; **≥N** khi một nguồn chưa đọc được hoặc chưa trả lời, không bao giờ coi là
không có) và, khi có lượt chạy, pill **Tiến trình công việc** (icon · số lượt · %, thanh tiến độ
mảnh). Mục mới chỉ tăng chip và hiện một thông báo cạnh chip ~6 giây: lớp phủ không chiếm chỗ,
không bấm được, không lấy focus (trình đọc màn hình đọc qua vùng live). Hàng đợi mở bằng chip, pill
hoặc **Ctrl+Shift+X**, là ngăn phủ bên phải dưới thanh hệ thống (460 px, **Phóng rộng** tới 1180 px),
không bao giờ là cột: mở/đóng không đổi tập chọn, thứ tự ô hay focus (bấm chip giữ focus ở chip;
phím tắt giữ focus ở ô đang focus; Tab từ chip/pill vào ngăn; Escape trong ngăn đóng và trả focus
về nút đã mở). Đóng là trở lại đủ ô. Hai tab:
- **Cần xử lý**: mỗi mục một máy và một lý do — Máy N · box (nhóm được gán, không có thì "Chưa
  thuộc box") · lý do · từ lúc (tương đối + giờ; máy cần xử lý ghi "Thấy từ" vì đó là lúc cửa sổ
  thấy) · việc đang giữ máy (trục Công việc hiện tại; chưa đọc được thì "Chưa rõ") · bước tiếp.
  Nguồn: các máy của bộ đếm Cần xử lý (cùng tập); bài đăng Chưa rõ và lỗi ghi Sheet từ mô hình kết
  quả chung (§6) của các lượt trong 24 giờ và các lượt đăng `uncertain`/`partial` trong 30 ngày;
  máy thất bại/chưa rõ của lượt Nuôi/Tương tác/Flow có lỗi trong 24 giờ. Mỗi lượt đăng có tổng ba
  số riêng (`PublishRunOutcomeTotals`), nên chi tiết chiến dịch, hàng đợi và Lượt chạy cùng số.
  Dòng bài dùng `PostOutcomeLayers`; lỗi Sheet vẫn giữ link. **Chưa rõ và lỗi Sheet ở lại tới khi có
  kết luận** ("Ở lại tới khi có kết luận"), chỉ có **Xem bằng chứng** và **Theo dõi**; không có nút
  đăng/gửi lại hay nhận máy. Mục thất bại khác có **Đã xem** theo lượt (nhớ trên máy tính này, lượt
  cập nhật thì hiện lại, Hoàn tác được). Máy cần xử lý có **Chi tiết máy**. Thứ tự cố định theo số
  máy. Ngân sách đọc hữu hạn: tối đa 30 lượt đăng và 20 lượt khác.
- **Đang chạy**: các lượt đang chạy (từ hai lượt có danh sách tiến độ từng lượt), lượt đang chọn
  với tiến độ từng máy, nhật ký, bằng chứng và **Dừng/Tạm dừng**. Lượt đã xong rời tab này (vào
  **Lượt chạy**), trừ lượt đang được theo dõi (đang mở trên tab, chọn bằng Theo dõi, vừa bắt đầu
  hoặc vừa dừng) cho tới khi đóng ngăn. **Lượt chạy** có **Xem nhật ký từng máy** để mở lại một lượt
  bất kỳ ở tab này.
Đổi trang vẫn giữ hàng đợi; tab Đang chạy giữ máy/tab/vị trí đọc khi đóng rồi mở lại. Toast không
che lệnh: nằm trên thanh chọn máy.
Khi có máy được chọn, mọi lệnh trên các máy đó nằm ở một **thanh chọn máy** dính dưới lưới:
Mở màn hình, Đồng bộ thao tác, Chạy Flow…, Công cụ nhóm…, Thêm vào nhóm, Bỏ chọn. Toolbar chỉ
giữ lệnh toàn fleet (Mở toàn bộ và Công cụ khi chưa chọn máy, Nhóm, Bảo trì, quét lại).
**Ổn định khi chọn (10/2026, PLAN §3):**
- Thứ tự thẻ và dòng cố định theo số máy trong nhóm; trạng thái đổi không bao giờ sắp xếp lại.
  Trong lúc nút chuột đang nhấn trên lưới hoặc bảng, tập máy hiện, thứ tự và hai banner phía trên
  lưới giữ nguyên dù nhấn lâu bao nhiêu; chỉ chính con trỏ kết thúc: thả đúng con trỏ đó (áp dụng
  sau cú click của lần thả), hủy, cửa sổ mất focus giữa thao tác, hoặc một sự kiện sau đó của
  chính con trỏ đó chứng minh nút đã nhả (thả ngoài cửa sổ); con trỏ khác (bút, chuột thứ hai)
  không mở khóa. Không có hẹn giờ nào tự mở khóa. Chữ
  trạng thái trong thẻ vẫn cập nhật vì dải thông tin cao cố định.
- Mọi bộ lọc người vận hành đặt (nhóm, trạng thái, tìm, kết nối) hiện thành một dòng
  **Đang lọc: …** kèm "đang thấy V/T máy" và nút **Bỏ lọc**; dòng này chỉ xuất hiện hay mất
  khi người vận hành đổi lọc. Đổi lọc không bao giờ thêm máy vào lựa chọn; phạm vi lệnh là
  lựa chọn, không suy từ bộ lọc.
- Lựa chọn bám udid. Thanh chọn máy luôn ghi **"N đã chọn · V đang thấy · H ẩn do lọc"**
  (thêm "· K bị loại" khi có); bấm vào đó hoặc **Ctrl+Shift+L** mở danh sách đủ: Máy N,
  nhóm, @tài khoản, đánh dấu máy ẩn do lọc, và danh sách bị loại.
- Máy đang chạy việc (chủ Nuôi, Tương tác hoặc Tác vụ tự động, hoặc roster `busy`), mất kết
  nối hoặc rời danh sách không thêm được vào lựa chọn: click, khung kéo, Ctrl+A, **Chọn tất
  cả (N)** (N chỉ đếm máy chọn được), bảng lệnh và chọn theo nhóm đều bỏ qua và báo tên máy
  cùng lý do. Máy đã chọn rồi chuyển sang trạng thái đó rời tập nhận lệnh, vào dòng **Bị
  loại (K): Máy N – lý do** trên thanh, và không tự được chọn lại khi rảnh; chỉ người vận
  hành chọn lại. Chủ giữ chốc lát hoặc thay chính người vận hành (điều khiển trực tiếp, Đồng
  bộ, Sửa chữa, dọn nền) không làm máy bị loại. Chủ tác vụ chỉ được đọc ở trang này; ở trang
  khác chỉ roster đếm.
- Khi mọi máy đã chọn bị loại, lựa chọn rỗng là **không máy nào**, không phải toàn bộ: lệnh
  của thanh tắt, Công cụ nhóm và Chạy JSON nâng cao ghi 0 máy, cho tới khi người vận hành chọn
  lại hoặc **Xóa danh sách bị loại** / **Bỏ chọn**.
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
lựa chọn bằng đúng các ID đó rồi về Trung tâm điều khiển. Câu cho người đọc đứng trước; mã nguyên nhân,
mã lượt chạy và mã thiết bị nằm trong **Chi tiết**. Mục không có `cause` không bị gom theo chữ.
Lời toast theo nguyên tắc ACK chưa phải bằng chứng: "Đã gửi lệnh…", không "Thành công" cho
tác động chưa được xác minh.

**Kết quả bài đăng: một bộ từ.** Chi tiết chiến dịch (cột Kết quả, Bài đã đăng, Ghi Sheet), cửa sổ
tiến trình (dòng máy, `PublishDeviceRecovery`, ô "Kết quả từng máy") và Lượt chạy dùng cùng
`features/operations/postOutcomeModel.ts` + `PostOutcome.tsx`: các lớp **Đã gửi → Đã xác minh →
Đã ghi Sheet**, mỗi lớp có biểu tượng và chữ, cùng **Đang xác minh** và **Chưa rõ** là trạng thái
riêng. **Đã xác minh** chỉ khi bản ghi là trạng thái đã xác minh của backend: `succeeded` +
`post.publicationVerified === true` + link TikTok chuẩn; backend chỉ đặt cờ đó sau khi bước lấy link
chứng minh link, tài khoản, caption và thời điểm của lượt. Thời điểm là **id-allocation-time
evidence**: content ID chuẩn được cấp trong cửa sổ của lượt (sau `prepared_at`) trên đúng tài khoản
dự kiến với caption khớp — không phải giờ xuất bản (định nghĩa đầy đủ: `docs/development/contracts.md`,
Xác minh Publish); UI không hạ bài thỏa định nghĩa này và không gọi nó là giờ đăng. Cờ không thắng
bằng chứng mâu thuẫn trong chính bản ghi: `captionMatched=false`, `submissionTimeMatched=false`,
thiếu tài khoản đã ghi trước Post, tài khoản trong evidence (`post.expectedAccount`,
`accountDiagnostic.expectedAccount/observedAccount`) khác tài khoản frozen trong effect intent (intent là
nguồn chính; evidence chỉ thay khi bản ghi cũ không có tài khoản intent; so handle đã chuẩn hóa: bỏ `@`,
khoảng trắng, không phân biệt hoa thường), hoặc tác giả trong link khác tài khoản đó thì dòng là Chưa rõ
kèm lý do và không tính vào số đã xác minh/đã ghi Sheet. Có link mà thiếu cờ, hoặc capability `alreadyVerified`
(chỉ kiểm hình dạng link) cũng không tính là đã xác minh. Header chiến dịch và nhãn chiến dịch dùng
cùng mô hình: chỉ "Đã hoàn tất"/"Hoàn tất" khi mọi bài đã xác minh, chỉ "Đã ghi Sheet" khi mọi dòng
cần ghi đã được Sheet xác nhận; snapshot chỉ giữ phạm vi thử lại. Dòng nhật ký `succeeded` thô chỉ
nói lượt đăng đã kết thúc, không tự nhận đã xác minh. **Đã ghi Sheet** cần `sheetDelivery.state === "sent"`;
lỗi Sheet không hạ bài đã xác minh và vẫn giữ link. Tổng hợp luôn là số riêng ("20 đã gửi · 18 đã
xác minh · 17 đã ghi Sheet · 2 chưa rõ", thêm "N đang xác minh" khi có), không cộng đã gửi thành
thành công. Dòng Chưa rõ chỉ có **Xem bằng chứng** (cách xác minh, tài khoản, giờ bấm Đăng, khớp
caption/thời điểm, link ứng viên, dữ liệu gốc) và các lệnh theo dõi đã có (**Kiểm tra liên kết**,
**Tiếp tục xác minh bài đã gửi**); không có nút đăng/gửi lại.

**Lý do bị khóa nằm cạnh nút.** Nút chính bị tắt luôn có lý do ngay bên cạnh (`BlockedReason`,
`aria-describedby` trỏ vào lý do) và nút sửa đưa focus tới đúng trường: Nuôi (phạm vi, thiết lập),
Tương tác (link, Chọn máy, trường bước 2), Đăng bài (thư mục, Ghép tự động, máy nhận, caption),
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

Một hàng 48 px từ 1090 px, kể cả khi có lượt chạy, chip Cần xử lý, 40 máy, bảy bộ đếm **Chưa rõ** và
tên trang dài nhất; không lượt chạy thì từ 1024 px: từ 1920 px phạm vi/giờ cập nhật là một cột cạnh bộ
đếm; từ 1024 đến 1919 px phạm vi và giờ cập nhật thành một dòng chú thích nằm trên bộ đếm trong cùng
48 px, nhường chỗ cho tên trang, chip Cần xử lý, pill tiến trình, Ctrl+K, Inspector có chữ, chuông và
nút quét lại. Chip và pill không co và không cắt chữ: chữ "đang chạy" của pill chỉ hiện từ 1920 px,
dưới 1440 px pill chỉ còn icon và % (câu đầy đủ ở tooltip và tên truy cập). Dưới 1760 px mẫu số chỉ còn
ở dòng phạm vi và **Tổng**; dưới 1366 px (dưới 1440 px khi có pill) bộ đếm bỏ icon, giữ chữ và số;
dưới 1280 px **Tổng** nhường chỗ vì dòng phạm vi ngay cạnh hoặc ngay trên ("Laptop này · N máy") đã là
tổng. Tên trang tối đa hai dòng. Dưới 1024 px (và 1024–1089 px khi có pill) thanh xuống hai hàng (77 px):
tên trang và các nút ở hàng trên, phạm vi, bộ đếm và giờ cập nhật ở hàng dưới. Không chồng chữ và
không cuộn ngang ở 820–1920 px (đo trên Thiết bị và Trung tâm ứng dụng, có và không có lượt chạy).

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
| Trung tâm điều khiển | toolbar, group tabs, grid/table, drawer | nhóm, trạng thái, tìm kiếm, máy chọn | trạng thái máy; mở máy hoặc Chẩn đoán | tập lọc giống grid/table; tile/canvas không đổi; chưa có máy: bốn bước cắm USB → cho phép gỡ lỗi USB/Tin cậy → Làm mới → Chẩn đoán; trống vì lọc: nêu bộ lọc + Bỏ lọc |
| Cài đặt máy | chuẩn cài đặt (checkbox + tự áp dụng), phạm vi, bảng kết quả theo máy | danh mục cài đặt, máy/nhóm/toàn bộ | trạng thái đọc từng cài đặt; kết quả applied/alreadyOk/needsManual/refusedBusy/failed | đọc lỗi là “Chưa rõ”, máy bận bị từ chối không chiếm quyền, iOS “Chưa hỗ trợ” không gọi máy, áp dụng chỉ sau khi đã lưu |
| Chẩn đoán | bảng điều kiện, detail bằng chứng | máy/phạm vi | readiness/lỗi; sửa đúng điều kiện | không tự repair từ health false-negative |
| Nuôi | Thiết lập/Hẹn giờ/Theo dõi; tóm tắt cạnh nút bắt đầu luôn liệt kê mọi hành động công khai sẽ chạy (tim, lưu, bình luận, theo dõi); nhãn "Tỷ lệ hành động (%)", "Thời gian xem ngắn nhất/dài nhất" | scope, nhịp, effect, lịch | phiên/máy/effect; dừng: "Yêu cầu dừng" → "Đang dừng · a/n máy đã xác nhận" → kết quả từng máy; chỉ dòng kết thúc của đúng mã lượt đã dừng mới xác nhận; "Đã dừng" chỉ khi mọi máy đó xác nhận | credential riêng, draft/readiness, target isolation; máy không có dòng trạng thái = Chưa rõ, không phải đã dừng; không hứa thu hồi hành động đã gửi |
| Tương tác | Thiết lập/Theo dõi, assignment | URL hiện tại, actors, nội dung | campaign/outcome; source retry | URL parse stale, profile identity, uncertain; máy thực hiện đang bận chặn Bắt đầu tới khi bỏ máy (chưa có thao tác giành máy bận) |
| Đăng bài | thanh bước 1 Nội dung · 2 Ghép máy–tài khoản · 3 Xem lại & xác nhận · 4 Theo dõi; ba khung chọn bài / bài↔máy / thiết bị hoặc **Từng bước** (nhớ theo máy người dùng); caption dialog | nguồn/caption/nhạc/Sheet/máy | **Ghép tự động (chưa đăng)** chỉ sửa bản nháp; **Xem lại trước khi đăng** mở hộp "Xem lại và xác nhận đăng công khai", cửa xác nhận duy nhất, đọc đủ Máy N · @ · phiên bản nội dung (tên file, thời lượng, mã bản) · caption · nhạc · link Sheet; preflight, Post/URL/Sheet/cleanup; retry phạm vi thiếu | không đăng lại Partial; active bị lọc ẩn, confirm stale, focus caption, target-bound digest; bản xem lại không bao giờ cắt "…" |
| Flow | editor mở, mode/device/fleet, execution detail | graph/node/target/revision | run/node history; mở lỗi | Save/Archive/import identity, guard, node effects |
| Lượt chạy | bảng dense, filters, detail; **Nâng cao** chứa Chạy JSON nâng cao | source/status/time | total/page/source link | bài khác máy; active cũ; pagination; rỗng tách: lọc (Bỏ lọc) · khoảng thời gian (Xem toàn bộ lịch sử) · chưa có lượt (Mở Nuôi TikTok) |
| Ứng dụng › Quy trình ứng dụng | thư viện tích hợp + quy trình đã lưu | tìm, tạo/chỉnh; nhập JSON trong **Nâng cao** | mở chức năng hoặc editor | tải quy trình không giả thành rỗng; import bàn phím qua menu Nâng cao; rỗng có Tạo quy trình đầu tiên |
| Tác vụ đã lưu | bảng và form cấu hình | ứng dụng, revision, scope | chạy/lịch có xác nhận | pending/error không báo 0; giữ revision và target; lập lịch duyệt đúng danh sách máy đã lưu, hộp xác nhận nêu máy và 5 lần chạy đầu; rỗng có Tạo tác vụ đầu tiên |
| Quản lý tài khoản | bảng và form tài khoản; **Nâng cao** chứa nhập/xuất JSON | tên, handle, nền tảng, máy | bản ghi đã lưu/đối chiếu | response kind cũ không ghi đè; không coi handle nhập là login proof; rỗng gọi đúng tên nút Thêm tài khoản và có Thêm tài khoản đầu tiên |
| Lịch chạy | bảng lịch và cấu hình (xếp chồng khi ≤1200 px, gồm 1093 px của laptop 125%) | thời gian local (nêu múi giờ), trạng thái, máy đã duyệt | 5 lần chạy tới; kết quả gần nhất bằng lời | **Lưu** và **Lưu & bật** tách riêng, không chạy ngay; giữ revision; giữ danh sách máy đã duyệt, phạm vi đổi thì hiện chênh lệch, không bật được và bị từ chối tới khi **Duyệt lại**; rỗng: chưa có cấu hình → Tạo cấu hình để hẹn giờ (mở Nuôi TikTok › Hẹn giờ), có cấu hình → Tạo lịch từ cấu hình |
| Kho nội dung | bảng metadata, bulk toolbar | artifact và target | ledger từng máy | restore monitor, cancel queued, no uncertain retry; rỗng có Thêm nội dung đầu tiên; tìm không khớp có Xóa tìm kiếm |
| Ứng dụng › Gói cài đặt | bảng package, contextual action | package/version/target | batch/item result; gỡ/cài lại theo máy | artifact snapshot, restart uncertainty; gỡ có confirm mất dữ liệu/tài khoản, máy bận bị từ chối, không tự gửi lại |
| Dữ liệu | năng lực, tác vụ 24 giờ, nhật ký gần nhất | tìm kiếm trong tối đa 200 log đã tải | số liệu theo phạm vi; tra cứu sâu tại Tác vụ | hiển thị giới hạn và phạm vi lọc/xuất |
| API | listener status, config section | địa chỉ/credential | actual bind/restart | config khác listener; lỗi bind hiển thị |
| Cài đặt | section rõ, lưu từng vùng | form/credential | persisted readback | stale response, draft guard, restart indication |
| Trợ giúp | hướng dẫn theo nhiệm vụ; bảng Từ GenFarmer sang Riviu | lối vào theo việc cần làm | điều hướng tới màn thật | không phát tác vụ khi bấm lối tắt; bảng tên cũ ghi rõ giới hạn (Inspector chỉ Android) |

Sidebar theo `MENU` và `PINNED` trong `components/navigationMenu.ts`; editor Quy trình ứng dụng,
Điều phối và Macro là subview. Dữ liệu và
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
- Thanh có `flex-wrap` xuống dòng theo flex-basis trước khi co, nên thanh dựa vào phần tử co được phải `nowrap` và đổi bố cục bằng breakpoint hoặc `:has()` (đo ở thanh hệ thống 1024–1099 px khi có lượt chạy). Một nhóm flex được co (`min-width: 0`) mà con của nó `flex: none` thì con tràn ra và đè lên nút bên cạnh, `scrollWidth` không phát hiện được: đo chồng hình chữ nhật giữa các nút.
- Menu tràn "⋯" dựng từ chính các nút inline phải giữ style popup trong đúng mức media hiện menu, và đóng khi mức đổi bằng cách theo dõi nút mở (ResizeObserver), không theo bề rộng cửa sổ; nếu không, trạng thái mở cũ đổi style các nút inline.
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
  1920 px, và ở mọi độ rộng khi có pill tiến trình, chỉ còn icon có tooltip ghi phím tắt) dùng primitive `components/ui/CommandPalette.tsx`: Base UI Dialog + Autocomplete inline, CSS
  thuần, cùng alias với menu. Lọc không phân biệt dấu ("may 3" khớp "Máy 3"). Giai đoạn 1 chỉ điều
  hướng và chọn: đi tới trang (các trang của sidebar, hai thẻ của Ứng dụng và Điều phối; tên cũ
  Control Center, My Apps, Automation, Runs, Tasks, Schedules, Trung tâm ứng dụng vẫn khớp), mở
  danh sách máy Android của **Bắt thuộc tính (Inspector)**, mở bảng **Nhóm máy** (Groups, Box) ở
  Trung tâm điều khiển, đi tới **Máy N** (về Trung tâm điều khiển,
  chỉ chọn máy đó, focus thẻ/dòng của nó; nếu bộ lọc đang ẩn máy thì bỏ lọc), và **Chọn N máy cần
  xử lý** (đúng tập của bộ đếm Cần xử lý trên header; khi bộ đếm đó chưa biết thì dòng này tắt, không
  chọn cả đội máy). **Chạy tác vụ đã lưu…** chỉ mở trang Tác vụ đã lưu; nút Chạy ở đó vẫn hỏi xác
  nhận. Bảng lệnh không chạy tác vụ trên máy và không thay menu chức năng của máy. Bảng lệnh là
  dialog modal khi mở; Escape trả focus về nơi đã mở (kể cả cửa sổ điện thoại), đóng xong cửa sổ
  điện thoại vẫn non-modal. Dòng được chọn chạy sau khi bảng đóng hẳn; đi tới trang thì focus
  chuyển sang mục sidebar của trang đó (hoặc tab Điều phối). Ctrl+K không mở khi đang có một dialog
  modal khác (ví dụ hộp xác nhận).
- **Bản xem lại có hiệu lực theo phiên bản (PLAN §3).** Hộp **Xem lại và xác nhận đăng công
  khai** gắn với một lần kiểm tra (`inputDigest` của backend: máy và roster, tài khoản từng máy,
  file nội dung theo hash, caption, kiểu nhạc, đích Sheet, bản TikTok). Khi hộp đang mở mà một
  máy đã xem lại mất kết nối, chuyển `busy` (ngoài các máy hộp đã nói sẽ nhả) hoặc đổi
  @tài khoản, nút xác nhận tắt và hộp ghi **Bản xem lại đã hết hiệu lực. Chưa gửi lệnh đăng
  nào.** cùng danh sách thay đổi; phải **Kiểm tra lại**. Bấm xác nhận thì app kiểm lại đúng
  yêu cầu đó ngay trước khi gửi; khác máy, bài, bản TikTok, dung lượng, máy cần nhả, đích
  Sheet, roster hoặc digest thì dừng, không gửi, giữ kết quả kiểm tra mới trên hộp kèm danh
  sách đã đổi, và người vận hành xác nhận lại. Máy cần nhả so theo đúng việc đang giữ
  (`envelope`), không chỉ theo máy: cùng máy nhưng việc khác thì hiện "đổi từ … sang …" và phải
  xác nhận lại; Start gửi kèm phong bì đã xem để backend chỉ nhả đúng các việc đó. Kiểm ở UI chỉ
  để báo sớm; backend vẫn tự kiểm bản duyệt trước mọi thay đổi chủ (xem `contracts.md`). Không tự
  thay máy hay tài khoản, không âm thầm bớt phạm vi. Lượt đã gửi (có dấu chờ đối chiếu) không bị
  kiểm lại hay gửi lại.
- Hộp xác nhận Nuôi và Tương tác cũng hết hiệu lực khi phạm vi, thiết lập hoặc trạng thái
  roster của máy trong phạm vi đổi trong lúc hộp mở (máy vừa bận hoặc mất kết nối): không bắt
  đầu máy nào và báo phải kiểm tra lại.
- Không card lồng card, không hero marketing: tối đa hai tầng khung lồng nhau, viền chỉ ở ranh
  giới vùng. Bảng (`ResponsiveTable`) và thanh bước (`WorkflowStepper`) nằm trong panel nên chỉ kẻ
  đường trên/dưới, không tự vẽ khung; toolbar Thiết bị là một hàng trên nền làm việc với một
  đường kẻ dưới, không phải thẻ. Toolbar/bảng/section có kích thước ổn định.
- Toolbar: một hàng 40 px từ 1024 px (Thiết bị: Mở toàn bộ, Nhóm, Công cụ, Bảo trì, quét lại, rồi tìm,
  lọc trạng thái, Chọn tất cả, chế độ xem). Ô tìm không bao giờ hẹp hơn 176 px để đọc được từ đã gõ; chỗ
  cho nó lấy từ thao tác phụ: từ 1280 px có chữ; 1180–1279 px Nhóm/Công cụ/Bảo trì chỉ còn icon (giữ tên
  đọc được và tooltip); 1024–1179 px Nhóm, Công cụ, Bảo trì và Quét lại vào menu **⋯ Thêm thao tác thiết
  bị** có chữ (Escape đóng và trả focus về ⋯; hộp thoại mở từ menu trả focus về ⋯; đổi cỡ cửa sổ hoặc
  ghim/bỏ ghim qua mốc thì menu đóng, focus về Nhóm khi thao tác trở lại hàng, về ⋯ khi chúng vào menu). Nếu vẫn không đủ, phần lọc
  xuống hàng thứ hai chứ không co ô tìm. Nút chính giữ chữ. Toolbar không lặp số máy (đã ở thanh hệ thống). Dưới 1024 px
  phần lọc xuống hàng thứ hai; thông báo luôn chiếm một hàng riêng. Nút chính ≥ 40 px được giữ ở cả các
  footer/toolbar riêng (Nuôi, Đăng bài, Quy trình ứng dụng, Tác vụ đã lưu, Quản lý tài khoản).
- Không resize control theo label/hover/loading. Chữ wrap có chủ đích; không tràn, đè hàng sau hoặc scale theo viewport.
- Dùng khoảng cách/màu/icon nhất quán; disabled/loading/error/empty phải phân biệt và có hành động tiếp theo.
- Desktop/laptop và viewport hẹp giữ nút chạy, filter, status đọc được; overflow thuộc bảng/canvas phù hợp.
- Kiểm tra contrast, focus-visible, keyboard/dialog, tooltip gần mép, reload và chuyển trang khi chạy/bản nháp.

## Thuật ngữ trạng thái máy và phạm vi

Một bộ từ cho trạng thái máy (`deviceWork.ts`), ba trục; mỗi trục có **Chưa rõ** khi thiếu
bằng chứng. Thiếu dữ liệu không bao giờ tự thành Sẵn sàng hay Mất kết nối.

| Trục | Giá trị | Điều kiện |
|---|---|---|
| **Kết nối** | Đã kết nối · Mất kết nối · Chưa rõ | Mất kết nối = roster `disconnected`; trạng thái roster chưa nhận diện = Chưa rõ |
| **Điều khiển** | Sẵn sàng · Chưa kiểm · Không đạt (+ lý do) · Chưa rõ | Sẵn sàng chỉ khi có bằng chứng còn hiệu lực cho kết nối hiện tại: Android cần bản ghi agent `ready` + `authReady` + `sessionReady` + `helperReady` (bộ nhớ đệm, chỉ đọc trong Trung tâm điều khiển, đọc lại khi dòng roster đổi); iOS roster `ready` hoặc WDA sẵn sàng. Agent `unknown`/`starting` hoặc thiếu helper = Chưa kiểm; agent lỗi hoặc cần khôi phục helper = Không đạt; chưa đọc được bản ghi = Chưa rõ |
| **Công việc** | Rảnh · Đang chạy (+ việc) · Cần xử lý (+ lý do) · Chưa rõ | Đang chạy = có chủ sở hữu hiện tại hoặc roster `busy` (không nhận việc mới); Cần xử lý = điều khiển Không đạt, roster `error` hoặc máy quá tải; chưa đọc được chủ sở hữu = Chưa rõ |

**Chọn được** (Nuôi, Tương tác) / **có thể gán** (Đăng bài) là quy tắc riêng của từng workspace,
không phải một giá trị trạng thái; dòng trạng thái không cấp quyền chọn và không phải lời khẳng định
đã sẵn sàng. Máy Đang chạy không nhận việc mới; quy tắc chọn máy (gồm loại máy bận) thuộc §3.
Trong phạm vi Nuôi/Tương tác, máy roster `busy` mặc định **Bị loại** và không nhận lệnh; dòng phạm vi
ghi "N trong phạm vi · M thực hiện · K bị loại (Máy 4 – Đang chạy tác vụ, …)". Bị loại được giữ
nguyên khi máy rảnh lại; chỉ bỏ máy khỏi phạm vi rồi chọn lại mới đánh giá lại. Bắt đầu không bao
giờ dừng việc cũ của máy bị loại, và **chưa có thao tác giành máy bận** trên trang này: người vận
hành dừng việc cũ trong Lượt chạy. Máy vừa bận trong lúc hộp xác nhận mở làm hộp đó hết hiệu lực.
Nuôi chạy trên các máy còn lại; Tương tác chặn Bắt đầu tới khi bỏ từng máy bị loại, vì bỏ một máy
sẽ đổi cách chia bài.

Ô máy, dòng danh sách, thẻ chọn máy, chi tiết máy và Ctrl+K dùng một dòng gộp
(`deviceOperationalView` → `DeviceStatusLine`): Mất kết nối → Đang chạy → Cần xử lý → Chưa rõ →
Chưa kiểm → Sẵn sàng, kèm biểu tượng riêng cho từng trạng thái để đọc được khi không có màu.
Màu: Sẵn sàng xanh lá, Chưa kiểm xanh xám, Đang chạy xanh dương, Cần xử lý hổ phách, Mất kết nối
xám + icon rút cáp, Chưa rõ tím xám. Chi tiết máy hiện riêng ba trục và lời giải thích thường trực;
trang Trợ giúp có cùng lời giải thích. Không dùng "sẵn sàng" cho tập rộng hơn dòng gộp Sẵn sàng.
Thẻ chọn máy chỉ có dòng roster (không có chủ sở hữu, không có bằng chứng helper), nên máy Android
rảnh hiện **Chưa rõ: đã chuẩn bị** (hoặc chưa chuẩn bị; tooltip nói rõ đã kết nối), không bao giờ Sẵn sàng;
iPhone dùng chính bằng chứng WDA của dòng roster (`platform` + `wdaReady` truyền vào thẻ chọn), không
bị đọc như Android thiếu helper.

Thanh hệ thống luôn có bộ đếm **Tổng · Sẵn sàng · Chưa kiểm · Đang chạy · Cần xử lý · Mất kết nối ·
Chưa rõ**, mỗi số kèm mẫu số Tổng. Mỗi máy được đếm đúng một lần theo dòng gộp (các trục chồng
nhau, nên bộ đếm ghi rõ điều này trong tooltip), nên các số cộng lại bằng Tổng. Một số hiện **?**
khi máy Chưa rõ có thể thuộc nhóm đó (chưa đọc chủ sở hữu → Sẵn sàng/Chưa kiểm/Đang chạy/Cần xử lý;
chưa đọc bản ghi điều khiển → Sẵn sàng/Chưa kiểm/Cần xử lý); trước khi danh sách máy tải xong mọi
số đều **?**. Bấm một bộ đếm đặt đúng state lọc của select **Trạng thái thiết bị** trên lưới (cùng
bảy giá trị), bấm lại hoặc bấm **Tổng** thì bỏ lọc; từ trang khác thì chuyển về Trung tâm điều khiển.
Khi đang lọc, cạnh select có nhãn **Đang lọc: …** và nút **Bỏ lọc**. Bộ đếm là dòng đọc trạng
thái (icon trạng thái · nhãn · số), không viền; bộ đếm đang lọc có nền xám và chữ đậm, không cam.
Bộ đếm và bộ lọc dùng chung một state và cùng điều kiện của thẻ máy, không có bản sao thứ hai;
chân sidebar không còn khối "Kết nối · Sẵn sàng" trùng lặp (chỉ giữ Đã chọn trong lưới và Đồng bộ).

Mở và đóng **Chọn thiết bị**
mà không sửa thì giữ nguyên phạm vi Toàn bộ, Nhóm hoặc Máy cụ thể. Phạm vi chỉ thành Máy
cụ thể khi người vận hành sửa trong drawer. Phạm vi rỗng không bao giờ thành Toàn bộ;
nút chạy hiện lý do bị khóa.

Phạm vi, máy thực hiện và phân công là ba số khác nhau, mỗi số có nhãn riêng:

| Khái niệm | Nghĩa | Nhãn trên UI |
|---|---|---|
| **Phạm vi máy** | Toàn bộ, Nhóm hoặc Máy cụ thể; giới hạn máy nào được dùng | "N máy trong phạm vi" (Nuôi, Tương tác), "N trong phạm vi" (Đăng bài) |
| **Máy thực hiện** | Máy sẽ chạy lượt Tương tác: máy chọn trong phạm vi cộng máy được thêm bởi tag tài khoản | "N máy · K từ tag", "N máy thực hiện" |
| **Phân công** | Đăng bài ghép từng bài với một máy có thể gán; **Ghép tự động (chưa đăng)** và **Hoàn tác** nằm trong nhóm "Phân công", không đổi phạm vi và không đăng | "N/M bài đã gán" |

Nuôi chỉ có một trình sửa phạm vi: select **Phạm vi thiết bị** chọn kiểu, drawer **Chọn máy**
sửa danh sách; Nuôi không có drawer **Chọn thiết bị** thứ hai. Tương tác và Đăng bài giữ
**Chọn thiết bị** cho phạm vi, còn **Chọn máy** của Tương tác sửa máy thực hiện và tài khoản.

Test entrypoints: `apps/desktop/e2e/pages.spec.ts`, `typography.spec.ts`,
`flow-workspace.spec.ts`, unit của workspace và token tests. Snapshot mock chỉ xác nhận
layout/interaction fixture; smoke Tauri xác nhận renderer/backend thật ở phạm vi đã chạy.
