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

Sau khi Nuôi TikTok, Tương tác hoặc Đăng bài kết thúc, Riviu tự đóng TikTok trên
**từng điện thoại khi máy đó đã hết việc**; Riviu trên PC vẫn chạy. Máy còn lượt
đang chạy/chờ trong cùng quy trình, bài đang tải hoặc chưa xác minh được link sẽ
chờ. Với bước Điều phối chạy theo nhóm, máy chờ bước hiện tại chốt kết quả.
Yêu cầu đóng được lưu qua restart, chỉ ghi đã đóng khi thiết bị trả bằng chứng
TikTok không còn chạy. Lỗi đóng không đổi kết quả đăng hoặc tự đăng lại bài.

Trong Đăng bài, thẻ máy hiện ngay bài còn chờ lấy link và nút **Xem bài đang chờ**.
Bài đang tải/chưa rõ kết quả chặn lượt đăng mới trên máy đó. Bài cũ được xác nhận
không còn giữ máy vẫn hiện cảnh báo kiểm tra link, theo điều kiện của lượt cũ.
Không đọc được trạng thái thì chờ kiểm tra lại trước khi đăng.

## Thiết bị

Sidebar giữ chiều rộng và tên mục. Bấm tiêu đề **Automation**, **Tài nguyên** hoặc
**Hệ thống** để ẩn/hiện các mục trong nhóm; lựa chọn được giữ cho lần mở sau.
**Nuôi TikTok**, **Tương tác**, **Đăng bài** trên sidebar mở ba trang vận hành cũ.
Trong **My Apps**, tên ứng dụng và **Mở chức năng** cũng mở trang cũ;
**Thêm Flow** mở trình kéo thả bổ sung của ứng dụng.
Control Center chỉ chứa quản lý thiết bị. Sidebar bỏ Dữ liệu và Mạng & Router. Trang **Lượt chạy** hiển thị bảng;
chọn tên hoặc Chi tiết để mở ngăn kết quả. Monitor rỗng tự ẩn.

Trong Control Center, bảng **Hiển thị** có thanh đổi kích thước ô xem trước và màn
hình điều khiển; hình điện thoại phóng theo ô và giữ tỷ lệ. Rê chuột vào thẻ **Hiển thị**
để bung bảng nổi, đưa chuột ra ngoài để tự ẩn. Bấm **Ghim bảng Hiển thị** để giữ bảng
cạnh lưới; bỏ ghim để trở lại hover. Bàn phím mở bảng bằng Tab/Enter, Escape đóng
bảng chưa ghim. Dấu **+** cạnh **Nhóm thiết bị** mở quản lý để tạo và phân máy vào nhóm.
Bấm tên nhóm để lọc lưới và bung/thu các số máy thuộc nhóm; số bên phải là đã chọn/tổng.
Các lựa chọn hiển thị được giữ cho lần mở sau. Chất lượng/FPS Android áp dụng
ngay khi thả thanh trượt; lỗi đọc hoặc áp dụng hiển thị tại bảng. Bộ lọc USB/Wi-Fi, nhóm và bảng số
máy chỉ thay tập đang xem/chọn, không tự thay phạm vi chiến dịch đã cấu hình. Trên
màn hẹp, bảng này giữ bên trái với chiều rộng gọn hơn; sidebar vẫn luôn có chữ.
Khi máy tính có nhiều ADB server, Riviu đọc và gộp thiết bị ở cổng 5037 và 5038;
mỗi máy dùng cổng đang quản lý máy đó cho cả điều khiển và stream. Vì vậy dàn máy
chia giữa hai server vẫn hiện đủ trong một lưới. Có thể giới hạn một cổng bằng
`RIVIU_ADB_SERVER_PORT`.

Cửa sổ điện thoại có màn hình bên trái, bảng lệnh bên phải, phím điều hướng cố định
phía dưới. Chỉ có một cửa sổ phóng to: chọn máy B khi A đang mở sẽ chuyển sang B
trong cùng cửa sổ và trả phiên điều khiển A. Ctrl/Shift chọn nhiều máy vẫn chỉ đổi
phạm vi chọn. Bật **Đồng bộ** tự mở máy chính, chuẩn bị phiên cho toàn bộ nhóm và chỉ
nhận chạm, vuốt, lăn chuột, phím hoặc chữ khi trạng thái là **Đang hoạt động N/N**.
Kéo tiêu đề có số máy để di chuyển, kéo góc dưới phải để đổi kích thước. Trang chính
vẫn dùng được khi cửa sổ mở. Nút Đóng hoặc Escape đóng cửa sổ đang thao tác. Tìm chức năng lọc toàn bộ bảng;
các lệnh chưa dùng được vẫn hiện lý do/trạng thái theo máy.

**PC → Điện thoại** mở hộp chọn một/nhiều tệp trên PC, sau đó mở thư mục điện thoại.
Chọn thư mục đích rồi bấm **Đưa vào thư mục này**. Nếu có tệp lỗi, bấm lại chỉ gửi
những tệp chưa thành công. **Điện thoại → PC** mở thư mục điện thoại để đánh dấu
tệp/thư mục, rồi **Lấy về máy tính** mở hộp chọn nơi lưu trên PC. Hai chiều có tiêu đề
và nút riêng. **Tệp trên máy…** vẫn mở bảng quản lý đủ cả đưa/lấy/xoá.
Bảng tệp căn trái tên, giữ khung gọn khi tải và có chuyển động mở/đóng.

### Riviu Helper trên Android

