# Hướng dẫn vận hành

Riviu Manager quản lý điện thoại thật bằng một control plane. Mỗi thao tác phải có
phạm vi máy rõ ràng và kết quả có thể đọc lại. Chọn hồ sơ chỉ nạp cấu hình; không tự chạy.

## Quy tắc chung

- Kiểm tra số máy, nhóm, ứng dụng và tài khoản đang hiển thị trước khi chạy.
- Mỗi workspace automation giữ phạm vi riêng; đổi trang không biến một máy thành toàn bộ fleet.
- Flow thiết bị/Điều phối giữ thao tác `Lưu`, `Bỏ thay đổi`, `Ở lại`. Các tab còn lại tự lưu thiết lập sau khi ngừng nhập và trước khi chuyển tab/đóng app; lỗi lưu xuất hiện tại vùng đang sửa.
- Tự lưu bài/máy/caption không tạo chiến dịch, không bật lịch và không cấp quyền đăng. Hồ sơ có tên vẫn được lưu riêng khi cần ghim revision cho Flow. Credential đi qua kho thông tin xác thực hiện có.
- `queued` còn đang chờ; `running` đã bắt đầu; `uncertain` cần đối chiếu bằng chứng. Không thử lại thao tác công khai chỉ vì thiếu ACK.
- Trạng thái ngắn hạn nằm trong vùng hoạt động; lịch sử bền nằm ở Tác vụ/Dữ liệu và monitor nguồn.
- Mất thiết bị hoặc lỗi quyền phải hiện lỗi ở vùng liên quan. Không dùng ảnh preview còn lưu làm bằng chứng máy đang sẵn sàng.

## Thiết bị

Thanh **Tiến trình công việc** nằm trong cửa sổ nổi ở góc trên bên phải khi có tác vụ đang chạy
hoặc vừa kết thúc. Cửa sổ nằm trên các trang, không chiếm diện tích layout; kéo tiêu đề để đổi vị trí. Bấm mở rồi chọn số máy/alias để xem kết quả và nhật ký
`giờ:phút:giây`. Chuyển trang vẫn giữ monitor. 100% là đã xử lý xong, không có nghĩa
tất cả đều thành công; đọc trạng thái lỗi/chưa xác nhận bên cạnh. Dữ liệu thiếu giờ
không được tự gán giờ hiện tại. Các tác vụ cũ hơn xem tại **Tác vụ**.
Giữ và kéo trên thanh tiêu đề để di chuyển; bấm nhẹ mở/thu gọn, nút dấu trừ thu nhỏ
cửa sổ. Nút thùng rác
xoá bản ghi đã kết thúc khỏi cửa sổ theo dõi, không xoá lịch sử/bằng chứng trong
**Tác vụ** và không dừng công việc. **Hoàn tác xoá** khôi phục lượt xoá gần nhất.
Trong cửa sổ, chọn tác vụ theo tên và thời gian; tìm máy hoặc lọc **Cần kiểm tra**.
Danh sách máy chỉ báo tiến độ phần trăm khi còn chạy; máy đã kết thúc có trạng thái
riêng. Tab **Nhật ký** hiển thị hoạt động theo giờ, tab **Bằng chứng** chứa kết quả
xác minh. Dòng lặp giữ số lần và thời gian cuối; nút thông tin bên phải mở bản ghi gốc.
Nút **Phóng rộng tiến trình** mở rộng vùng đọc; **Khôi phục kích thước** trở lại cửa sổ
nhỏ. Thu nhỏ giữ máy/tab/vị trí đang đọc. Nhật ký mặc định mới nhất trước, đổi thứ tự
bằng nút cạnh số mốc. Trên màn hẹp, chọn máy mở chi tiết toàn chiều rộng và dùng mũi
tên quay lại danh sách. Menu ba chấm có lệnh dọn các bản ghi đã kết thúc.
Mặc định cửa sổ gọn: mỗi máy một dòng, chọn máy mới mở nhật ký. Thu nhỏ còn thanh
300px; nút phóng rộng mới bật bảng hai cột trên màn hình lớn.

Trong cửa sổ điều khiển, nút hình điện thoại **Đưa về màn hình dọc** yêu cầu máy đổi
về dọc và đọc lại kết quả. Máy đang ngang vẫn giữ đúng tỉ lệ ảnh và menu cuộn riêng;
không cần đóng/mở cửa sổ để bố cục theo hướng mới.

