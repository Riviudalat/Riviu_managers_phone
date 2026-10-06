# Nuôi và Tương tác

[Hướng dẫn vận hành](../operator-guide.md) · Chi tiết chức năng hiện hành; bảng khả năng và proof trên máy quyết định quyền chạy.

## Nuôi TikTok

Giao diện dev hiện dùng các tab ngang **Thiết lập · Hẹn giờ · Theo dõi**.
Ô **Tổng số video muốn lướt** nằm ngay đầu Thiết lập và là tổng cho mỗi máy;
phiên dừng khi đủ số video hoặc hết thời lượng. Nhập tổng mới sẽ dùng một vòng,
còn hồ sơ nhiều vòng vẫn hiển thị tổng đã nhân để dễ kiểm tra.
Trang chỉ có một ô nhập tổng video; phần Hành vi giữ các tùy chỉnh nhịp xem, không
hiển thị lại giới hạn/vòng hay các tỷ lệ tương tác đã có ở thiết lập phiên.

Thanh **Máy thực hiện** ở đầu Thiết lập giữ **Phạm vi thiết bị**, số máy đã chọn
và nút **Chọn máy**. Nút này mở bảng chọn hai cột có tìm kiếm và cuộn riêng;
**Xong** hoặc Escape đóng bảng nhưng giữ lựa chọn. **Chọn tất cả đang kết nối** gồm
cả máy ngoài kết quả tìm kiếm; **Bỏ chọn** xóa lựa chọn. Có thể chọn máy đang bận
hoặc chưa chuẩn bị; khi bắt đầu, app dừng tác vụ cũ liên quan, chờ nhả máy rồi kiểm tra
sẵn sàng. Máy mất kết nối không được chọn thêm. Tìm kiếm không đổi phạm vi của lượt.
Cấu hình và tóm tắt phiên đọc từ thiết lập hiện tại; thanh tab và nút bắt đầu
nằm ngoài vùng cuộn. Phạm vi Nuôi không thay lựa chọn của Tương tác hoặc Đăng bài.

Nút **Tạm dừng** của phiên đăng bài (các tác vụ khác là **Dừng**) cạnh danh sách trong cửa sổ **Theo dõi tác vụ** dừng toàn bộ máy của
tác vụ đang chọn và đóng TikTok về màn hình chính. Đây là dừng/hủy phần việc còn lại,
không phải tạm ngưng để tự chạy tiếp. App chờ thao tác đang thực hiện trả quyền máy,
báo từng máy đã đóng hoặc cần kiểm tra. Kết quả đã đăng/gửi và bằng chứng vẫn giữ;
kiểm tra link tự động của tác vụ Đăng bài đã dừng cũng được dừng qua lần mở app sau.
Nếu máy đang có tác vụ khác, app báo rõ thay vì đóng ứng dụng của tác vụ đó.

Khi kiểm tra khả năng chạy, Nuôi và Tương tác chờ tác vụ chuẩn bị helper hoặc
dọn nền nhả máy trong tối đa 9 giây. Nếu vẫn bận, app giữ lý do thật; không
giành quyền của tác vụ đang chạy.

Nuôi đang chờ quyền điều khiển vẫn nhận lệnh **Dừng**; thời lượng phiên có tính
thời gian chờ này. Thao tác đã gửi được chờ kết thúc trước khi nhả máy. Bấm chạy
trùng một máy không thay trạng thái và số video của phiên đang chạy; lượt bị từ
chối vẫn có trong lịch sử.

**Thiết lập** có các tab trực tiếp **Phiên nuôi · Hành vi · AI**. Lịch sử bình luận
và token AI nằm trong **Theo dõi → Bình luận & chi phí AI**, bên cạnh **Tiến độ máy**.
Nút **Sửa thiết lập** mở đúng tab và đưa focus về trường cần sửa.
Mỗi máy hiển thị trạng thái bằng chữ cùng lý do khi chưa sẵn sàng. Tab **Hẹn giờ** chứa lịch
tự chạy và các khung giờ riêng; bấm **Áp dụng hẹn giờ** để lưu lịch xuống ứng dụng.
Chuyển tab hoặc sửa bản nháp chưa bắt đầu phiên; **Kiểm tra & bắt đầu** vẫn là
thao tác riêng. Giữ máy tính và Riviu đang mở để lịch chạy.

