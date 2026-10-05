# Đăng bài

[Hướng dẫn vận hành](../operator-guide.md) · Chi tiết chức năng hiện hành; bảng khả năng và proof trên máy quyết định quyền chạy.

## Đăng bài

Mục **Giới hạn chạy đồng thời** trên trang Đăng bài cho phép xem và lưu giới hạn
chuyển media, thao tác TikTok, xác minh link và tổng lượt điều khiển theo máy chủ.
Sau khi đổi giới hạn, kiểm tra lại lịch trước khi lưu; lịch quá tải có cảnh báo.

Đăng ngay và hẹn giờ dùng chung hàng chờ bền vững theo từng bài/máy. Mặc định toàn
ứng dụng cho phép 64 lượt chuyển media, 4 lượt thao tác TikTok và 4 lượt xác minh
liên kết, với tổng tối đa 64 máy; mỗi máy chỉ có một chủ điều khiển. Cấu hình giới hạn đã lưu trên PC được
giữ nguyên: chọn **Chạy song song toàn bộ máy** rồi **Lưu giới hạn** để mở toàn bộ
lượt trên PC đang dùng giới hạn cũ. Lệnh ADB vẫn có giới hạn I/O chung theo bus USB.
Sheet có tối đa 2 request, trong đó tối đa
1 request báo tiến độ. Theo dõi hiển thị giai đoạn, thời điểm vào hàng và lý do chờ.
Sau khi gửi, app trả lượt thao tác TikTok rồi xác minh riêng; retry trước gửi giữ
nguyên bài dự kiến, retry lấy link hoặc ghi Sheet không gửi bài lần nữa.

Khi đã lưu liên kết ứng viên đúng lượt đăng nhưng dữ liệu công khai của TikTok
chưa sẵn sàng, app dùng lại link và tự kiểm tra qua mạng mỗi 5 phút, tối đa 12 lượt
trong một đợt xác minh. Giai đoạn này không mở lại điện thoại để Copy. Link chỉ
được ghi Sheet sau khi xác minh đúng tài khoản, nội dung và thời điểm bài đăng.
Lỗi tìm bài hoặc thao tác điện thoại vẫn dừng sau ba lượt không có bằng chứng mới.
Hết giới hạn riêng của metadata, chọn **Tiếp tục xác minh bài đã gửi** để mở đợt
kiểm tra mới. Link và định danh bài cũ được giữ nguyên; thao tác Đăng không lặp lại.

Lỗi `copy <tệp ảnh>` nằm ở bước sao chép nội dung trên PC vào thư mục quản lý,
trước chuyển qua ADB. Thông báo gồm tệp nguồn, tệp đích và lỗi hệ điều hành. Kiểm
tra tệp còn tồn tại, quyền đọc Downloads và dung lượng/quyền ghi ổ chứa dữ liệu Riviu.
Không dùng bấm Đăng lại để giải quyết lỗi sao chép sau một kết quả gửi chưa rõ.

Trước khi chọn ảnh trên Android, Riviu đọc lại album đã tải, đối chiếu tên, dung
lượng và hash với nội dung đã chốt. Nếu thứ tự thời gian ảnh bị lệch, ứng dụng
sửa metadata của đúng album rồi đọc xác nhận; không tải ảnh hoặc tạo album lại.
Kiểm tra này áp dụng cả lượt mới và lượt thử lại trước Đăng. Nếu ảnh thiếu, thừa
hoặc byte ảnh đã đổi, lượt đăng dừng trước bộ chọn ảnh để người vận hành kiểm tra.
Bài đã gửi và lượt chưa rõ kết quả không được đăng lại để sửa thứ tự.

Giờ hẹn là lúc ứng dụng nhận đợt đăng vào hàng chờ. Đợt được nhận trong 30 giây
được xử lý theo giới hạn đã lưu; các máy còn chờ lượt không bị ghi Lỡ lịch chỉ vì
hết lượt chạy đồng thời. Đợt chưa được nhận trong cửa sổ đó hoặc app mở lại sau
giờ hẹn được ghi **Lỡ lịch**, không tự đăng bù. Bài đã gửi vẫn được xác minh.
Lịch đông máy có thể bắt đầu thao tác ở từng máy muộn hơn giờ hẹn.

Mỗi bài dự kiến trên một tài khoản có `publicationId` cố định; các lần thử có
`attemptId` riêng. Mỗi bài chỉ chiếm một dòng Sheet. Sau khi mở đợt báo cáo mới,
Theo dõi hiển thị **Đợt báo cáo đã đóng** cho nghĩa vụ cũ; lịch sử và xác minh bài
vẫn được giữ trong app, kể cả khi link về muộn. Trạng thái này không có nghĩa
Sheet đã xác nhận bài.


Kết quả kiểm tra liệt kê riêng nội dung, luồng soạn, nhạc, dung lượng, kết nối,
chuyển media, khả năng xác minh link và lượt trước đang chờ. Các mục tài khoản,
clipboard và kết quả xuất bản ghi **Chưa quan sát** cho tới khi chạy bước tương ứng.
Mã lỗi lấy link và bài chờ xác minh có thông báo riêng; bốn mục đầu đạt chưa có
nghĩa toàn bộ đợt đăng đã sẵn sàng. Locale tiếng Anh có vùng như `en-US` dùng
cùng hợp đồng nhãn với `en`.

Trong **Cài đặt → Kết nối và API → Nhận diện giao diện**, bật/tắt hỗ trợ AI,
chọn provider/model hoặc dùng cấu hình AI đã lưu, và đặt trần request. Dịch vụ
nhận diện khởi động khi cần. **Kiểm tra dịch vụ** chỉ xác nhận tiến trình và
giao thức; kết quả model phải được đối chiếu với màn hình trong lúc chạy.
**Xuất chẩn đoán nhận diện** giữ các quan sát và request liên quan để kỹ thuật
phát lại. **Nhập gói tương thích** yêu cầu fixture đúng và thiếu/mơ hồ cho từng
target; **Khôi phục gói tương thích trước** áp dụng cho các phiên mới.