**Đầu vào:** kết nối USB, nhóm, bộ lọc trạng thái, từ khoá và các máy được chọn.
Danh sách/lưới dùng cùng tập sau lọc; các ô chia đều chiều ngang, giữ tỷ lệ màn hình và
cử chỉ điều khiển. Ctrl+lăn chuột đổi mức zoom; hàng cuối giữ cùng kích thước ô.

**Thao tác:** làm mới roster; chọn nhóm; chọn máy; mở máy hoặc drawer chi tiết; thao tác
hàng loạt qua menu ngữ cảnh. Đọc số máy đích ngay tại lệnh. Dùng Chẩn đoán khi trạng thái
quyền/transport khác trạng thái kết nối.

Ba tab **Nuôi TikTok / Tương tác / Đăng bài** mở khung tác vụ cạnh lưới stream.
Chọn máy rồi bấm **Dùng N máy đã chọn**, hoặc mở **Phạm vi thiết bị** để chọn nhóm.
Phạm vi tác vụ không tự đổi khi bỏ chọn/lọc lưới. Khung có đủ Thiết lập và Theo dõi;
nút **Mở trang tác vụ** mở rộng workspace; để trở lại cạnh lưới, mở **Thiết bị** rồi chọn tab tác vụ
mà giữ bản nháp. Đóng/đổi tác vụ vẫn hỏi lưu nếu đã sửa.

**Kết quả:** roster, trạng thái work owner, preview và tiến độ từng máy. Máy rời fleet
đóng các vùng thao tác của chính máy đó. **Tiếp theo:** mở máy cần kiểm tra hoặc chuyển
đến automation với phạm vi đã review; không coi đang hiển thị preview là đã pass action.

## Chẩn đoán

**Đầu vào:** thiết bị và nhóm cần kiểm tra. **Thao tác:** đọc transport, helper, quyền và
bằng chứng foreground/stream; làm mới phép đo có chủ đích. **Kết quả:** từng điều kiện
sẵn sàng, lỗi cụ thể và bằng chứng đo được. **Tiếp theo:** sửa đúng điều kiện thất bại,
rồi kiểm tra lại. Repair/cài helper là lệnh riêng, không suy ra từ lỗi preview thoáng qua.

## Nuôi TikTok

Giao diện dev hiện dùng các tab ngang **Thiết lập · Hẹn giờ · Theo dõi**.
Ô **Tổng số video muốn lướt** nằm ngay đầu Thiết lập và là tổng cho mỗi máy;
phiên dừng khi đủ số video hoặc hết thời lượng. Nhập tổng mới sẽ dùng một vòng,
còn hồ sơ nhiều vòng vẫn hiển thị tổng đã nhân để dễ kiểm tra.
Trang chỉ có một ô nhập tổng video; phần Hành vi giữ các tùy chỉnh nhịp xem, không
hiển thị lại giới hạn/vòng hay các tỷ lệ tương tác đã có ở thiết lập phiên.

**Máy thực hiện** hiển thị tất cả máy trong một khung **hai cột** có ô tick, không phân trang.
**Chọn tất cả** chọn các máy sẵn sàng trong danh sách, kể cả ngoài kết quả tìm kiếm;
**Bỏ chọn** xóa lựa chọn. Máy bận/chưa sẵn sàng không được chọn thêm. Ô máy tự gọn
khi danh sách đông; cuộn trong khung khi cần, số máy luôn giữ nguyên khi tìm kiếm.
Ô **Phạm vi thiết bị** nằm cùng hàng Chọn tất cả/Bỏ chọn để chọn nhóm hoặc toàn bộ.
Khung máy đã thu hẹp, phần cấu hình phiên có thêm chỗ hiển thị.
Header ghi **Đã chọn X/Y**; tên/model mỗi máy hiện một lần. Cuộn trong danh sách
máy hoặc phần cấu hình độc lập, thanh tab và nút bắt đầu giữ ở vị trí cố định.

**Thiết lập** chứa cấu hình phiên, Hành vi, AI và Bình luận. Tab **Hẹn giờ** chứa lịch
tự chạy và các khung giờ riêng; bấm **Áp dụng hẹn giờ** để lưu lịch xuống ứng dụng.
Chuyển tab hoặc sửa bản nháp chưa bắt đầu phiên; **Kiểm tra & bắt đầu** vẫn là
thao tác riêng. Giữ máy tính và Riviu đang mở để lịch chạy.