Trang Thiết lập đặt cấu hình phiên cạnh tóm tắt; danh sách máy mở bằng **Chọn máy**.
Tắt một hành vi khóa các ô tỷ lệ của hành vi đó nhưng giữ số đã đặt để dùng khi bật lại.
Bộ tỷ lệ mặc định là Tim 20%,
Lưu 5%, Bình luận 2%, Theo dõi 1%; khi bật đủ bốn hành động, **Chỉ xem là 72%**.
Mỗi video chọn tối đa một hành động. Phần Chỉ xem tự bù để tổng luôn là 100%; tắt
một hành động trả tỷ lệ về Chỉ xem và giữ số đã đặt. Bình luận cần bật riêng khi có AI.
Hồ sơ cũ chỉ chuyển sang phân bổ này khi bạn lưu bản thiết lập đã duyệt; lượt đang
chạy giữ chế độ của mình. Chọn máy rồi bấm **Kiểm tra & bắt đầu**.
Thời lượng trên trang được gửi vào phiên thật. Lịch nằm trong **Hẹn giờ**; Theo dõi
có log riêng từng máy.

Trong **Phiên nuôi → Nguồn video**, chọn **Lướt theo từ khóa**, nhập ví dụ `đà lạt`.
Android mở Tìm kiếm, nhập đúng từ khóa, chuyển sang Videos và mở một kết quả để lướt.
Tab Videos có thể đổi vị trí khi TikTok tải kết quả; app chờ vị trí ổn định và kiểm tra
tab đã chọn trước khi mở video. Kết quả phải giữ đúng từ khóa đã nhập.
App xác nhận ô tìm kiếm trước khi mở kết quả; không thấy nút/kết quả hoặc rời màn video
tìm kiếm thì báo lỗi/dừng, không chuyển ngầm sang FYP. Đổi từ khóa áp dụng từ phiên
tiếp theo. Giới hạn video, thời lượng, tỷ lệ và đóng TikTok cuối phiên vẫn áp dụng.

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
riêng bài đã gửi giữ nội dung đã chuyển. Android sẽ tắt/mở lại TikTok khi tới lượt
xác minh link, lặp sau 5 phút khi còn ngân sách; 3 lượt không tiến triển thì cần kiểm tra. Máy còn bài đang tải hoặc chưa rõ kết quả Đăng
chưa bắt đầu phiên tự động mới có bước tắt TikTok. Riêng bản ghi đã có `Posted`, đã
dừng tự xác minh và không có pipeline hoạt động: sau ít nhất 4 giờ kể từ cập nhật cuối,
việc thiếu link chuyển thành lưu ý **Liên kết bài cũ**, không giữ máy mãi. Bài cũ vẫn
cần đối soát trong Theo dõi, không được đưa lại vào lượt đăng hay báo Sheet thành công.
Bản ghi cũ chưa phát thao tác và reservation xác minh chưa Send không giữ máy mãi.
Nếu bằng chứng xác nhận chỉ còn văn bản chưa dọn, app có thể đóng tiến trình bằng
lease mới; lịch sử vẫn chưa rõ và không được tự gửi lại. Bản ghi đã qua Send mà chưa
xác minh vẫn giữ điều kiện kiểm tra.
Lỗi dọn được báo riêng. Lượt đã qua Send/Post mà chưa rõ kết quả
không tự gửi/đăng lại; việc kiểm tra lại chỉ đi theo phạm vi đã được ghi nhận.

Trên TikTok Trill 38.3.2 giao diện tiếng Anh, Riviu tự chọn **Deny** cho hộp
thoại quyền vị trí đã nhận diện, rồi xác minh lại màn hình. Nhánh này áp dụng cả
trước khi mở ứng dụng và lúc quay về feed; hộp thoại chưa nhận diện vẫn cần kiểm tra.

## Tương tác

Nếu bằng chứng tác giả/nội dung thay đổi trong lúc copy link trước tương tác, app
lấy lại toàn bộ bằng chứng đúng bài một lần trong thời hạn hiện có. Chỉ tiếp tục
khi canonical link và trạng thái bài đều khớp. Kết quả Like/Send đã gửi không được
tự lặp lại khi chưa rõ kết quả.