Các tab chính là **Thiết lập · Hẹn giờ · Theo dõi**. Thiết lập mở **Bàn đăng nhanh**
với ba khung nhìn đồng thời: **Chọn bài đăng → Bài ↔ máy → Thiết bị**. Tìm bài ở
khung trái, kiểm/đổi từng cặp ở khung giữa và chọn/gán thiết bị ở khung phải.
Danh sách trong mỗi khung cuộn riêng; thanh **Kiểm tra & đăng** có hàng riêng ở cuối,
không che trường nhập. Nút **Xem ảnh và sửa caption** hoặc **Sửa caption** mở ảnh,
nội dung, đối tác và thông tin nhạc của đúng bài. Escape/Đóng giữ bản nháp, trả focus
về nút đã mở; đổi nguồn không được chuyển nội dung đang sửa sang bài khác.
Thanh đầu đặt **Chọn thư mục** cạnh **Quét**. Phần Google Sheets chỉ gồm ô
**Link Google Sheet**, nút **Đăng nhập Google** và nút **Kiểm tra kết nối**.
Dán link của đúng tab (`gid` trong link), đăng nhập trên trình duyệt rồi kiểm tra.
Link không có `gid` dùng tab `0`. Sửa link sẽ bỏ trạng thái sẵn sàng của link cũ.
Chuyển sang trang khác rồi quay lại giữ kết quả đã kiểm trong phiên app. Sau khi
khởi động lại, kết nối Google và đích đã lưu cho phép đi tiếp tới bước kiểm tra
trước đăng; nhãn trung tính **Đã liên kết Google Sheet** chưa phải xác minh quyền
ghi hiện tại. Preflight kiểm binding và có thể dùng proof quyền ghi còn hạn tối đa năm phút;
đổi quyền remote trong cửa sổ này có thể chỉ được phát hiện khi delivery. Lỗi kiểm tra chặn đăng. Nút **Kiểm tra kết nối** vẫn dùng để đối chiếu ngay khi cần.
Trên màn hẹp, bấm **Thiết lập Google Sheet** để mở các control kết nối; Escape đóng
phần này và trả focus. Trạng thái kết nối/lỗi thật vẫn hiển thị khi thu gọn.
Lượt đăng mới luôn ghi kết quả lên Sheet và dọn bản media đã chuyển sang điện thoại
sau khi xác minh bài; Bàn đăng nhanh không có công tắc tắt hai việc này.

Kết nối kiểm tra trực tiếp bảng và tab trong link, không mở Google Picker.
Khi đăng nhập, cấp quyền Google Sheets cho tài khoản có quyền sửa bảng đó; phiên
cũ chỉ cấp quyền theo file cần đăng nhập lại. Quyền Sheets rộng hơn quyền một file
của bản cũ, nhưng không thay quyền chia sẻ/sửa của bảng. Không cần chọn tab lần
nữa nếu link đã chứa `gid`. Đăng nhập xong chưa có nghĩa bảng đã sẵn sàng ghi:
chờ trạng thái xác minh màu xanh. Tab hoàn toàn trống được chuẩn bị header chuẩn,
không thêm hàng thử.

Mục Apps Script cũ và các bước chọn tab riêng đã được bỏ khỏi màn Đăng bài.
Bản release mới mang sẵn cấu hình ứng dụng Google; mỗi máy vẫn đăng nhập tài khoản
Google riêng rồi kiểm tra đúng Sheet. Quy trình build dừng nếu thiếu cấu hình.
Chép `.exe` hoặc thư mục app không chép phiên đăng nhập đã lưu trong kho
thông tin xác thực của máy cũ. Nếu dùng bản dev chưa có cấu hình, mở **Thiết lập Google**
ngay dưới ô kết nối, nhập OAuth Client ID loại Desktop và client secret (nếu có)
do quản trị viên cung cấp rồi bấm **Lưu cấu hình Google**. Không cần Picker API key
hoặc project number. Các trường này thuộc ứng dụng, không phải mật khẩu tài khoản.
Sau khi đủ cấu hình, form tự ẩn; tiếp tục **Đăng nhập Google** rồi **Kiểm tra kết nối**.
Phiên đăng nhập và cấu hình nhập tay lưu trong kho xác thực hệ điều hành. Trong lúc
đăng nhập, có thể hủy bằng nút Google; lượt chuyển kết nối đã được nhận sẽ hoàn tất
hoặc giữ trạng thái để tiếp tục.

Tab đã liên kết với bản cũ cần nâng cấp một lần để các PC cùng dùng giao thức ghi
chung. **Trước khi bấm “Đã dừng bản cũ · Nâng cấp”, dừng ghi Sheet trên mọi bản app
cũ và đợi các lượt đang gửi kết thúc.** Nâng cấp giữ dữ liệu và link sẵn có, không
cần tạo tab khác. Hủy xác nhận thì chưa nâng cấp; đừng cho bản cũ tiếp tục ghi sau
nâng cấp. Mỗi PC mới tự đăng nhập và kiểm tra cùng link bằng bản hỗ trợ giao thức mới.
Tab đã chuyển sang OAuth trực tiếp không cần dừng Apps Script thêm lần nữa trên
PC mới, kể cả khi PC đó còn cấu hình webhook cũ. Nếu bản 0.2.37 báo “Chưa xác nhận
Apps Script ngừng ghi” với tab đã chuyển, cập nhật bản sửa rồi kiểm tra lại cùng
link; không xóa dữ liệu hay tạo tab khác để né lỗi. Tab thực sự còn dùng Apps
Script vẫn cần xác nhận ngừng đường ghi cũ trước khi chuyển.
Nếu app báo PC khác đang ghi hoặc lượt ghi chưa rõ kết quả, không tự xóa khóa trên
Sheet hoặc cho rằng chờ lâu là được chiếm khóa; cần đối chiếu lượt cũ trước. Chức
năng reset cũ không hỗ trợ tab giao thức mới (schema 2), sẽ từ chối trước sao lưu
hoặc xóa. Đây là quy trình và giới hạn giao thức, không phải xác nhận đã nghiệm thu
ghi đồng thời trên nhiều PC thật.
Preflight phải xác minh thành công bảng, tab và quyền ghi trước khi đăng ngay hoặc
lưu lịch mới. Không có đường đăng mới bỏ qua Sheet. Khi kiểm tra trước đăng hoặc
lưu lịch, Riviu chốt đúng bảng/tab và chính sách dọn media của lượt đó. Đổi kết nối
cho lượt mới không chuyển các bài cũ sang bảng khác; chiến dịch cũ giữ nguyên
chính sách Sheet/dọn media đã lưu, không bị thay đổi hồi tố.
Hộp **Kiểm tra đợt đăng** tách pha chuẩn bị thiết bị và kiểm tra từng máy. Khi chờ,
danh sách bài–máy chỉ ghi **Chờ kết quả**, chưa coi máy là đạt. Khi có kết quả,
điều kiện Sheet chung và máy cần xử lý xuất hiện trước; bộ lọc chuyển giữa
**Cần xử lý / Đạt / Tất cả**. Trên cửa sổ hẹp, **Kiểm tra lại** nằm ở chân hộp
để không phải cuộn qua toàn bộ danh sách máy.
Lượt cũ chưa có đích đã chốt giữ nguyên lịch sử để kiểm tra, không tự gửi sang
kết nối mới.