Khi phát hiện điện thoại Android đã cho phép gỡ lỗi USB, Riviu Manager tự kiểm tra
**Riviu Helper** và cài app nếu thiếu, hoặc cập nhật bản cũ chưa có biểu tượng. App
hiển thị tên **Riviu Helper** cùng logo Riviu trong danh sách ứng dụng. Mở app để xem
chức năng và trạng thái dịch vụ; thao tác này không đổi bàn phím hoặc mở quyền mới.

Việc chuẩn bị chạy nền tối đa hai máy cùng lúc. Máy đang có tác vụ hoặc được mở điều
khiển sẽ chờ đến khi rảnh; stream vẫn giữ nguyên. Máy đã có bản phù hợp được dùng lại,
không cài lại mỗi lần quét. Cài xong phải đọc lại phiên bản và biểu tượng mới tính là
hoàn tất. Dịch vụ clipboard/media chỉ khởi động khi một chức năng cần sử dụng.

Nếu điện thoại chặn **Cài đặt qua USB**, ứng dụng ghi rõ lỗi ở máy và **Dữ liệu → Nhật
ký thao tác**. Bật quyền cài qua USB trên điện thoại rồi ngắt/kết nối lại để thử lại.
Mỗi kết nối chỉ có một lần thử cài, không lặp cài liên tục khi bị từ chối. Máy chưa
chấp nhận gỡ lỗi USB hoặc mất kết nối chưa bắt đầu cài.

Thanh công cụ đặt **Mở máy**, **Đồng bộ**, **Nhóm** và **Công cụ** cạnh phạm vi máy.
**Đồng bộ** mở bảng chọn máy chính, xem từng máy nhận cùng trạng thái phiên và bật/tắt
đồng bộ. Chọn ít nhất hai máy đang kết nối rồi bấm bật; cửa sổ máy chính tự mở. Phạm
vi được khóa cho lượt đó: đổi lựa chọn, máy chính hoặc roster sẽ tắt đồng bộ, giữ cửa
sổ máy chính ở chế độ một máy và yêu cầu kiểm tra rồi bật lại. Lỗi một máy chuyển nhóm
sang **Cần xử lý**; **Thử lại điều khiển** chỉ mở lại phiên, không phát lại thao tác cũ.
Mục **Độ trễ và độ lệch thao tác** dùng chung cấu hình trong Cài đặt; bấm Áp dụng để
lưu. Máy chính luôn nhận ngay tại đúng tọa độ; độ trễ và độ lệch chỉ áp cho máy nhận.
Mở hoặc đóng bảng không tự bật đồng bộ.

Chuột phải trên ô máy hoặc ngay trên màn hình stream, chọn **Đọc và gán nick TikTok**
để mở Hồ sơ và lưu username vào danh sách thiết bị. Chuột phải trên một máy đã chọn
sẽ đọc lần lượt toàn bộ nhóm đang chọn; máy ngoài nhóm chỉ đọc riêng máy đó.
Username hiện thêm dưới tên máy, không thay số máy, tên máy hay trạng thái.
Máy đọc lỗi, mất kết nối, username trùng hoặc vừa được sửa sẽ báo riêng và giữ nick cũ;
các máy còn lại vẫn tiếp tục. Android đang mở điều khiển dùng lại phiên hiện có;
máy còn bài đăng cần giữ chưa được mở Hồ sơ để đọc nick. iOS chưa hỗ trợ thao tác này.
Lệnh **Sửa Riviu Agent** nằm trong **Bảo trì**; hộp xác nhận nêu rõ số máy và việc
khởi động lại stream. Nút quét thiết bị nằm bên phải toolbar. Trạng thái **Toàn hệ
thống** trên header khác phạm vi **Máy thực hiện** của từng tác vụ.

Các cửa sổ chi tiết giữ thao tác bàn phím bên trong; Escape đóng cửa sổ trên cùng
và trả focus về nơi mở. Cửa sổ tiến trình cho phép tiếp tục làm việc với trang chính.

### Ghi Macro

**Bắt thuộc tính & ghi Flow:** mở màn hình một máy Android, chọn mục cùng tên trong
menu bên phải. Rê chuột để tô phần tử; bấm chọn chỉ xem chữ, mô tả, ID và loại.
**Bấm và kiểm tra kết quả** mới gửi thao tác. Bật **Bắt đầu ghi**, thực hiện các bước,
**Dừng ghi** rồi **Lưu thành Flow**. Mở Flow thiết bị để chỉnh selector/hậu điều kiện
và chạy. Phiên có bước chưa thấy phần tử kết quả cần được kiểm tra trước khi lưu.
Macro tọa độ bên dưới vẫn dùng cho thao tác cử chỉ; Flow Inspector tìm lại phần tử.

Agent có thể dùng MCP Riviu khi API cục bộ đã bật. Chạy `npm run agent-mcp` trong
apps/desktop với `RIVIU_API_TOKEN` và `RIVIU_API_URL` từ cấu hình API. Các tool
riviu_devices, riviu_observe, riviu_tap, riviu_record và riviu_recording dùng cùng
phiên/ghi nhận với Inspector. Mẫu chỉ dẫn:

```text
Chọn đúng UDID được yêu cầu. Quan sát màn hình trước mỗi bước, dùng selector
duy nhất và kiểm tra phần tử kết quả. Ghi quy trình bằng riviu_record.
Giữ bước chưa xác minh để người vận hành xem; không tự lặp thao tác có thể đã gửi.
```