Trang Thiết lập đặt cấu hình phiên cạnh danh sách máy. Chọn Nhẹ nhàng/Cân bằng
hoặc chỉnh từng tỷ lệ, chọn máy rồi bấm **Kiểm tra & bắt đầu**. Thời lượng trên
trang được gửi vào phiên thật; Nhẹ nhàng là 15 phút, Cân bằng 20 phút. Phần AI, nhịp
và cấu hình nằm trong **Thiết lập**, lịch nằm trong **Hẹn giờ**. Theo dõi có log riêng từng máy.

**Đầu vào:** phạm vi máy, thời gian xem, giới hạn phiên, nhịp, hành động và lịch.
Credential AI được lưu riêng.

**Thao tác:** sửa và lưu thiết lập hiện tại; lưu phần credential bằng vùng lưu riêng; kiểm tra cấu
hình và readiness; chạy hoặc lên lịch; chuyển sang Theo dõi để xem tiến độ từng máy.
Trần video là giới hạn trên, không phải mục tiêu bắt buộc.

**Kết quả:** phiên, số card quan sát, effect/bằng chứng và kết thúc từng máy. **Tiếp theo:**
đọc lý do partial/uncertain trước khi tạo phiên mới; không bù số lượng bằng cách lặp
comment. Dừng phiên không chứng minh các effect đã gửi được hoàn tác.
Từ0.2.21, **Lưu thiết lập** lưu trực tiếp; **Lưu lịch từ thiết lập** chụp cấu hình và
phạm vi cho lịch mới. Lịch cũ vẫn bật/tắt, đổi tên và chu kỳ được; sửa thiết lập
hiện tại không đổi cấu hình của lịch đã lưu.

Kết thúc Nuôi TikTok chỉ tắt ứng dụng trên các máy đã được nhận vào phiên.
Nếu báo `16/20 máy đã bắt đầu`, bốn máy còn lại chưa được xử lý và không bị tự tắt.
Xem danh sách máy không bắt đầu và nhật ký `nurture.start.skipped`; số **TikTok đã tắt**
chỉ tính các máy trong phiên có chứng cứ tiến trình, không tính mọi máy đang trên lưới.

Khi bắt đầu phiên mới, Nuôi/Tương tác/Đăng bài lấy quyền sử dụng máy và tắt riêng TikTok
có kiểm chứng trước khi mở lại. Chỉ mở tab không làm việc này. Không xóa dữ liệu/cache,
không đăng xuất, không tắt app khác. Kết thúc công việc thì tắt TikTok và đóng stream;
riêng bài đã bấm Đăng nhưng chưa xác minh được liên kết giữ TikTok chạy và giữ nội dung
đã chuyển để quá trình tải tiếp tục. Máy còn bài chờ xác minh chưa bắt đầu phiên tự động
mới có bước tắt TikTok. Lỗi dọn được báo riêng. Lượt đã qua Send/Post mà chưa rõ kết quả
không tự gửi/đăng lại; việc kiểm tra lại chỉ đi theo phạm vi đã được ghi nhận.

## Tương tác

Các tab chính là **Thiết lập · Hẹn giờ · Theo dõi**; cấu hình nằm trong
Thiết lập, Hẹn giờ lưu bản chụp thiết lập hiện tại. Các phần Chọn bài viết/Hành động & máy/
Kiểm tra & chạy dùng tab ngang; nút chạy chỉ bật khi đã đủ điều kiện.

Khung **Máy thực hiện** dùng cùng ô chọn hai cột như Nuôi: tìm máy, Chọn tất cả,
Bỏ chọn và cuộn trong một khung. Tài khoản vẫn mở từ dòng dưới tên máy. Máy được
thêm bởi tag chỉ gỡ khi sửa tag; nút Bỏ chọn bỏ các máy được tick trực tiếp.

Ba bước trên trang là **Chọn bài viết → Hành động & máy → Kiểm tra & chạy**.
Tắt Bình luận thì chỉ cần chọn Tim/Lưu và máy; phần AI được ẩn. Theo dõi hiển thị
bảng kết quả theo máy; **Xem log** mở bằng chứng và thao tác xử lý của đúng máy.