Khung **Máy thực hiện** có nút **Chọn nhanh**: lấy bài trong nguồn (tối đa 100) và
mọi máy sẵn sàng trong phạm vi; mỗi máy một bài. Khi đã chọn phạm vi máy, khung
mặc định chỉ hiện máy trong phạm vi và máy đã ghép cần sửa; mở **Bộ lọc thiết bị →
Hiện máy ngoài phạm vi** để xem toàn dàn. Cặp đã ghép hợp lệ được giữ. Nếu đợt
trước chỉ gán một phần mà sau đó thêm máy sẵn sàng, bấm lại sẽ lấp tiếp máy trống bằng
bài còn trong nguồn — không kẹt ở tập bài/máy của lần gán cũ. Bài đang chọn chưa đủ máy
vẫn chỉ dùng tập đó cho đến khi đã ghép xong. Thiếu máy thì ghép phần đủ và báo số bài
còn lại vì hết máy. **Hoàn tác** trả lại lần gán nhanh gần nhất nếu chưa sửa ánh xạ.
Bỏ tick máy gỡ bài của máy đó; dropdown **Máy nhận bài …** ở từng dòng cho đổi riêng
bài được nêu tên. Hai bài đã có máy hợp lệ dùng **Đổi chỗ**; bài chưa có máy hợp lệ
lấy máy của bài khác phải xác nhận **Thay bài**, bài bị thay trở về chờ ghép.
Nếu nguồn, phân công hoặc trạng thái máy đổi trong lúc xác nhận, app không áp quyết
định cũ. Máy đã ghép bị offline/ra phạm vi vẫn được giữ để chỉ rõ lỗi, không âm thầm
bỏ cặp. Bài đang gán bị bộ lọc ẩn sẽ khóa Gán/Đổi ở khung phải và có **Hiện bài**;
không tự gán sang bài khác. **Bỏ chọn toàn bộ bài/máy** tác động cả phần bị tìm kiếm ẩn.
Dòng tổng kết và hộp kiểm tra ghi rõ đích Sheet bắt buộc. Có thể Hủy khi đang
chuyển nội dung trước Đăng. Theo dõi tự đọc lại kết quả mỗi5giây khi đang mở.
Đăng nhiều máy chạy theo từng máy: máy nào tải đủ và xác nhận ảnh xong sẽ bắt đầu
đăng ngay khi có lượt điều khiển, trong lúc máy khác tiếp tục tải. Một máy lỗi
không dừng cả nhóm. Chi tiết hiển thị đồng thời các máy đang tải, chờ đăng, đang
gửi và chờ liên kết. Hủy chặn các bài chưa bấm Đăng; bài đã gửi vẫn được xác minh.
Trước nút Đăng, Riviu kiểm tra clipboard của Helper rồi khôi phục nội dung ban đầu.
Phiên bản TikTok phải khớp lần kiểm tra đã duyệt. Mỗi tài khoản chỉ có một bài
đang chờ xác minh link, kể cả khi đăng nhập trên nhiều máy; bài sau chờ bài trước
có link. Chờ ghi Sheet không giữ tài khoản. Lượt cũ chưa qua cơ chế kiểm tra mới
cần kiểm tra và tạo lại trước khi chạy. Với bài đã gửi trong lịch sử, bấm kiểm tra
liên kết để tiếp tục xác minh. Khi còn đủ bằng chứng tài khoản và thời điểm gửi,
lượt kiểm tra chủ động sẽ lưu lịch kiểm tra lại mỗi 5 phút trong ngân sách hữu hạn,
kể cả sau khi mở lại app. Lượt đã Cần kiểm tra vì hết ngân sách chỉ được kiểm một
lần bằng nút này; muốn tự kiểm tiếp phải xác nhận **Tiếp tục xác minh bài đã gửi**. Bài thiếu bằng chứng hoặc đang ở màn nháp/soạn bài vẫn cần kiểm tra riêng;
app không gửi lại bài đã có intent Đăng.

Khi tìm link, app đọc toàn bộ caption, tài khoản và thời gian của bài. Caption bị
rút gọn chỉ được mở khi chính vùng caption có thể bấm và nhận diện đúng; sau đó
đọc lại nội dung. Có nhiều bài cùng khớp hoặc chưa đủ bằng chứng thì giữ trạng thái
cần kiểm tra. App không tự bấm Đăng lần nữa để giải quyết lỗi lấy link.