Riviu tự lấy và lưu ID bình luận sau khi xác minh trên TikTok Android `45.7.3/en`.
Khi có ID đã lưu, lượt reply mở và đối chiếu lại ID của câu đang hiển thị; người dùng
không cần lấy ID thủ công. Nếu không lấy được ID trong thời hạn, Riviu giữ cách tìm
bằng nội dung và tác giả đã xác minh. Câu không xuất hiện trên tài khoản trả lời vẫn
báo lỗi/chờ kiểm tra, và bình luận đã gửi không tự gửi lại.

**Phân bổ số lượt và cụm hội thoại** cho nhập Tim/Lưu/Share riêng trên mỗi bài.
Tim/Lưu đã có được bỏ qua và bù bằng máy khác trong phạm vi đã chọn; hết máy thì
báo số còn thiếu. Share gửi một người có nhãn Bạn bè trên TikTok; không có bạn
thì bỏ qua. Không gửi lại hành động đã qua Send mà chưa rõ kết quả.

Tổng bình luận gồm bình luận đơn và hội thoại (tính cả câu gốc). Ví dụ 40 tổng,
20 đơn thì còn 20 câu hội thoại. App chia cụm tối đa 4 tài khoản, chia lại phần
dư để không có cụm một người. Nhập caption/mô tả thật để AI soạn, kiểm từng câu
và bảng phân công trước chạy; có thể sửa trực tiếp. Mỗi lượt giữ nguyên nội dung
và tài khoản khi thử lại. Xem trước hành động và nghỉ giữa bình luận có khoảng
giây riêng; đây là điều tiết nhịp chạy, không bảo đảm tránh giới hạn của TikTok.

Các tab chính là **Thiết lập · Hẹn giờ · Theo dõi**; cấu hình nằm trong
Thiết lập, Hẹn giờ lưu bản chụp thiết lập hiện tại. Các phần Chọn bài viết/Hành động & máy/
Kiểm tra & chạy dùng tab ngang; nút chạy chỉ bật khi đã đủ điều kiện.

Thanh **Máy thực hiện** giữ phạm vi và số máy ở đầu workspace. **Chọn máy** mở
bảng hai cột dùng cùng kiểu ô chọn như Nuôi: tìm máy, Chọn tất cả, Bỏ chọn và cuộn
trong bảng. **Xong** hoặc Escape đóng bảng, giữ lựa chọn; tài khoản mở từ dòng dưới
tên máy. Máy được thêm bởi tag chỉ gỡ khi sửa tag; Bỏ chọn bỏ các máy tick trực tiếp.

Ba bước trên trang là **Chọn bài viết → Hành động & máy → Kiểm tra & chạy**.
Tắt Bình luận thì chỉ cần chọn Tim/Lưu và máy; phần AI được ẩn. Theo dõi hiển thị
bảng kết quả theo máy; **Xem log** mở bằng chứng và thao tác xử lý của đúng máy.

Sau từng hành động và khi lượt kết thúc, Riviu chụp ảnh mới từ phiên của đúng máy
trước khi dọn ứng dụng. Nút **Ảnh** mở ảnh đã lưu theo lượt, kèm thời điểm và hash
trong bằng chứng. Nếu lần chụp mới thất bại, giao diện báo rõ và không lấy ảnh cũ
thay thế. Lỗi lưu ảnh không làm app lặp lại hành động đã gửi.

**Đầu vào:** URL bài, hành động, nội dung/AI và máy thực hiện. **Thao tác:** parse
đúng chuỗi URL hiện tại; sửa lỗi parse trước khi chạy; review assignment và số bài/số máy;
chạy rồi theo dõi kết quả từng hành động.