**Đầu vào:** URL bài, hành động, nội dung/AI và máy thực hiện. **Thao tác:** parse
đúng chuỗi URL hiện tại; sửa lỗi parse trước khi chạy; review assignment và số bài/số máy;
chạy rồi theo dõi kết quả từng hành động.

`Riêng lẻ` cho phép một máy và một bình luận; kiểu chuỗi vẫn cần ít nhất hai máy.
Tim/Lưu và bình luận thủ công không phụ thuộc cấu hình AI. “Bỏ qua: chưa đọc được
trạng thái” không có nghĩa đã Tim/Lưu; xem lý do và bằng chứng trước khi chạy lượt mới.
Nếu tên hiển thị khác handle, ứng dụng có thể đối chiếu link từ chính bài trước hành động.

Ô tài khoản cạnh từng máy nhận **username TikTok**, ví dụ `@ten.nick`, không phải
tên hiển thị. Tên/số máy vẫn giữ riêng; nick được gắn với định danh máy. Rời ô sẽ lưu;
nếu báo lỗi, sửa hoặc bấm **Tải lại nick đã lưu** trước khi chạy. Không gán cùng nick
cho hai máy để tránh nhầm actor khi tag. Nick lưu trong Riviu chưa chứng minh máy
đang đăng nhập tài khoản đó; sau khi đổi tài khoản trên TikTok cần đối chiếu lại.

**Đọc tài khoản từ máy** mở Hồ sơ, đối chiếu nick đã gán với nick quan sát được,
báo Khớp/Lệch/Chưa đọc được và thời điểm. Không tự đổi tài khoản hoặc ghi đè nick.
Hiện đã đo `trill 38.3.2/en`; bản/ngôn ngữ khác cần hiệu chỉnh trước.

**Nhập từ Google Sheet:** dán link đúng tab, chọn cột chứa link, bấm Đọc Sheet.
Chọn các dòng hợp lệ rồi Thêm bài đã chọn. Link trùng/lỗi có trạng thái riêng; đọc
Sheet không chạy chiến dịch. Xem bảng **Phân công bài và máy** trước khi xác nhận.
Nguồn phải đọc được bằng quyền xem liên kết; Riviu không ghi ngược Sheet ở bước này.

Với lượt **Chưa chắc kết quả**, **Kiểm tra lại kết quả** chỉ mở đúng bài để đọc lại
Tim/Lưu. Không gửi lại, không thay lịch sử uncertain. Bình luận chưa được kiểm lại
bằng nút này; giữ bằng chứng đã gửi và không tự retry vì thiếu kết quả readback.

**Kết quả:** campaign, assignment, prepared content và trạng thái effect. Đổi URL rồi
parse lỗi không được dùng target cũ. **Tiếp theo:** mở bằng chứng cho uncertain; retry chỉ
phần được hệ thống xác định còn hợp lệ, không gửi lại một comment chỉ vì nhận ACK không rõ.

## Đăng bài

Các tab chính là **Thiết lập · Hẹn giờ · Theo dõi**. Thiết lập mở **Bàn đăng nhanh**:
nội dung, caption/đối tác và máy thực hiện nằm trên cùng màn hình. Ở cửa sổ nhỏ,
cuộn trong vùng làm việc để xem phần dưới; nút **Kiểm tra & đăng** ở cuối vẫn giữ vị trí.
Thanh đầu đặt **Chọn thư mục** cạnh **Quét**, cùng ô **Link Google Sheet** và nút
**Kết nối Sheet**. Đăng bài không còn phần hồ sơ hoặc tab cài đặt riêng. Nhập link Sheet
rồi kiểm tra; đọc được bảng chưa đồng nghĩa đã xác minh đúng kết nối ghi kết quả.
Nút này tự tạo tiêu đề nếu tab hoàn toàn trống; không thêm hàng thử. Kết quả chỉ
xanh khi kết nối ghi xác nhận đúng bảng và tab. Bản nội bộ0.2.21 mang cấu hình
Sheet chung, máy mới tự nạp kết nối khi chưa có cấu hình. Bảng đã có dữ liệu giữ nguyên.
Khi đã nhập link, cần xác minh thành công trước khi đăng hoặc lưu lịch có bật ghi
Sheet. Có thể tắt **Ghi kết quả lên Sheet** để chạy lượt không ghi bảng.