Sau Đăng, Theo dõi hiển thị lý do chưa xác minh, lần kiểm gần nhất và lần kiểm
kế tiếp. Trên Android, sau khi nhận kết quả đã gửi và máy rảnh, app tắt hẳn đúng
TikTok rồi mở lại trước khi lấy link. Nếu chưa có link, chu kỳ tắt/mở và kiểm tra
lặp sau 5 phút, tính từ cuối lần kiểm trước, trong ngân sách hữu hạn bên dưới.
Nhật ký **Kiểm tra liên kết lần N** là kiểm thao tác đã có, không phải bấm Đăng thêm;
**Chưa xác minh được liên kết** không khẳng định bài đã công khai. Media được giữ khi còn chờ link; link
xác minh xong được gửi Sheet ngay. Chỉ sau khi bài và liên kết chính tắc được xác
minh, app mới xóa bản media đã nhập vào điện thoại; ảnh/video nguồn trên PC không
bị xóa. Lỗi dọn media được theo dõi riêng và không biến thành yêu cầu Đăng lại.
Không đăng lại bài để lấy link. Không restart khi máy đang bận, tác vụ đã dừng
hoặc link đã xác minh.
Tắt/mở lỗi cũng giữ bài và chờ lần kiểm kế tiếp.
Lỗi đọc clipboard được hiển thị riêng với trường hợp đọc thành công nhưng chưa
có link mới. Thông báo TikTok đang xử lý có ảnh đối chiếu sau Copy; trạng thái
này vẫn là chờ, chưa tính là xuất bản hoặc ghi link thành công.
Với Global 45.7.3, nếu lần kiểm tra đã thấy thông báo xử lý trên đúng bài nhưng
chưa có link, app có thể tắt/mở lại TikTok đúng một lần cho intent đó sau khi
xác nhận màn bài ảnh và caption khớp qua hai lần đọc mới. Đây chỉ là bước
khôi phục lấy link, không đăng lại; khi màn soạn, tải lên hoặc bài khác chưa rõ
kết quả, app giữ nguyên TikTok và tiếp tục báo Cần kiểm tra.
Khi TikTok hiện “Post is being processed” sau Sao chép liên kết và app đọc được
thông báo đó, Theo dõi ghi **TikTok báo bài đang được xử lý**. Nếu không đọc được
thông báo, app chỉ ghi **TikTok chưa trả link**; không suy đoán bài đã xuất bản.
Sau khi đọc đủ caption của bài đang mở, lỗi sao chép được giữ làm lý do chờ;
app không quay sang tìm bài khác rồi ghi đè bằng lỗi caption hoặc hồ sơ.
Với bài **hẹn giờ**, phiên đăng trả quyền điều khiển trước khi lấy link. Lần kiểm
tra Android đầu tiên bắt đầu khi phiên đăng đã nhả máy; iOS giữ mốc sau 2 phút.
Bài đã gửi nhưng chưa có link, kể cả **đăng ngay**,
được kiểm tra lại mỗi 5 phút khi còn ngân sách. Với bước tìm bài trên điện thoại,
sau **3 lần kiểm liên tiếp không có
bằng chứng mới của đúng bài**, app chuyển **Cần kiểm tra**, bỏ lịch kiểm tiếp và
hiển thị lý do; không coi là Đăng thất bại và không đăng lại. Lỗi đọc/kết nối hoặc
TikTok vẫn xử lý cũng không được thử mãi. Caption của bài khác, giờ kiểm mới và số
ảnh đọc tăng không tính là tiến triển. Chỉ caption đầy đủ đúng bài, rồi caption và
thời gian cùng bài khớp lần đầu mới mở thêm ngân sách chờ; chưa thay xác minh link.
Link ứng viên đã lưu đúng định danh có ngân sách riêng tối đa 12 lượt chờ dữ liệu
công khai; các lượt này dùng lại link và không tính vào ba lượt thao tác điện thoại.
Mở lại app giữ mốc Đăng, lần kiểm tiếp và bộ đếm. Máy offline/bận chưa có lượt đọc
thì chờ khi sẵn sàng. Bài cũ chưa có bộ đếm bắt đầu từ lượt quan sát đầu của bản mới.
Trước khi mở TikTok để đọc link trên Android, app thử gỡ màn khóa chỉ vuốt và
đọc lại trạng thái khóa. Máy dùng PIN/pattern vẫn phải mở bằng tay; không coi
ACK mở app là bằng chứng TikTok đã lên foreground.
Bài từng dừng vì giới hạn 30 phút/4 giờ tự tiếp tục nếu còn đủ tài khoản và thời
điểm gửi đã ghi nhận. Bài thiếu bằng chứng hoặc cần kiểm tra vì lý do khác vẫn
hiển thị Cần kiểm tra. Việc tìm link không bấm Đăng lại.
Mọi máy đang chờ liên kết kiểm tra độc lập khi Ready (một observer mỗi máy), chỉ
chiếm quyền điều khiển trong từng lần đọc. Cột Link trên Sheet chỉ có giá trị khi
trạng thái **Đã xác minh**; trước đó trống là đúng — đọc Trạng thái / Lỗi. Có thể
bấm **Kiểm tra liên kết** để kiểm ngay mà không chờ lượt tiếp theo.
Sheet nội bộ cập nhật đúng dòng/STT của mỗi bài dù link về ngược thứ tự. Cột Link
chỉ có giá trị khi trạng thái **Đã xác minh**; trước đó cột trống là đúng — đọc cột
Trạng thái / Lỗi để biết đang chờ hay đã dừng tự kiểm tra. Mẫu chỉ ghi bài đã xác
minh thì thêm dòng theo thứ tự nhận link; ngày vẫn lấy lúc Đăng. Gửi lại cùng kết
quả không thêm dòng mới.
Kết quả **Ghi Sheet** được theo dõi riêng: **Đang chờ ghi Sheet** có giờ thử tiếp,
số lần đã thử và lỗi gần nhất; **Cần xử lý ghi Sheet** yêu cầu sửa nguyên nhân rồi
bấm **Ghi lại Sheet**; **Sheet đã xác nhận** chứng minh đúng hàng và link đã được
ghi. Mất mạng được thử lại với thời gian chờ tăng dần, tối đa 15 phút giữa các
lần; khởi động lại app giữ lịch gửi. Máy đã có link không cần chờ các máy khác
để gửi kết quả lên Sheet. Lỗi báo cáo tiến độ cũng không chặn link mới của máy khác.

Bản 0.2.16 kiểm tra phone có đang bị một máy tính khác điều khiển ADB qua mạng
trước khi chuẩn bị đăng. Nếu thấy địa chỉ máy phụ, ngắt kết nối phone ở máy phụ;
phone đang cắm cáp có thể chuyển sang ADB USB. Wi-Fi Internet vẫn dùng để tải bài.
Các máy mới cần đăng nhập TikTok, giờ hệ thống đúng và mạng truy cập được TikTok;
chỉ có biểu tượng Wi-Fi chưa đủ để tải danh sách nhạc.