Vào **Công cụ → Macro**, nhập tên nếu cần rồi bấm **Bắt đầu ghi**. Hộp công cụ thu lại
thành thanh **Ghi Macro**, gồm số bước và **Dừng ghi**. Mở điện thoại để chạm, vuốt hoặc
bấm phím; thanh ghi nằm trong menu điều khiển, luôn ở ngoài vùng danh sách cuộn.
Đổi máy, đóng điện thoại, mất kết nối hoặc chuyển trang vẫn giữ phiên ghi; khi không mở
điện thoại, thanh ghi nằm dưới header ứng dụng. Escape đóng điện thoại, không dừng ghi.

Bấm **Dừng ghi** để trở lại đúng tab Macro và lưu. Tên nháp, số vòng và các bước được giữ
nguyên; điện thoại đang mở vẫn ở phía dưới hộp công cụ. Nếu chưa có bước nào, nút lưu
chưa bật. Khi dừng ở trang khác, hộp chỉ hiển thị Macro; đóng hộp rồi mở Công cụ tại
Thiết bị để dùng các công cụ khác.

Máy đích để phát lại được chốt khi bắt đầu phiên ghi. Đổi lựa chọn trong lưới hoặc máy
vừa kết nối không tự mở rộng phạm vi này. Phiên bắt đầu khi không có máy đích vẫn có
thể lưu Macro để dùng sau, nhưng không phát lại lên máy vừa xuất hiện. Bắt đầu/dừng/lưu
bản ghi không tự phát lại thao tác; nút **Chạy** là hành động riêng. Bản ghi đang thu
giữ trong phiên ứng dụng; lưu thành Macro trước khi đóng ứng dụng để giữ lâu dài.

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
**Chọn tất cả sẵn sàng** chọn các máy sẵn sàng trong danh sách, kể cả ngoài kết quả tìm kiếm;
**Bỏ chọn** xóa lựa chọn. Máy bận/chưa sẵn sàng không được chọn thêm. Ô máy tự gọn
khi danh sách đông; cuộn trong khung khi cần, số máy luôn giữ nguyên khi tìm kiếm.
Ô **Phạm vi thiết bị** nằm cùng hàng Chọn tất cả sẵn sàng/Bỏ chọn để chọn nhóm hoặc toàn bộ.
Khung máy đã thu hẹp, phần cấu hình phiên có thêm chỗ hiển thị.
Header ghi **Đã chọn X** cùng số máy sẵn sàng và tổng máy; tên/model mỗi máy hiện một lần. Cuộn trong danh sách
máy hoặc phần cấu hình độc lập, thanh tab và nút bắt đầu giữ ở vị trí cố định.

Nút **Dừng** cạnh danh sách trong cửa sổ **Theo dõi tác vụ** dừng toàn bộ máy của
tác vụ đang chọn và đóng TikTok về màn hình chính. Đây là dừng/hủy phần việc còn lại,
không phải tạm ngưng để tự chạy tiếp. App chờ thao tác đang thực hiện trả quyền máy,
báo từng máy đã đóng hoặc cần kiểm tra. Kết quả đã đăng/gửi và bằng chứng vẫn giữ;
kiểm tra link tự động của tác vụ Đăng bài đã dừng cũng được dừng qua lần mở app sau.
Nếu máy đang có tác vụ khác, app báo rõ thay vì đóng ứng dụng của tác vụ đó.

**Thiết lập** có các tab trực tiếp **Phiên nuôi · Hành vi · AI**. Lịch sử bình luận
và token AI nằm trong **Theo dõi → Bình luận & chi phí AI**, bên cạnh **Tiến độ máy**.
Nút **Sửa thiết lập** mở đúng tab và đưa focus về trường cần sửa.
Mỗi máy hiển thị trạng thái bằng chữ cùng lý do khi chưa sẵn sàng. Tab **Hẹn giờ** chứa lịch
tự chạy và các khung giờ riêng; bấm **Áp dụng hẹn giờ** để lưu lịch xuống ứng dụng.
Chuyển tab hoặc sửa bản nháp chưa bắt đầu phiên; **Kiểm tra & bắt đầu** vẫn là
thao tác riêng. Giữ máy tính và Riviu đang mở để lịch chạy.

Trang Thiết lập đặt cấu hình phiên cạnh danh sách máy. Bộ tỷ lệ mặc định là Tim 20%,
Lưu 5%, Bình luận 2%, Theo dõi 1%; khi bật đủ bốn hành động, **Chỉ xem là 72%**.
Mỗi video chọn tối đa một hành động. Phần Chỉ xem tự bù để tổng luôn là 100%; tắt
một hành động trả tỷ lệ về Chỉ xem và giữ số đã đặt. Bình luận cần bật riêng khi có AI.
Hồ sơ cũ chỉ chuyển sang phân bổ này khi bạn lưu bản thiết lập đã duyệt; lượt đang
chạy giữ chế độ của mình. Chọn máy rồi bấm **Kiểm tra & bắt đầu**.
Thời lượng trên trang được gửi vào phiên thật. Lịch nằm trong **Hẹn giờ**; Theo dõi
có log riêng từng máy.