Khung **Máy thực hiện** có nút **Chọn nhanh**: có bài/máy đã chọn thì dùng tập đó;
chưa chọn thì lấy bài trong nguồn (tối đa100) và máy sẵn sàng trong phạm vi.
Mỗi máy nhận một bài; các cặp đã ghép được giữ. Thiếu máy vẫn ghép phần đủ và báo
số bài còn thiếu. **Hoàn tác** trả lại lần gán nhanh gần nhất nếu chưa sửa ánh xạ.
Bỏ tick máy gỡ bài của máy đó; **Máy nhận bài đang chỉnh** cho đổi riêng từng bài.
Dòng tổng kết và hộp kiểm tra ghi rõ Sheet đang bật hay tắt. Có thể Hủy khi đang
chuyển nội dung trước Đăng. Theo dõi tự đọc lại kết quả mỗi5giây khi đang mở.
Sau Đăng, Theo dõi hiển thị lý do chưa xác minh, lần kiểm gần nhất và lần kiểm
kế tiếp. App giữ TikTok/media khi chờ; link xác minh xong được gửi Sheet ngay.

Bản 0.2.16 kiểm tra phone có đang bị một máy tính khác điều khiển ADB qua mạng
trước khi chuẩn bị đăng. Nếu thấy địa chỉ máy phụ, ngắt kết nối phone ở máy phụ;
phone đang cắm cáp có thể chuyển sang ADB USB. Wi-Fi Internet vẫn dùng để tải bài.
Các máy mới cần đăng nhập TikTok, giờ hệ thống đúng và mạng truy cập được TikTok;
chỉ có biểu tượng Wi-Fi chưa đủ để tải danh sách nhạc.

**Hẹn giờ** dùng nguồn đã quét trong Thiết lập. Khi chưa có bản nháp lịch, những
bài và cặp bài–máy đã chọn được mang sang. Bản nháp hiện có được giữ khi chuyển tab.
Ngày/giờ nằm trên cùng, danh sách bài bên trái, máy nhận và bảng phân công bên phải.
Trong cửa sổ thấp, cuộn vùng nội dung để xem bảng; ngày/giờ và nút lưu vẫn cố định.
**Chọn nhanh** nằm đầu vùng máy: bấm một lần để tự gán bài. Có bài đã chọn thì
dùng tập đó; chưa chọn bài thì lấy tối đa 100 bài trong nguồn. Có máy đã tick thì
chỉ dùng tập đó; chưa tick máy thì lấy máy sẵn sàng trong phạm vi. Cặp đã gán giữ
nguyên, máy dư không nhận bài; thiếu máy hiện số bài còn thiếu. Bấm lại không
đảo cặp, **Hoàn tác** phục hồi toàn bộ lần chọn nhanh.
Tick nhiều bài bên trái và máy bên phải. Kéo một bài trong nhóm đã tick vào vùng
máy để phân lần lượt theo thứ tự bài và số máy; kéo một bài chưa tick chỉ gán bài đó.
Thả trực tiếp lên máy trống chỉ chuyển đúng bài đang kéo, kể cả khi đã chọn cả
nhóm và máy đích chưa tick. Thả vào vùng chung mới phân nhóm đang chọn.
Máy đã nhận bài cùng giờ giữ nguyên; phần thiếu chỗ nằm ở **Chưa có máy**. Kéo tiếp
chỉ lấp chỗ trống. Có thể dùng bàn phím chọn bài/máy và bấm **Gán bài đã chọn**.

Chọn **Ngày đăng** và **Giờ chung** một lần cho cả lượt. **Chỉnh giờ từng bài**
cho đặt giờ khác; đổi giờ chung không sửa giờ riêng. Một máy có thể nhận nhiều bài
khác giờ. Bảng **Bài → Máy → Giờ** cho đổi máy, gỡ gán hoặc bỏ bài. **Hoàn tác**
khôi phục lần chỉnh gần nhất; bỏ chọn máy chỉ thay tập máy cho lần tự gán tiếp theo.
Bấm **Kiểm tra lịch**, đọc kết quả từng dòng, xác nhận công khai rồi **Lưu lịch N bài**.
Kéo thả và sửa giờ chỉ lưu bản nháp; mỗi lịch tối đa 100 bài khác nhau.
Rê hoặc focus một bài đánh dấu máy và dòng phân công tương ứng. Thanh cuối chỉ
ra bước còn thiếu và có đường dẫn tới trường cần sửa. Lỗi lưu nháp giữ ở dòng
riêng cùng **Thử lưu nháp lại**, không bị thông báo đã gán bài che mất.