Khi chọn nhạc, app chờ TikTok tải xong và danh sách ổn định, tối đa **3 phút** cho
toàn bộ bước mở bảng, chọn và xác nhận nhạc. App dùng vị trí vừa đọc lại để chọn;
không bỏ qua nhạc khi tải chậm. Quá thời hạn hoặc mất kết nối thì dừng trước Đăng
và hiện nguyên nhân. Bấm **Tạm dừng** sẽ kết thúc cả vòng chờ nhạc.
Nếu TikTok hiện lỗi mạng trong danh sách nhạc, app báo rõ lỗi đó và dừng trước Đăng.
Chi tiết từng máy có dòng nhận diện khi đổi chiến lược hoặc hết thời gian chờ.
Đọc cây UI bị lỗi không được tính là thiếu nút hoặc đã đăng thành công. Tạm dừng
chặn bước kế tiếp; thao tác đã gửi được xử lý tới khi có kết quả để tránh bấm lặp.
Tên nhạc bị rút gọn ở hàng đã chọn vẫn phải được xác nhận đầy đủ trên màn soạn bài.
Trên Global 45.7.3 tiếng Anh, nếu bảng nhạc đã hiện mà OCR không đọc đủ tên hàng,
app chỉ chuyển sang cây giao diện khi đã xác nhận đúng bảng, tab Hot và các hàng ổn
định. Không chứng minh được thì báo lỗi trước Đăng; không tự chạm vào một hàng bất kỳ.

Lỗi tạm thời trước Đăng được **tự thử lại tối đa 3 lần ngoài lần chạy đầu**, chờ lần
lượt 2, 5 và 10 giây. Nếu agent Android không đọc được cây UI hoặc không trả `/status`,
lượt kế tiếp chờ ít nhất đến hết khoảng phục hồi của phiên (tối đa khoảng 65 giây),
không tiêu hết lượt thử trong lúc instrumentation còn cooldown. Bước chọn nhạc có
tổng thời hạn 3 phút, gồm các lần đọc lại và khoảng chờ thử lại; không cấp thêm
3 phút sau mỗi lỗi. Hết thời hạn thì máy dừng trước Đăng và cần bấm **Thử lại**.
App ưu tiên
tiếp tục bước đang lỗi, giữ bài, caption và nhạc đã chọn; nếu mất màn soạn thì dựng
lại từ bản đã duyệt. Media còn đúng hash và MediaStore được dùng lại.
Máy mất kết nối được chờ đúng serial tối đa **2 phút**, không giữ slot của máy khác.
Caption tạm thời chưa đọc được được kiểm tra lại trong thời hạn của bước, không
bị coi ngay là đã đổi nội dung. Sau nút Tiếp bị mất phản hồi, app kiểm tra màn đã
tới trước khi tiếp tục; không bấm lại chỉ vì chưa nhận được ACK.
Sai account, chưa cho phép USB, thiếu quyền hoặc phiên bản chưa hỗ trợ cần xử lý trước.
Khi mở phiên TikTok trên Android user 0, app tự cấp các quyền runtime còn thiếu trong
danh sách Camera, Microphone, đọc và ghi bộ nhớ nếu TikTok đã khai báo chúng; app đọc
lại trạng thái quyền trước khi tiếp tục. Android từ chối hoặc không đọc được trạng thái
thì máy báo lỗi, không tự bấm hộp thoại và không tiếp tục thao tác đăng. Cơ chế này
không cấp quyền cho ứng dụng khác và không thay thế bước cho phép USB debugging.

Trong **Theo dõi tiến trình**, mỗi máy lỗi có nút **Thử lại**. Bấm nút này chỉ chạy
thêm **một lượt bài**; trong lượt đó lỗi tạm thời trước Đăng vẫn được thử lại tối đa
ba lần ở đúng bước. Không tự tạo lượt bài khác nếu lượt thủ công này thất bại.
Nếu agent đang phục hồi, lệnh được ghi nhận
nhưng chờ hết cooldown của máy trước khi mở phiên mới. Khi đang tự phục hồi, nút bị khóa và
hiện bước cùng số lần. Bài đã Post chỉ có **Kiểm tra liên kết** hoặc **Ghi lại Sheet**;
không gửi lại Post. Dừng hủy cả lượt đang chờ. Mở lại app giữ bộ đếm và yêu cầu bấm
Thử lại cho lượt phục hồi bị gián đoạn. **Tải lại tiến độ** chỉ đọc lại màn hình theo dõi.
Nếu máy đã làm xong nhưng lưu kết quả tạm thời lỗi, app thử lưu lại kết quả đó,
không chạy lại điện thoại. Khi mở app mà còn kết quả chưa đối soát được, Đăng bài
báo rõ nguyên nhân và tự kiểm tra lại sau 30 giây. Kiểm tra quyền ghi và dung lượng
ổ đĩa nếu trạng thái này kéo dài; không tạo thêm lượt đăng để xử lý lỗi lưu kết quả.
Khi quay lại từ phần nhạc, app đối chiếu lại toàn bộ caption. Khoảng trắng cuối dòng
do TikTok thêm sau hashtag được chấp nhận; thay đổi chữ, hashtag hoặc xuống dòng vẫn
bị từ chối trước Đăng.

Với bài đã gửi nhưng còn thiếu link, **Tạm dừng** kết thúc kiểm tra tự động, đóng
TikTok và nhả từng máy sau khi đã đóng được phiên của máy đó. Bài cũ vẫn nằm trong
lịch sử cần kiểm tra link, nhưng không chặn gán nội dung mới cho máy đã nhả. Máy
còn việc của phiên khác hoặc chưa đóng được sẽ báo riêng. Mở lại app không tự
đăng hay kiểm tra tiếp phiên đã dừng; chỉ tiếp tục kiểm tra khi bạn yêu cầu.