Trong **Phiên nuôi → Nguồn video**, chọn **Lướt theo từ khóa**, nhập ví dụ `đà lạt`.
Android mở Tìm kiếm, nhập đúng từ khóa, chuyển sang Videos và mở một kết quả để lướt.
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
riêng bài đã bấm Đăng nhưng chưa xác minh được liên kết giữ TikTok chạy và giữ nội dung
đã chuyển để quá trình tải tiếp tục. Máy còn bài đang tải hoặc chưa rõ kết quả Đăng
chưa bắt đầu phiên tự động mới có bước tắt TikTok. Riêng bản ghi đã có `Posted`, đã
dừng tự xác minh và không có pipeline hoạt động: sau ít nhất 4 giờ kể từ cập nhật cuối,
việc thiếu link chuyển thành lưu ý **Liên kết bài cũ**, không giữ máy mãi. Bài cũ vẫn
cần đối soát trong Theo dõi, không được đưa lại vào lượt đăng hay báo Sheet thành công.
Lỗi dọn được báo riêng. Lượt đã qua Send/Post mà chưa rõ kết quả
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

**Hội thoại theo kịch bản** cho mỗi link một nội dung riêng. Chọn bài trong mục kịch bản,
dán các dòng `@vai: nội dung`, bấm Phân tích rồi sửa câu, chủ đề, parent và tag.
AI có thể soạn trước từ mô tả/caption do bạn nhập; toàn bộ câu vẫn phải duyệt trước chạy.
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
Hiện đã đo `trill 38.3.2/en`; bản/ngôn ngữ khác cần hiệu chỉnh trước.

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

## Đăng bài

Mục **Giới hạn chạy đồng thời** trên trang Đăng bài cho phép xem và lưu giới hạn
chuyển media, thao tác TikTok, xác minh link và tổng lượt điều khiển theo máy chủ.
Sau khi đổi giới hạn, kiểm tra lại lịch trước khi lưu; lịch quá tải có cảnh báo.

Đăng ngay và hẹn giờ dùng chung hàng chờ bền vững theo từng bài/máy. Mặc định toàn
ứng dụng có 4 lượt chuyển media, 4 lượt thao tác TikTok, 4 lượt xác minh liên kết;
tổng lượt điều khiển thiết bị tối đa 8. Sheet có tối đa 2 request, trong đó tối đa
1 request báo tiến độ. Theo dõi hiển thị giai đoạn, thời điểm vào hàng và lý do chờ.
Sau khi gửi, app trả lượt thao tác TikTok rồi xác minh riêng; retry trước gửi giữ
nguyên bài dự kiến, retry lấy link hoặc ghi Sheet không gửi bài lần nữa.

Giờ hẹn là lúc bắt đầu xử lý. Bài chưa được cấp lượt trong 30 giây hoặc app mở lại
sau giờ hẹn được ghi **Lỡ lịch**, không tự đăng bù. Công việc đã bắt đầu đúng cửa
sổ tiếp tục, và bài đã gửi vẫn được xác minh. Kiểm tra lịch cảnh báo số bài cùng
thời điểm vượt số lượt chuyển media đang cấu hình.

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

Các tab chính là **Thiết lập · Hẹn giờ · Theo dõi**. Thiết lập mở **Bàn đăng nhanh**:
nội dung, caption/đối tác và máy thực hiện nằm trên cùng màn hình. Ở cửa sổ nhỏ,
cuộn trong vùng làm việc để xem phần dưới; nút **Kiểm tra & đăng** ở cuối vẫn giữ vị trí.
Thanh đầu đặt **Chọn thư mục** cạnh **Quét**. Phần Google Sheets chỉ gồm ô
**Link Google Sheet**, nút **Đăng nhập Google** và nút **Kiểm tra kết nối**.
Dán link của đúng tab (`gid` trong link), đăng nhập trên trình duyệt rồi kiểm tra.
Link không có `gid` dùng tab `0`. Sửa link sẽ bỏ trạng thái sẵn sàng của link cũ.

Với bảng chưa được cấp quyền, nút kiểm tra mở Google Picker; chọn đúng bảng vừa
dán để cấp quyền và tiếp tục. Chọn một bảng khác sẽ báo lỗi, không tự thay link
hoặc ghi sang bảng đó. Không cần chọn tab lần nữa nếu link đã chứa `gid`.
Đăng nhập xong chưa có nghĩa bảng đã sẵn sàng ghi: chờ trạng thái xác minh màu
xanh. Tab hoàn toàn trống được chuẩn bị header chuẩn, không thêm hàng thử.

Mục Apps Script cũ và các bước chọn tab riêng đã được bỏ khỏi màn Đăng bài.
Bản release mới mang sẵn cấu hình ứng dụng Google; mỗi máy vẫn đăng nhập tài khoản
Google riêng rồi kiểm tra đúng Sheet. Quy trình build dừng nếu thiếu cấu hình.
Chép `.exe` hoặc thư mục app không chép phiên đăng nhập đã lưu trong kho
thông tin xác thực của máy cũ. Nếu dùng bản dev chưa có cấu hình, mở **Thiết lập Google**
ngay dưới ô kết nối, nhập OAuth Client ID loại Desktop, client secret (nếu có),
Picker API key và project number do quản trị viên cung cấp rồi bấm **Lưu cấu hình Google**.
Các trường này thuộc ứng dụng, không phải mật khẩu tài khoản. Sau khi đủ cấu hình,
form tự ẩn; tiếp tục **Đăng nhập Google** rồi **Kiểm tra kết nối**.
Phiên đăng nhập và cấu hình nhập tay lưu trong kho xác thực hệ điều hành. Trong lúc
đăng nhập hoặc chọn bảng, có thể hủy bằng nút Google; lượt chuyển kết nối đã được
nhận sẽ hoàn tất hoặc giữ trạng thái để tiếp tục. Một tab chỉ nhận ghi từ PC đã
liên kết. Khi chuyển sang PC mới, chọn tab Sheet mới; app chưa có thao tác bàn giao
tab đang thuộc PC cũ. Chọn nhầm tab đó sẽ báo rõ xung đột và giữ kết nối hiện tại,
không làm kẹt việc chọn tab khác.
Khi đã nhập link, cần xác minh thành công trước khi đăng hoặc lưu lịch có bật ghi
Sheet. Có thể tắt **Ghi kết quả lên Sheet** để chạy lượt không ghi bảng.
Khi kiểm tra trước đăng hoặc lưu lịch, Riviu chốt đúng bảng/tab và chế độ báo cáo
của lượt đó. Đổi kết nối cho lượt mới không chuyển các bài cũ sang bảng khác.
Lượt cũ chưa có đích đã chốt giữ nguyên lịch sử để kiểm tra, không tự gửi sang
kết nối mới.