Lịch đã lưu xuất hiện từng lượt trong **Theo dõi**, kèm ngày giờ và nút hủy.
Giờ dùng múi giờ máy tính và là thời điểm bắt đầu xử lý, không phải thời điểm
TikTok hoàn tất tải bài. Giữ Riviu và máy tính đang chạy, điện thoại kết nối mạng.
Mở lại app sau giờ hẹn sẽ đánh dấu lượt đó **Lỡ lịch**, không tự đăng bù.
Muốn ngày khác, tạo một lịch mới với ngày khác; tính năng này không tự lặp hằng ngày.

Cách chạy một lượt trong Bàn đăng nhanh:

1. Bấm **Chọn thư mục**, rồi **Quét**; đánh dấu từng bài hoặc **Chọn tất cả bài**.
   Một thư mục con là một bài gồm toàn bộ ảnh; có thể chọn trực tiếp thư mục của một bài.
   Ô Thư mục nguồn cho nhập đường dẫn và **Quét**. Đổi nguồn trong lúc quét sẽ bỏ kết quả
   trả muộn của nguồn cũ. Quét xong chưa tự chọn bài hoặc tạo chiến dịch.
2. Bấm một bài để xem caption, ảnh/video và đối tác trong **Bài đang chỉnh**. Sửa trực tiếp
   **Nội dung bài đăng**; bản nháp giữ thay đổi, file nguồn giữ nguyên. **Phóng to ảnh**
   ở cạnh ảnh đại diện cho chuyển ảnh; Escape trở về. Chọn ảnh xem trước chỉ đổi
   ảnh đang xem, không bớt ảnh khỏi bài.
3. Chọn máy và bấm **Ghép bài với máy đã chọn**. Dòng cuối phải đủ `N/N bài có máy`.
   Chọn 10 bài chỉ cần 10 máy dù đang kết nối 20 máy. Nếu chọn riêng ít hơn số bài,
   thêm máy hoặc giảm số bài rồi ghép lại.
4. Chọn **Ghi kết quả lên Sheet** và giữ/xóa bản chuyển, rồi bấm **Kiểm tra & đăng**.
   Đọc kết quả từng máy, bấm **Xác nhận đăng N bài** và xác nhận công khai. Nhạc được
   chọn và đọc lại trong TikTok sau khi mở máy; kiểm tra đầu vào chưa có nghĩa đã chọn
   nhạc. Các máy có thể dùng trùng nhạc. Bài chưa xác minh giữ nội dung trên điện thoại;
   trạng thái dọn được hiển thị riêng trong chi tiết.

Rời trang hoặc mở lại app khôi phục bài, caption và ghép máy đã lưu trong bản nháp.
Kết quả kiểm tra và quyền xác nhận đăng không được khôi phục; phải kiểm tra lại trước lượt mới.

**Theo dõi** có bộ lọc **Tất cả / Đang chạy / Cần xử lý / Hoàn tất** và ô tìm nguồn
hoặc ngày đăng. Chọn chiến dịch ở danh sách để xem kết quả từng máy bên cạnh;
nút kiểm tra liên kết, ghi lại Sheet hoặc hủy nằm trong chi tiết của lượt đã chọn.
Chuyển về tab **Thiết lập** giữ nguyên bài và máy đang ghép.

**Đầu vào:** một MP4 H.264/AAC hoặc 1–35 ảnh, caption, âm nhạc, Sheet và máy được
gán cho từng bài. **Thao tác:** preflight trước dispatch; đối chiếu một bài với một máy
đã gán và toàn bộ hậu quả cleanup; chạy pipeline rồi xem projection trong Theo dõi.

**Kết quả:** bằng chứng Post, URL, nhạc, Sheet và cleanup riêng biệt. **Đã bấm Đăng · chờ xác minh**
nghĩa TikTok đã nhận thao tác nhưng Riviu chưa xác nhận bài đã xuất bản. Về bảng tin
không phải bằng chứng tải xong. Riviu giữ TikTok chạy, chờ máy rảnh rồi tự kiểm tra liên kết;
quá trình tiếp tục khi mở lại app và kết nối lại máy. Nút **Kiểm tra liên kết** cho kiểm tra
sớm, chỉ lấy liên kết/ghi Sheet, không đăng lại bài hay đăng các bài còn lại trong lượt đó.
Giữ Riviu chạy, máy tính không ngủ và điện thoại có mạng. Không có thời gian hoàn tất cố định
vì TikTok còn xử lý/xét duyệt. **Chưa chắc chắn** giữ bằng chứng riêng và không cho đăng lại.