**Hẹn giờ** dùng nguồn đã quét trong Thiết lập. Khi chưa có bản nháp lịch, những
bài và cặp bài–máy đã chọn được mang sang. Bản nháp hiện có được giữ khi chuyển tab.
Ngày/giờ nằm trên cùng, danh sách bài bên trái, máy nhận và bảng phân công bên phải.
Trong cửa sổ thấp, cuộn vùng nội dung để xem bảng; ngày/giờ và nút lưu vẫn cố định.
**Chọn nhanh** nằm đầu vùng máy: bấm một lần để tự gán bài từ nguồn (tối đa 100) vào
mọi máy sẵn sàng trong phạm vi. Cặp đã gán giữ nguyên; bấm lại sẽ lấp máy sẵn sàng
mới bằng bài còn lại, không kẹt ở lần chọn cũ. Thiếu máy thì báo số bài còn lại vì hết
máy; hết bài thì báo số máy sẵn sàng còn trống. Bấm lại không đảo cặp đã đúng,
**Hoàn tác** phục hồi toàn bộ lần chọn nhanh.
Tick nhiều bài bên trái và máy bên phải. Kéo một bài trong nhóm đã tick vào vùng
máy để phân lần lượt theo thứ tự bài và số máy; kéo một bài chưa tick chỉ gán bài đó.
Thả trực tiếp lên máy trống chỉ chuyển đúng bài đang kéo, kể cả khi đã chọn cả
nhóm và máy đích chưa tick. Thả vào vùng chung mới phân nhóm đang chọn.
Máy đã nhận bài cùng giờ giữ nguyên; phần thiếu chỗ nằm ở **Chưa có máy**. Kéo tiếp
chỉ lấp chỗ trống. Có thể dùng bàn phím chọn bài/máy và bấm **Gán bài đã chọn**.
Khi đã chọn bài và máy còn chỗ, thanh cuối cũng hiện **Gán bài** để không phải
cuộn xuống tìm nút trong cửa sổ thấp. Số **máy đã ghép** chỉ đếm cặp trong bản nháp;
chỉ số **bài có máy khả dụng** mới tính máy đang đủ điều kiện để kiểm tra lịch.

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
TikTok hoàn tất tải bài. Phải giữ Riviu/controller và máy tính đang chạy đến giờ
hẹn, điện thoại kết nối mạng. Mở lại app sau giờ hẹn sẽ đánh dấu lượt đó
**Lỡ lịch**, không tự đăng bù hoặc gửi lại Post.
Muốn ngày khác, tạo một lịch mới với ngày khác; tính năng này không tự lặp hằng ngày.

Cách chạy một lượt trong Bàn đăng nhanh:

1. Bấm **Chọn thư mục**, rồi **Quét**; đánh dấu từng bài hoặc **Chọn tất cả bài**.
   Một thư mục con là một bài gồm toàn bộ ảnh; có thể chọn trực tiếp thư mục của một bài.
   Caption nhận `caption*.txt`, tên có từ `caption` như `Tiktok_Caption.txt`, hoặc
   tệp `.txt` duy nhất trong bài. File đối tác nhận `partners*.xlsx` hoặc `.xlsx`
   duy nhất, chẳng hạn `DanhSach_DoiTac.xlsx`; bỏ qua file khóa Excel `~$...`.
   Nếu có nhiều caption/Excel cùng phù hợp, app báo lỗi thay vì tự đoán.
   Ảnh PNG/JPG/JPEG có tiền tố số vẫn cần đủ thứ tự 01, 02…; nếu toàn bộ ảnh
   không có tiền tố số, app xếp theo tên tự nhiên (ảnh 2 trước ảnh 10) và nhắc
   kiểm tra thứ tự trong xem trước. Không trộn hai cách đặt tên trong cùng bài.
   Ô Thư mục nguồn cho nhập đường dẫn và **Quét**. Đổi nguồn trong lúc quét sẽ bỏ kết quả
   trả muộn của nguồn cũ. Quét xong chưa tự chọn bài hoặc tạo chiến dịch.
2. Bấm một bài để xem caption, ảnh/video và đối tác trong **Bài đang chỉnh**. Sửa trực tiếp
   **Nội dung bài đăng**; bản nháp giữ thay đổi, file nguồn giữ nguyên. **Phóng to ảnh**
   ở cạnh ảnh đại diện cho chuyển ảnh; Escape trở về. Chọn ảnh xem trước chỉ đổi
   ảnh đang xem, không bớt ảnh khỏi bài.
3. Chọn máy và bấm **Ghép bài với máy đã chọn**. Dòng cuối phải đủ `N/N bài có máy`.
   Chọn 10 bài chỉ cần 10 máy dù đang kết nối 20 máy. Nếu chọn riêng ít hơn số bài,
   thêm máy hoặc giảm số bài rồi ghép lại.
4. Kiểm tra đúng link/tab Google Sheet đã xác minh, rồi bấm **Kiểm tra & đăng**.
   Đọc kết quả từng máy, bấm **Xác nhận đăng công khai N bài** (xác nhận cuối, không có popup thứ hai). Nhạc được
   chọn và đọc lại trong TikTok sau khi mở máy; kiểm tra đầu vào chưa có nghĩa đã chọn
   nhạc. Các máy có thể dùng trùng nhạc. Bài chưa xác minh giữ media đã chuyển trên
   điện thoại; sau khi xác minh liên kết, app dọn bản chuyển và hiển thị kết quả dọn
   riêng trong chi tiết. Thư mục nguồn trên PC được giữ nguyên.

Rời trang hoặc mở lại app khôi phục bài, caption và ghép máy đã lưu trong bản nháp.
Kết quả kiểm tra và quyền xác nhận đăng không được khôi phục; phải kiểm tra lại trước lượt mới.

**Theo dõi** có bộ lọc **Tất cả / Đang chạy / Cần xử lý / Hoàn tất** và ô tìm nguồn
hoặc ngày đăng. Chọn chiến dịch ở danh sách để xem kết quả từng máy bên cạnh;
nếu cửa sổ hẹp, app đưa khung chi tiết vào tầm nhìn ngay sau khi chọn chiến dịch.
nút kiểm tra liên kết, ghi lại Sheet hoặc hủy nằm trong chi tiết của lượt đã chọn.
Chuyển về tab **Thiết lập** giữ nguyên bài và máy đang ghép.

**Đầu vào:** một MP4 H.264/AAC hoặc 1–35 ảnh, caption, âm nhạc, Sheet và máy được
gán cho từng bài. **Thao tác:** preflight trước dispatch; đối chiếu một bài với một máy
đã gán và toàn bộ hậu quả cleanup; chạy pipeline rồi xem projection trong Theo dõi.