Khung **Máy thực hiện** có nút **Chọn nhanh**: lấy bài trong nguồn (tối đa 100) và
mọi máy sẵn sàng trong phạm vi; mỗi máy một bài. Cặp đã ghép hợp lệ được giữ. Nếu đợt
trước chỉ gán một phần mà sau đó thêm máy sẵn sàng, bấm lại sẽ lấp tiếp máy trống bằng
bài còn trong nguồn — không kẹt ở tập bài/máy của lần gán cũ. Bài đang chọn chưa đủ máy
vẫn chỉ dùng tập đó cho đến khi đã ghép xong. Thiếu máy thì ghép phần đủ và báo số bài
còn lại vì hết máy. **Hoàn tác** trả lại lần gán nhanh gần nhất nếu chưa sửa ánh xạ.
Bỏ tick máy gỡ bài của máy đó; **Máy nhận bài đang chỉnh** cho đổi riêng từng bài.
Dòng tổng kết và hộp kiểm tra ghi rõ Sheet đang bật hay tắt. Có thể Hủy khi đang
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
lượt kiểm tra chủ động sẽ lưu lịch kiểm tra lại mỗi 5 phút, kể cả sau khi mở lại
app. Bài thiếu bằng chứng hoặc đang ở màn nháp/soạn bài vẫn cần kiểm tra riêng;
app không gửi lại bài đã có intent Đăng.

Khi tìm link, app đọc toàn bộ caption, tài khoản và thời gian của bài. Caption bị
rút gọn chỉ được mở khi chính vùng caption có thể bấm và nhận diện đúng; sau đó
đọc lại nội dung. Có nhiều bài cùng khớp hoặc chưa đủ bằng chứng thì giữ trạng thái
cần kiểm tra. App không tự bấm Đăng lần nữa để giải quyết lỗi lấy link.

Sau Đăng, Theo dõi hiển thị lý do chưa xác minh, lần kiểm gần nhất và lần kiểm
kế tiếp. App giữ TikTok/media khi chờ; link xác minh xong được gửi Sheet ngay.
Lỗi đọc clipboard được hiển thị riêng với trường hợp đọc thành công nhưng chưa
có link mới. Thông báo TikTok đang xử lý có ảnh đối chiếu sau Copy; trạng thái
này vẫn là chờ, chưa tính là xuất bản hoặc ghi link thành công.
Khi TikTok hiện “Post is being processed” sau Sao chép liên kết và app đọc được
thông báo đó, Theo dõi ghi **TikTok báo bài đang được xử lý**. Nếu không đọc được
thông báo, app chỉ ghi **TikTok chưa trả link**; không suy đoán bài đã xuất bản.
Sau khi đọc đủ caption của bài đang mở, lỗi sao chép được giữ làm lý do chờ;
app không quay sang tìm bài khác rồi ghi đè bằng lỗi caption hoặc hồ sơ.
Với bài **hẹn giờ**, phiên đăng trả quyền điều khiển trước khi lấy link. Lần kiểm
tra đầu sau 2 phút kể từ Đăng. Bài đã gửi nhưng chưa có link, kể cả **đăng ngay**,
được kiểm tra lại mỗi 5 phút đến khi xác minh được link. Nhịp chờ tính từ lúc kết
thúc lần kiểm tra trước; lỗi đọc/kết nối vẫn chờ 5 phút. Mở lại app giữ mốc Đăng và
lần kiểm tra tiếp theo. Máy mất kết nối hoặc đang bận được kiểm khi sẵn sàng.
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
tạo hàng chờ hoặc gửi webhook. Bật thì phải kết nối thành công trước lượt mới;
nếu kết nối mất sau khi đăng, link đã xác minh được giữ để gửi lại đúng bảng/tab.
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

### Tiếp tục xác minh bài đã gửi sau khi Dừng

Trong **Theo dõi → Chi tiết máy**, bài thuộc chiến dịch đã Dừng có thể hiện
**Tiếp tục xác minh bài đã gửi** khi backend xác nhận còn đủ identity của lần Đăng.
Đọc xác nhận trước khi tiếp tục: chỉ kiểm tra bài cũ trên máy đó, không đăng lại,
không tiếp tục những bài chưa gửi của máy khác. Thiếu tài khoản/thời điểm gửi hoặc
cần kiểm tra thủ công vì lý do khác thì không được mở quyền này. Nếu trạng thái
vừa thay đổi, app yêu cầu đọc lại thay vì áp dụng xác nhận cũ.
Nếu tác vụ vẫn đang Dừng/nhả máy hoặc lần dừng bị gián đoạn, hoàn tất **Dừng** trước
khi tiếp tục xác minh. Không mở lại observer trong khi lệnh đóng phiên cũ còn chạy.