**Hoàn tất** chỉ xuất hiện khi có liên kết đã xác minh và Sheet đã xác nhận nếu bật ghi.
Còn nợ Sheet thì hiển thị **Hoàn tất một phần**, với **Ghi lại Sheet** khi phù hợp.
Máy chờ xác minh không được tính vào số máy hoàn tất hoặc hiển thị tiến độ 100%.
`Ghi kết quả lên Sheet` là lựa chọn riêng cho từng chiến dịch: tắt thì không
tạo hàng chờ hoặc gửi webhook; bật mà chưa cấu hình thì giữ link trong outbox pending.
Đổi lựa chọn phải chạy preflight lại. Nhạc được chọn ngẫu nhiên có seed từ tối đa
năm đề xuất/thịnh hành đang hiện trên tài khoản, không lấy danh sách ngoài TikTok.

Nếu bước kiểm tra báo chưa hỗ trợ, xem **phiên bản TikTok và ngôn ngữ của từng máy**.
Chọn một máy đã hỗ trợ hoặc cung cấp thông tin đó để hiệu chỉnh; không suy rằng cài
Riviu thành công đồng nghĩa mọi phiên bản TikTok đều đăng được. Máy cài cả TikTok
quốc tế và TikTok châu Á cần mở đúng ứng dụng muốn dùng rồi kiểm tra lại. Kết quả
đạt ở bước này chỉ xác nhận điều kiện đầu vào; nhạc vẫn được chọn và đọc lại trong
luồng đăng. Thông báo gốc và mã máy nằm trong **Chi tiết kỹ thuật**.

Bản 0.2.9 bổ sung luồng ảnh/chọn nhạc cho TikTok quốc tế **46.0.41** giao diện
tiếng Anh. Nếu TikTok hiện “No internet connection” hoặc nhạc chỉ quay Loading,
kiểm tra Internet ngay trên điện thoại; Wi-Fi đã kết nối chưa chứng minh tải được
nhạc. Các phiên bản chưa có bộ nhận diện vẫn được báo ở bước kiểm tra từng máy.

Source cập nhật §9.184 bổ sung **45.7.3 tiếng Anh**, gồm chọn nhiều ảnh/nhạc,
đọc tài khoản và nhận diện Send. Bộ cài0.2.9 đã giao trước đợt này chưa chứa các
thay đổi mới. Preview nguồn có nút phóngto, chuyểnảnh và Escape quaylại; caption,
sốảnh và dunglượng nằm ngay hàng bài. Chứng nhận Post công khai vẫn theo từnglượt.

**0.2.14 — box mới:** hỗ trợ ảnh/video H.264, chọn nhạc và đọc tài khoản cho
TikTok quốc tế tiếng Anh **45.4.3, 45.7.3, 46.0.41, 46.1.3, 46.2.1, 46.4.3**;
TikTok Trill **38.3.2 tiếng Anh** giữ hỗ trợ đã có. Các bản khác/ngôn ngữ khác
vẫn cần kiểm tra riêng. Máy có cả hai TikTok cần mở đúng ứng dụng trước preflight;
máy chưa cài hoặc chưa đăng nhập cần hoàn tất bước đó. Nhạc cần Internet thật.
Riviu Helper đã triển khai cho30 phone được nhận ở đợt §9.189. Lấy liên kết bài
Global còn phụ thuộc mapping riêng, không suy từ Đã đăng sang Đã lấy link/Sheet.

**Tiếp theo:** retry chỉ phạm vi metadata/outbox
còn thiếu; không mở lại full pipeline cho bài đã đăng. Cleanup là tập effect cụ thể,
không phải xoá tùy ý theo tên thư mục.

## Flow

**Đầu vào:** graph thiết bị hoặc điều phối fleet, cấu hình node, target và profile.
**Thao tác:** chọn đúng chế độ; sửa graph; validate; lưu trước khi chạy; theo dõi execution
và từng node. Import/export JSON là thao tác bản nháp có guard; archive phải phản ánh
đúng identity đang mở.