**Kết quả:** bằng chứng Post, URL, nhạc, Sheet và cleanup riêng biệt. **Đã bấm Đăng · chờ xác minh**
nghĩa TikTok đã nhận thao tác nhưng Riviu chưa xác nhận bài đã xuất bản. Về bảng tin
không phải bằng chứng tải xong. Riviu chờ máy rảnh rồi mở lại TikTok trên Android để kiểm tra liên kết;
quá trình tiếp tục khi mở lại app và kết nối lại máy. Nút **Kiểm tra liên kết** cho kiểm tra
sớm, chỉ lấy liên kết/ghi Sheet, không đăng lại bài hay đăng các bài còn lại trong lượt đó.
Giữ Riviu chạy, máy tính không ngủ và điện thoại có mạng. Không có thời gian hoàn tất cố định
vì TikTok còn xử lý/xét duyệt. **Chưa chắc chắn** giữ bằng chứng riêng và không cho đăng lại.

Với lượt mới, **Hoàn tất** chỉ xuất hiện khi có liên kết đã xác minh và Sheet đã
xác nhận.
Còn nợ Sheet thì hiển thị **Hoàn tất một phần**, với **Ghi lại Sheet** khi phù hợp.
Máy chờ xác minh không được tính vào số máy hoàn tất hoặc hiển thị tiến độ 100%.
Mọi chiến dịch mới cần kết nối Sheet đã xác minh; nếu kết nối mất sau khi đăng,
link đã xác minh được giữ để gửi lại đúng bảng/tab, không phát lại Post. Chiến dịch
cũ giữ chính sách Sheet và dọn media đã lưu lúc tạo, kể cả khi chính sách mặc định
cho lượt mới thay đổi. Nhạc được chọn ngẫu nhiên có seed từ tối đa
năm đề xuất/thịnh hành đang hiện trên tài khoản, không lấy danh sách ngoài TikTok.

Nếu bước kiểm tra báo chưa hỗ trợ, xem **phiên bản TikTok và ngôn ngữ của từng máy**.
Chọn một máy đã hỗ trợ hoặc cung cấp thông tin đó để hiệu chỉnh; không suy rằng cài
Riviu thành công đồng nghĩa mọi phiên bản TikTok đều đăng được. Máy cài cả hai TikTok cần chọn binding ứng dụng trong Chi tiết thiết bị rồi kiểm tra lại. Kết quả
đạt ở bước này chỉ xác nhận điều kiện đầu vào; nhạc vẫn được chọn và đọc lại trong
luồng đăng. Thông báo gốc và mã máy nằm trong **Chi tiết kỹ thuật**.

Capability phải đọc từ backend trên đúng máy/package/build/locale. Các số đo
0.2.x và danh sách máy của đợt cũ nằm trong [snapshot lịch sử](../archive/technical-snapshots-2026-10-01/operator-guide.md),
không là chứng nhận cho bản TikTok khác. Giao diện hỗ trợ không đồng nghĩa live Post đã đạt.

**Tiếp theo:** retry chỉ phạm vi metadata/outbox
còn thiếu; không mở lại full pipeline cho bài đã đăng. Cleanup là tập effect cụ thể,
không phải xoá tùy ý theo tên thư mục.

### Tiếp tục xác minh bài đã gửi sau khi Dừng

Trong **Theo dõi → Chi tiết máy**, bài thuộc chiến dịch đã Dừng hoặc đã dừng tự
kiểm vì 3 lượt không tiến triển có thể hiện **Tiếp tục xác minh bài đã gửi** khi
backend xác nhận còn đủ identity của lần Đăng. Chỉ xác nhận này mở ngân sách mới;
refresh, khởi động lại app hoặc Kiểm tra liên kết thất bại không tự mở lại.
Đọc xác nhận trước khi tiếp tục: chỉ kiểm tra bài cũ trên máy đó, không đăng lại,
không tiếp tục những bài chưa gửi của máy khác. Thiếu tài khoản/thời điểm gửi hoặc
cần kiểm tra thủ công vì lý do khác thì không được mở quyền này. Nếu trạng thái
vừa thay đổi, app yêu cầu đọc lại thay vì áp dụng xác nhận cũ.
Nếu tác vụ vẫn đang Dừng/nhả máy hoặc lần dừng bị gián đoạn, hoàn tất **Dừng** trước
khi tiếp tục xác minh. Không mở lại observer trong khi lệnh đóng phiên cũ còn chạy.

Nhận yêu cầu không có nghĩa đã lấy được link. App dùng worker hiện có để kiểm tra
lại sau mỗi 5 phút tính từ cuối lần kiểm trước: ba lượt tìm bài liên tiếp không có
bằng chứng mới, hoặc tối đa 12 lượt chờ dữ liệu công khai của link đã giữ. Bộ đếm
được giữ qua restart; máy offline/bận chờ khi sẵn sàng. Campaign vẫn có thể mang nhãn **Đã huỷ** dù một bài trong đó đang
được tiếp tục xác minh. Bài và lịch sử Đăng, các máy chưa gửi, cùng đích Sheet/đợt
báo cáo ban đầu không đổi. **Kiểm tra liên kết** chỉ quan sát bài đã gửi; báo máy
bận, đã dừng, chưa đủ điều kiện hoặc chưa có link không phải thành công.

Để dừng lại, bấm **Dừng kiểm tra lại** và xác nhận. Nút này dừng quyền xác minh đã
được tiếp tục của **cả chiến dịch đang chọn**, không chỉ máy đang xem. Bài đã gửi,
link đã có và trạng thái chiến dịch được giữ; dừng kiểm tra không hoàn tác bài hoặc
cho phép đăng lại.

**Tiếp tục xác minh không phải nút mở khoá máy.** Máy còn upload/chưa rõ kết quả vẫn
bị giữ. Với bài Submitted cũ, app chỉ có thể nhả giữ khi đã qua ít nhất 4 giờ từ lần
Đăng, không có pipeline đang chạy và có quan sát mới đúng Hồ sơ/tài khoản/package,
khớp identity của bài; bằng chứng này chỉ có hiệu lực 24 giờ. Máy còn khoản giữ khác
vẫn chưa được dùng cho lượt mới. Không dùng ảnh cũ, thời gian chờ riêng lẻ hoặc xóa
lượt để vượt điều kiện an toàn.

Bài **Dừng trước khi đăng** có thao tác **Thử lại máy này**, chỉ dành đúng bài chưa
qua Post. Khi backend báo pipeline còn chạy, máy bận hoặc chưa đọc được quyền thử
lại, nút bị khoá kèm lý do; chờ/đọc lại chi tiết, không tạo campaign khác để né guard.
Thao tác này khác tiếp tục xác minh, kiểm tra link và ghi lại Sheet.