Nhận yêu cầu không có nghĩa đã lấy được link. App dùng worker hiện có để kiểm tra
lại sau mỗi 5 phút tính từ cuối lần kiểm trước, kể cả sau restart; máy offline/bận
chờ khi sẵn sàng. Campaign vẫn có thể mang nhãn **Đã huỷ** dù một bài trong đó đang
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
Xem lệnh và tham số trong [hướng dẫn phát triển](developer-guide.md#nghiệm-thu-publish-qua-ứng-dụng-đang-chạy).

Đọc từng mốc, không gộp thành một dấu thành công:

1. **Enqueued**: đã thấy công việc trong pipeline; chưa chứng minh Đăng.
2. **Submitted**: receipt đã gửi; chưa chứng minh TikTok cấp link.
3. **Verified**: backend lưu proof xuất bản và canonical URL; chỉ URL hiện trên dòng
   hoặc state succeeded cũ không đủ.
4. **Sheet sent**: backend đã settle delivery, tách khỏi trạng thái bài.
5. **URL readback**: đối chứng đọc ô Sheet. Harness hiện chưa có IPC đọc lại hàng
   private/receipt identity, nên báo **unsupported**, không xuất token và không giả
   xanh. CSV công khai cũng không thay bằng chứng đúng writer/target/epoch.

Mã thoát `2` là còn chờ/hết hạn hoặc ACK chưa rõ; `1` là lỗi/bị chặn; `3` là đã có
verified + sent nhưng chưa đủ readback bổ sung. `0` của inspect/preflight chỉ nghĩa
bước đọc/kiểm đầu vào xong, **không** là nghiệm thu post→Sheet. Khi mất ACK tạo, giữ
nguyên report-dir và requestId đã lưu; kỹ thuật tiếp tục cùng request. Đã có Execute
intent thì harness không tự gửi Execute lần nữa, kể cả chưa biết backend đã nhận.
Không xóa intent, tạo thư mục khác hoặc đổi requestId để “thử lại”. Bài pending được
app kiểm theo lịch hiện có; không đóng TikTok hoặc gửi lại để giải quyết thiếu link.

Canary Rust cũ chỉ dùng cô lập để khảo sát/rehearsal, Sheet tắt và thiếu worker đầy
đủ; không dùng làm chứng cứ nghiệm thu OAuth end-to-end. Không chạy nó song song
Riviu trên cùng điện thoại hoặc nhập DB thử vào dữ liệu đang vận hành.

## Flow

**My Apps → Mở trình thiết kế** mở graph riêng của ứng dụng. Thư viện bước ở bên trái,
canvas ở giữa; chọn bước để chỉnh thông số bên phải. Kéo bước, nối cổng, xóa bước,
hoàn tác/làm lại và lưu tạo revision. Tìm node bằng tên/ID rồi Enter để đưa vào vùng nhìn.
**Cấu hình ứng dụng** nhận hồ sơ đã lưu hoặc nguồn/link/nội dung đầu vào. **Chạy** yêu
cầu lưu trước và chọn phạm vi thiết bị; theo dõi và hủy trong bảng chạy.

Nuôi TikTok dùng thứ tự Thích/Lưu/Bình luận đã nối, tỷ lệ từng bước, giới hạn xem và
thời lượng. Các bước chuẩn bị/xác minh trong Tương tác/Đăng bài giữ thứ tự phụ thuộc
của engine; nối sai bị báo khi kiểm tra/lưu. Chờ và Ghi nhật ký đặt trước/sau pipeline;
lịch theo hồ sơ chưa nhận các bước Chờ/Ghi nhật ký bao quanh. Graph này chưa thay thế
thư viện thao tác thấp của **Flow thiết bị** và không chứng nhận tương đương mọi node GenFarmer.

**Tác vụ đã lưu** ghim ứng dụng/revision cùng máy thực hiện; **Lịch chạy** quản lý
lịch của các hồ sơ, hiển thị lần chạy kế tiếp và lỗi gần nhất. **Quản lý tài khoản**
giữ metadata, nhóm và máy; đọc tài khoản là thao tác riêng. Nhập/xuất JSON không gửi
lệnh vào điện thoại. **Mạng & Router** giữ hồ sơ kết nối, thử TCP và áp dụng/xóa proxy
HTTP không xác thực trên Android; kết quả phải đọc lại khớp trên từng máy.
Phần chợ ứng dụng chưa được thêm trong đợt này.

**Đầu vào:** graph thiết bị hoặc điều phối fleet, cấu hình node, target và profile.
**Thao tác:** chọn đúng chế độ; sửa graph; validate; lưu trước khi chạy; theo dõi execution
và từng node. Import/export JSON là thao tác bản nháp có guard; archive phải phản ánh
đúng identity đang mở.

Trong bảng bước có **Gán biến**, **Đọc văn bản**, **So sánh biến**, **Nếu thấy phần tử**
và **Ghi nhật ký**. Biến là chuỗi riêng của từng máy; tên chỉ gồm chữ Latin, số và
gạch dưới, bắt đầu bằng chữ hoặc gạch dưới. Gán/đọc biến trước khi dùng; hai nhánh
của bước so sánh phải được nối. Lịch sử hiển thị giá trị đã ghi và nhánh thực sự chạy.
Flow có thao tác UI vẫn cần **Mở ứng dụng** làm bước thao tác đầu tiên. Nếu thấy
phần tử hiện dùng cây đầy đủ trên Android; máy thiếu khả năng này báo rõ trước khi đọc.

Chọn bước **Đọc tệp**, **Ghi tệp**, **Gọi HTTP**, **Đọc ô Sheet** hoặc
**Ghi ô Sheet** rồi mở **Kết nối dữ liệu: tệp, HTTP và Google Sheet** trong bảng
thuộc tính để nhập tệp hoặc lưu token. Tệp nằm trong `flow-data` của Riviu; đường
dẫn tương đối như `inputs/posts.csv` chỉ đọc/ghi trong thư mục này. Mỗi kết quả tối
đa 4.096 ký tự, tệp/HTTP tối đa 16 KiB. CSV và Sheet trả bảng JSON gồm các hàng
chuỗi; ghi bảng nhận cùng dạng `[["Máy 1","Sẵn sàng"]]`. Đặt **Biến lưu kết quả**
rồi dùng `${ten_bien}` trong đường dẫn hoặc nội dung bước sau. Token HTTP lưu bằng
tên tham chiếu trong kho xác thực của hệ điều hành. HTTP gửi một lần; khi thiếu
phản hồi sau gửi, kiểm tra phía nhận trước khi tạo lượt chạy mới.

Các bước Sheet dùng kết nối đã cấu hình ở **Đăng bài**, yêu cầu link bảng, tên tab
chính xác và vùng A1 hữu hạn tối đa 1.000 ô. Cập nhật bản triển khai Apps Script
bằng [`publish-sheet.gs`](apps-script/publish-sheet.gs) đi kèm để bật đọc/ghi vùng;
giữ cấu hình bảng/token hiện có. Vùng phải nằm trong lưới đã có; ghi đủ số hàng,
số cột và đọc lại khớp mới báo thành công. Công thức nhập trong giá trị được lưu
như văn bản. Thiết lập connector không chạy Flow hay sửa bảng từ xa.

**Nhập Flow** nhận JSON Flow cũ, script GenFarmer hoặc Macro Riviu. Chọn định dạng,
đọc file/dán JSON, **Xem trước**, rồi **Mở bản nháp**. Import chỉ chuyển những bước
có hợp đồng tương thích; danh sách lỗi nêu bước cần sửa. Các bước tọa độ cần profile
hình học và điều kiện xác minh; dữ liệu thiếu được giữ để người dùng sửa từ nguồn,
không tự chọn tọa độ thay thế. Home nhập từ ngoài cần đặt đúng app launcher khi validate.
Ánh xạ ID nguồn hiện có trong màn hình xem trước; Flow lưu sau đó dùng ID Riviu.

**Lặp chuỗi hành động** tạo thêm bản sao body tuyến tính, tối đa 50 lượt và 500 bước. Mỗi bản sao
có ID và lịch sử riêng. Flow có rẽ nhánh cần chỉnh cấu trúc trước khi dùng chức năng
này. **Hoàn tác** trả lại toàn bộ graph trước khi lặp.

**Ghép hoặc chỉnh Flow con** lưu trọn nội dung một Flow vào bước **Flow con** hoặc
**Lặp Flow con**. Chọn Flow đã lưu và **Phiên bản nguồn**, bấm **Nạp đúng phiên bản**;
hoặc nạp tệp/chỉnh **JSON Flow con** trong hộp thoại. Bản nguồn đổi sau đó không đổi
nội dung đã ghép. Chọn bước đã ghép rồi mở lại nút này để sửa body hoặc ánh xạ biến.
Biến đầu vào dùng `biến con: biến cha`; đầu ra dùng `biến cha: biến con`. Mỗi lượt
có biến riêng, truyền đầu vào trước khi chạy body và trả đầu ra sau khi hoàn tất.
Ví dụ đầu vào `{"noiDung":"caption"}` lấy biến `caption` của cha vào `noiDung`
của con; đầu ra `{"ketQua":"daDoc"}` trả biến `daDoc` của con vào `ketQua` của cha.
Flow con hỗ trợ nhánh và lồng nhau, tối đa 8 cấp, 50 lượt cho mỗi bước lặp và
2.000 bước sau khi biên dịch. **Áp dụng Flow con** kiểm tra toàn bộ Flow cha trước
khi cập nhật bản nháp; vẫn cần lưu trước khi chạy.
Lịch sử hiển thị phiên bản Flow con và số lượt thực sự chạy; mở **Chi tiết** của
bước để đối chiếu ID nguồn, kể cả sau khi khởi động lại ứng dụng.

Khi chọn ảnh cho **Chạm theo ảnh / Nếu thấy ảnh**, cắt ảnh rồi dùng **Kiểm tra ảnh mẫu**
để xem vị trí, điểm khớp và các kết quả trùng trên ảnh đã chụp. Phép thử chạy cục bộ
và không thao tác điện thoại. **Dùng ảnh mẫu** mới ghi ảnh vào bước. Các tùy chọn
đa tỷ lệ của phép thử không thay config hoặc thuật toán thực thi Flow đã lưu.

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

Trang tách **Năng lực hiện tại** khỏi **Tác vụ trong 24 giờ qua**. Các số tác vụ dùng
đúng cửa sổ 24 giờ được ghi trên màn hình. **Nhật ký thao tác** hiển thị tối đa 200
bản ghi gần nhất; ô tìm kiếm chỉ lọc trong tập đã tải, xuất danh sách cũng chỉ xuất
kết quả đang lọc. Tra cứu lần chạy theo nguồn, trạng thái và khoảng ngày ở **Tác vụ**;
mở chi tiết rồi quay về workspace gốc để xử lý.

## API

**Đầu vào:** cấu hình listener, credential và địa chỉ client. **Thao tác:** lưu cấu hình
đúng vùng; đọc địa chỉ listener đang chạy, lỗi bind và yêu cầu restart riêng biệt.
**Kết quả:** trạng thái runtime, không chỉ giá trị trong form. **Tiếp theo:** xử lý port
bị chiếm hoặc restart theo chỉ báo; kiểm tra gọi API qua cùng admission/ownership với UI.
Bấm **Cấu hình kết nối** để mở thẳng nhóm **Kết nối và API** trong Cài đặt.

Trang **API** có danh mục HTTP theo chức năng và ví dụ PowerShell chạy, theo dõi,
dừng Flow. Tìm bằng tên lệnh hoặc đường dẫn. Phần tham chiếu runtime bên dưới dùng
qua Tauri invoke trong ứng dụng. HTTP chỉ lắng nghe trên `127.0.0.1`, mặc định cổng
`22222`; lấy địa chỉ thực tế ở **Trạng thái API**. Mọi request cần
`Authorization: Bearer TOKEN` với token lấy trong Cấu hình kết nối.

| Thao tác | HTTP |
|---|---|
| Danh mục action | `GET /v1/flows/catalog` |
| Flow đã lưu | `GET /v1/flows?includeArchived=false` |
| Đọc revision | `GET /v1/flows/{id}?revision=3` |
| Chạy Flow | `POST /v1/flows/{id}/runs` |
| Lịch sử Flow | `GET /v1/flow-runs?limit=100` (1–200) |
| Chi tiết lượt chạy | `GET /v1/flow-runs/{id}` |
| Yêu cầu dừng | `POST /v1/flow-runs/{id}/cancel`, body trống hoặc `{}` |
| Thiết bị / nhóm | `GET /v1/devices`, `GET /v1/groups` |
| Hàng đợi script | `GET /v1/jobs` (100 tác vụ mới nhất) |

Chạy Flow nhận body JSON như
`{"revision":3,"selection":{"mode":"selected","udids":["UDID_A","UDID_B"]}}`.
Thay ID, revision và UDID bằng dữ liệu đã đọc. Revision đã lưu là bất biến; bỏ
revision dùng bản mới nhất. `selection` bắt buộc: `one` nhận `udid`, `selected`
nhận 1–200 `udids` khác nhau, `allEligible` chọn mọi máy đủ điều kiện. Runtime
chụp danh sách máy khi nhận lượt chạy; đổi nhóm sau đó không đổi lượt đã nhận.

Phản hồi thành công có `ok: true` và `result`. Khi tạo lượt chạy, lưu `result.id`
rồi đọc chi tiết để theo dõi kết quả từng máy. `cancellationRequested: true` xác
nhận yêu cầu dừng; tiếp tục đọc tới trạng thái kết thúc. Nếu mất phản hồi POST,
tra lịch sử trước khi gửi lại để tránh tạo hai lượt. Khi lỗi, đọc `code`, `error`
và `details`: HTTP 400 sai dữ liệu, 401 sai token, 404 không tìm thấy, 409 máy bận
hoặc xung đột trạng thái, 503 ứng dụng đang đóng. Tổng header và body tối đa 64 KiB;
client gửi Content-Length. Run/cancel dùng chung admission và engine Flow với UI.

## Cài đặt

**Bảo trì → Riviu Agent** hiển thị phần mềm điều khiển của từng điện thoại. Android
dùng ADB/UiAutomator2 và helper Riviu; iPhone dùng Riviu Agent/WDA. Thông tin gói và
xác thực iOS nằm riêng trong **Cấu hình Agent iOS**. Khi chỉ dùng Android hoặc chưa có
iPhone đang kết nối, trang Thiết bị không hiện cảnh báo nhánh iOS; vẫn mở mục cấu hình
này để xem lý do nếu đang chuẩn bị kết nối iPhone. Khi có iPhone kết nối mà nhánh iOS
báo lỗi, cảnh báo hiện trên trang Thiết bị. Trạng thái này không thay kết quả kiểm tra
từng máy và không tự cài hoặc khôi phục Agent.

**Đầu vào:** giá trị của từng section và credential tương ứng. **Thao tác:** chỉnh sửa,
lưu/bỏ từng section; không coi text vừa nhập là đã persist. **Kết quả:** save status và
readback đúng section; phản hồi cũ không ghi đè bản nháp mới. **Tiếp theo:** áp restart
nếu thay đổi yêu cầu, hoặc quay lại vùng vừa sửa sau khi guard được giải quyết.

## Khi có lỗi

Ghi workspace, hồ sơ/revision, số/alias máy, thời điểm và execution ID. Mở bằng chứng
nguồn trước khi lặp lệnh. Không ghi token/password vào report. Khi kiểm tra WDA/iOS,
đọc [ràng buộc WDA](agents/02-wda-doc-truoc-khi-sua.md) và không chạy harness đồng thời
với desktop đang sở hữu thiết bị. Hướng dẫn này mô tả hợp đồng; nghiệm thu có ngày ở
[kho lịch sử](archive/README.md), không tự cấp chứng nhận cho mọi thiết bị/bản cài.


Khi dọn Sheet bị gián đoạn, kết nối hiển thị chưa sẵn sàng nhận bài. Tiếp tục cùng
đợt dọn để giữ bản sao và hoàn tất; các dòng mới được nhận sau khi đọc lại xác nhận
đã dọn. Gửi lại yêu cầu tạo bài bị mất phản hồi sẽ mở đúng chiến dịch đã tạo.