**Kết quả:** execution history, node outcomes và artifact nguồn. **Tiếp theo:** mở đúng
run để xem lỗi; retry chỉ theo contract của effect đó. Lịch sử execution không cho phép
phát lại mù một node có tác dụng ngoài hệ thống.

## Tác vụ

**Đầu vào:** nguồn, trạng thái, khoảng thời gian và trang kết quả. **Thao tác:** lọc trước
khi đọc detail, mở source monitor và bằng chứng. Số bài và số máy là hai đại lượng riêng.

**Kết quả:** tổng đếm trước phân trang và projection từ nguồn bền. Mở nguồn dùng đúng
source/run/item ID lịch sử, không mở batch mới nhất theo suy đoán và không phát lệnh chạy.
Mặc định xem 24 giờ,
active cũ vẫn nổi lên. **Tiếp theo:** thu hẹp phạm vi nếu nguồn quá lớn; hủy/retry tại
source có đủ contract, không suy ra kết quả từ danh sách bị cắt.

## Kho nội dung

**Đầu vào:** file nội dung, metadata, tập máy review. Phạm vi rỗng không tự trở thành
All; chọn rõ máy hoặc nhóm trước khi dispatch. **Thao tác:** nhập/xem bảng nội
dung; chọn artifact và thiết bị; xác nhận batch; đọc tiến độ, kể cả sau đổi trang/reload.

**Kết quả:** ledger ghi artifact snapshot và từng máy trước dispatch. Sau restart,
queued được hủy, running trở thành uncertain, terminal giữ nguyên. **Tiếp theo:** chỉ
hủy item còn queued; đối chiếu uncertain trực tiếp, không retry tự động.

## Trung tâm ứng dụng

**Đầu vào:** package/artifact và thiết bị đích được chọn rõ; không có máy chọn không
có nghĩa toàn bộ fleet. **Thao tác:** review package, phiên bản,
máy và lệnh cài/chuyển; chạy batch rồi đọc outcome từng máy. **Kết quả:** ledger và
tiến độ khôi phục được khi quay lại trang. **Tiếp theo:** kiểm tra package/version thật
khi uncertain; kết quả installer/ACK không tự chứng minh trạng thái sau restart.

## Dữ liệu

**Đầu vào:** nguồn lịch sử, khoảng ngày và bộ lọc. **Thao tác:** xem bản ghi, tổng và
detail; theo liên kết artifact/source. **Kết quả:** dữ liệu bền có nguồn gốc, không phải
bản sao toast. **Tiếp theo:** quay lại workspace gốc để xử lý; thu hẹp query khi giới
hạn nguồn được báo, không coi partial list là toàn bộ dữ liệu.

## API

**Đầu vào:** cấu hình listener, credential và địa chỉ client. **Thao tác:** lưu cấu hình
đúng vùng; đọc địa chỉ listener đang chạy, lỗi bind và yêu cầu restart riêng biệt.
**Kết quả:** trạng thái runtime, không chỉ giá trị trong form. **Tiếp theo:** xử lý port
bị chiếm hoặc restart theo chỉ báo; kiểm tra gọi API qua cùng admission/ownership với UI.

## Cài đặt

**Đầu vào:** giá trị của từng section và credential tương ứng. **Thao tác:** chỉnh sửa,
lưu/bỏ từng section; không coi text vừa nhập là đã persist. **Kết quả:** save status và
readback đúng section; phản hồi cũ không ghi đè bản nháp mới. **Tiếp theo:** áp restart
nếu thay đổi yêu cầu, hoặc quay lại vùng vừa sửa sau khi guard được giải quyết.

## Khi có lỗi

Ghi workspace, hồ sơ/revision, số/alias máy, thời điểm và execution ID. Mở bằng chứng
nguồn trước khi lặp lệnh. Không ghi token/password vào report. Khi kiểm tra WDA/iOS,
đọc [ràng buộc §2](agents/02-wda-doc-truoc-khi-sua.md) và không chạy harness đồng thời
với desktop đang sở hữu thiết bị. Hướng dẫn này mô tả hợp đồng; nghiệm thu có ngày ở
[nhật ký](agents/README.md), không tự cấp chứng nhận cho mọi thiết bị/bản cài.