Khi xác nhận thử lại, nhạc tự chọn chưa xác minh được chọn lại theo cấu hình đã
lưu của bài nếu danh sách Hot đã đổi; binding cũ được giữ trong audit. Retry tự
động trong cùng lần chạy không đổi nhạc. Nhạc đã xác minh và bài đã qua Post
không được reset hay gửi lại bằng thao tác này.

### Đọc cảnh báo nguồn trước khi xác nhận

Trong Bàn đăng nhanh, mở **Cảnh báo nguồn (N)** để đọc thông báo và đường dẫn bài/file
cụ thể, gồm thông tin đối tác thiếu, rỗng hoặc không đọc được do scanner báo. Con số
N thuộc lần quét hiện tại, không phải số máy lỗi. Thiếu đối tác không tự sửa nguồn:
bài hợp lệ vẫn có thể đăng, còn phần tên đối tác tương ứng sẽ trống.

Thông báo **caption trùng** chỉ tính các bài đang chọn theo nội dung bạn đang chỉnh;
đổi caption trong bản nháp sẽ cập nhật cảnh báo. Đây là lưu ý không chặn mặc định,
không tự thay chữ hoặc sửa file nguồn. Kiểm tra/preflight vẫn có thể chặn vì lỗi
nội dung hoặc điều kiện máy thật. Các hướng dẫn này mô tả chức năng, không khẳng
định một lượt đăng thật đã thành công.

### Đọc báo cáo nghiệm thu mà không chiếm chuột

Kỹ thuật có thể dùng `scripts/publish_acceptance.mjs` qua IPC của Riviu đang chạy,
không mở app/driver thứ hai hoặc đưa cửa sổ lên trước. Mặc định **inspect** chỉ đọc
roster/metadata và campaign đã chỉ định. **Observe** chỉ theo dõi bài cũ, không gửi
lệnh kiểm tra thiết bị, không tiếp tục bài đã Dừng, không đăng lại. Hết thời gian
báo cáo không dừng app hoặc worker đang xác minh; giữ máy tính/Riviu chạy và điện
thoại có mạng. Không restart app đang giữ upload chỉ để mở cổng debug.

**Preflight** là bước riêng, có kiểm thiết bị và kết nối Sheet. **Submit** có thể
đăng thật, chỉ dùng sau khi bạn duyệt đúng nguồn, cặp bài–UDID, số bài và Sheet/tab;
phải nhập hash xác nhận của chính preflight. Không lấy số máy làm ID. Báo cáo giữ
đủ roster và danh sách máy không nằm trong yêu cầu, không tự bỏ máy lỗi để đổi mẫu số.
Máy chưa gán username vẫn có thể tham gia nếu preflight đọc được username từ hồ sơ
đang đăng nhập; Submit sẽ đọc lại đúng username đó trước khi tạo lượt. Không đọc
được hoặc tài khoản đã đổi thì dừng, không tự gán nick hay thử đăng bằng tài khoản khác.
Xem lệnh và tham số trong [hướng dẫn phát triển](../developer-guide.md#nghiệm-thu-publish-qua-ứng-dụng-đang-chạy).

Báo cáo canary kỹ thuật cô lập trước đây có thể mang nhãn Sheet-disabled; đó không
phải đường tạo lượt đăng mới trong ứng dụng và không chứng minh ghi Sheet. Harness phải
đối soát tác vụ cũ trước Start, scope dev phải có activation ngẫu nhiên khớp process,
và kết quả chỉ được gọi `phoneOnly/sheetDisabled`, không phải nghiệm thu end-to-end
Sheet. Lượt mới ghi intent trước Start; mất ACK thì chỉ đọc lại receipt theo đúng
requestId, không tự gửi Start lần hai. Báo cáo cũ chỉ được quan sát, không replay
Create/Execute.

Đọc từng mốc, không gộp thành một dấu thành công:

1. **Enqueued**: đã thấy công việc trong pipeline; chưa chứng minh Đăng.
2. **Submitted**: receipt đã gửi; chưa chứng minh TikTok cấp link.
3. **Verified**: backend lưu proof xuất bản và canonical URL; chỉ URL hiện trên dòng
   hoặc state succeeded cũ không đủ.
4. **Sheet sent**: backend đã settle delivery, tách khỏi trạng thái bài.
5. **URL readback**: harness gọi `publish_sheet_readback` qua kết nối OAuth backend
   để đối chứng ô Sheet và receipt đúng assignment/revision/epoch. Chỉ `matched`
   khi URL và identity khớp; lỗi mạng/quyền hoặc chưa có receipt vẫn là `pending`.
   Không xuất token; CSV công khai không thay bằng chứng đúng writer/target/epoch.

Mã thoát `2` là còn chờ/hết hạn hoặc ACK chưa rõ; `1` là lỗi/bị chặn; `3` là đã có
verified + sent nhưng chưa đủ readback bổ sung. `0` của inspect/preflight chỉ nghĩa
bước đọc/kiểm đầu vào xong, **không** là nghiệm thu post→Sheet. Khi mất ACK Start, giữ
nguyên report-dir và requestId đã lưu; kỹ thuật chỉ đọc `publish_start_status`.
Có `start-intent.json` thì harness không tự gửi Start lần nữa, kể cả chưa biết
backend đã nhận.
Không xóa intent, tạo thư mục khác hoặc đổi requestId để “thử lại”. Bài pending được
app kiểm theo lịch hiện có; không đóng TikTok hoặc gửi lại để giải quyết thiếu link.

Canary Rust cũ chỉ dùng cô lập để khảo sát/rehearsal, Sheet tắt và thiếu worker đầy
đủ; không dùng làm chứng cứ nghiệm thu OAuth end-to-end. Không chạy nó song song
Riviu trên cùng điện thoại hoặc nhập DB thử vào dữ liệu đang vận hành.

Khi đã giữ được liên kết ứng viên của đúng bài đã gửi, lượt xác minh tiếp theo
kiểm tra dữ liệu công khai trước. TikTok chưa cung cấp đủ dữ liệu thì app giữ
bài chờ kiểm tra; liên kết ứng viên chưa được ghi là bài thành công hoặc ghi
Sheet. Lượt chưa có ứng viên tiếp tục dùng luồng quan sát TikTok có giới hạn.