**Hội thoại theo kịch bản** cho mỗi link một nội dung riêng. Chọn bài trong mục kịch bản,
dán các dòng `@vai: nội dung`, bấm Phân tích rồi sửa câu, chủ đề, parent và tag.
AI có thể soạn trước từ mô tả/caption do bạn nhập; toàn bộ câu vẫn phải duyệt trước chạy.
Đổi bài, vai, số câu hoặc yêu cầu trong lúc AI đang soạn sẽ bỏ kết quả trả về của
yêu cầu cũ. Kịch bản sinh bởi AI và kịch bản có sẵn đều được kiểm tra ID, vai,
thứ tự trả lời và giới hạn nội dung trước khi chạy trên cùng luồng Tương tác.
Chọn máy Android cho từng vai; bảng Vai → Máy → Username lấy username đã lưu
trong danh sách thiết bị, không lấy tên máy hay tên hiển thị TikTok. Máy thiếu nick
có nút Gán tài khoản mở ô sửa hiện có. Thiếu, trùng, không hợp lệ, chưa lưu hoặc
lỗi đọc tài khoản đều chặn chạy. Sửa nick đã lưu cập nhật bản nháp và kiểm tra lại.
Username là cấu hình: đổi tài khoản trực tiếp trên máy được tag mà chưa cập nhật
danh sách sẽ không được phát hiện bằng tra danh sách. Dùng Đọc tài khoản từ máy
khi cần đối chiếu; app vẫn kiểm tra tài khoản thực tế trên máy gửi.
Một vai luôn giữ cùng máy và username trong lượt đã tạo, kể cả khi
quay lại link sau nhiều lượt. Reply chờ parent đã xác nhận; mọi tag phải được chọn
và xác nhận từ gợi ý TikTok trước Send. Tag được gắn sau phần chữ để giữ token.

Nhập thời lượng chung (mặc định 120 phút) hoặc khung giờ bắt đầu–kết thúc. Engine
chọn một câu mỗi lượt, luân phiên các link đủ điều kiện; không trả lời hết một bài
rồi mới sang bài khác. Dự toán giữ 10% thời gian dự phòng, cảnh báo khối lượng vượt
thời lượng. Hết giờ dừng câu chưa gửi, hoàn tất đọc lại câu đã Send; không tự gửi
lại câu đã xác nhận hoặc chưa chắc kết quả. Theo dõi ghi giờ kết thúc, lượt kế,
người nói và nhánh. Mở lại app giữ kịch bản và tiến độ; tiếp tục dùng giờ kết thúc cũ.

Trên Android, sau thao tác Gửi app hiển thị **Đang xác minh nội dung**. App tự đọc lại
tối đa ba lượt có giới hạn; chỉ chuyển thành công đầy đủ khi khớp nội dung, tác giả
và nhánh. Nếu chưa đủ bằng chứng, dòng chuyển **Cần kiểm tra nội dung** và có nút
**Đọc lại bình luận**. Nút này chỉ kiểm tra, không gõ hay gửi thêm. Hết giờ chạy
vẫn có thể hoàn tất xác minh câu đã gửi. Bình luận nằm trong vùng bị ẩn có thể hiện
khác nhau giữa các tài khoản; app báo chưa tìm thấy câu cha thay vì tự chọn câu khác.

`Riêng lẻ` cho phép một máy và một bình luận; kiểu chuỗi vẫn cần ít nhất hai máy.
Tim/Lưu và bình luận thủ công không phụ thuộc cấu hình AI. “Bỏ qua: chưa đọc được
trạng thái” không có nghĩa đã Tim/Lưu; xem lý do và bằng chứng trước khi chạy lượt mới.
Nếu tên hiển thị khác handle, ứng dụng có thể đối chiếu link từ chính bài trước hành động.
**Follow** theo dõi đúng tác giả của bài đích trên phiên bản đã hỗ trợ. App kiểm nick
gán và profile trước thao tác; đã theo dõi thì bỏ qua, không đảo trạng thái. Sau khi bấm,
app mở lại profile chuẩn và đọc trạng thái trước khi xác nhận thành công. Nếu TikTok
hiện Following tạm thời rồi mất sau khi mở lại, kết quả giữ chưa chắc chắn và không bấm lại. Nếu kết
quả chưa rõ, dùng **Kiểm tra lại kết quả** để đọc trạng thái hiện tại; nút này không
Follow thêm và không xóa trạng thái chưa chắc chắn trong lịch sử. Account trên máy
khác nick đã gán sẽ chặn tương tác; kiểm lại máy và nick trước khi tạo lượt mới.

Công tắc **Tim bình luận của máy trước** cho máy trả lời tim đúng bình luận mà nó sắp
trả lời, trước khi bấm Trả lời. Tim là bật/tắt nên app đọc trạng thái trước: bình luận
đã được tim thì bỏ qua chứ không bỏ tim của người khác; không đọc được trạng thái thì
không bấm. Bình luận gốc của lượt không có ai để tim. Máy không đọc được nút tim (bản
TikTok chưa đo, hoặc chạy bằng đường pixel) thì bỏ qua và ghi lý do — câu trả lời vẫn
được gửi. Kết quả tim hiện thành dòng riêng trong **Theo dõi**, tách khỏi Tim của bài.

**Giữ bài (giây)** là số giây máy ở lại bài sau khi đã làm xong hành động, trước khi
app đóng TikTok; để trống hoặc 0 là rời ngay, tối đa 60 giây.

Danh sách bình luận thủ công chỉ cần đủ số câu bằng số bình luận mỗi link ở kiểu
`Nối tiếp`, vì ở đó câu sau trả lời câu trước; `Toả` và `Riêng lẻ` không có quan hệ đó
nên danh sách quay vòng cho đủ lượt, dùng ít câu hơn vẫn chạy.

Ô tài khoản cạnh từng máy nhận **username TikTok**, ví dụ `@ten.nick`, không phải
tên hiển thị. Tên/số máy vẫn giữ riêng; nick được gắn với định danh máy. Rời ô sẽ lưu;
nếu báo lỗi, sửa hoặc bấm **Tải lại nick đã lưu** trước khi chạy. Không gán cùng nick
cho hai máy để tránh nhầm actor khi tag. Nick lưu trong Riviu chưa chứng minh máy
đang đăng nhập tài khoản đó; sau khi đổi tài khoản trên TikTok cần đối chiếu lại.

Trước khi gửi bình luận trên Android, Riviu đọc username từ hồ sơ đang đăng nhập
và cập nhật nick của máy; số máy, tên, nhóm và ghi chú được giữ nguyên. Reply có
bật tag dùng username đã đọc của người được trả lời. Chưa xác định được username
thì lượt dừng trước Gửi, thay vì âm thầm bỏ tag. Kiểm tra lại bình luận cũ thiếu
username dùng nick đã đối chiếu, rồi vẫn kiểm tra hồ sơ tác giả của bình luận đó.

**Đọc tài khoản từ máy** mở Hồ sơ, đối chiếu nick đã gán với nick quan sát được,
báo Khớp/Lệch/Chưa đọc được và thời điểm. Không tự đổi tài khoản hoặc ghi đè nick.
Phạm vi đọc tài khoản theo capability/catalog của package, phiên bản và ngôn ngữ
đang chạy; xem **Chi tiết thiết bị → Kiểm tra khả năng**. Không dùng một danh sách
phiên bản chép trong tài liệu làm chứng nhận cho máy hiện tại.

Tương tác nhận link trực tiếp, mỗi dòng một bài; phần nhập Google Sheet đã bỏ khỏi
cả trang chính và cửa sổ nổi. Ba bước Chọn bài viết → Hành động & máy → Kiểm tra & chạy
giữ cấu hình khi chuyển bước. Phạm vi máy hiển thị thành một dòng riêng phía trên
chọn tất cả/bỏ chọn; xem bảng phân công trước khi xác nhận chạy.

Với lượt **Chưa chắc kết quả**, **Kiểm tra lại kết quả** chỉ mở đúng bài để đọc lại
Tim/Lưu. Không gửi lại, không thay lịch sử uncertain. Bình luận chưa được kiểm lại
bằng nút này; giữ bằng chứng đã gửi và không tự retry vì thiếu kết quả readback.

**Kết quả:** campaign, assignment, prepared content và trạng thái effect. Đổi URL rồi
parse lỗi không được dùng target cũ. **Tiếp theo:** mở bằng chứng cho uncertain; retry chỉ
phần được hệ thống xác định còn hợp lệ, không gửi lại một comment chỉ vì nhận ACK không rõ.
