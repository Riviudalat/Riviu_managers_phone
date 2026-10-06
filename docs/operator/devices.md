# Thiết bị

[Hướng dẫn vận hành](../operator-guide.md) · Chi tiết chức năng hiện hành; bảng khả năng và proof trên máy quyết định quyền chạy.

## Thiết bị

Sidebar mặc định hiện tên mục. Bấm tiêu đề **Automation**, **Tài nguyên** hoặc
**Hệ thống** để ẩn/hiện các mục trong nhóm; lựa chọn được giữ cho lần mở sau.
**Nuôi TikTok**, **Tương tác**, **Đăng bài** trên sidebar mở ba trang vận hành cũ.
Trong **My Apps**, tên ứng dụng và **Mở chức năng** cũng mở trang cũ;
**Thêm Flow** mở trình kéo thả bổ sung của ứng dụng.
Control Center chỉ chứa quản lý thiết bị. Sidebar bỏ Dữ liệu và Mạng & Router. Trang **Lượt chạy** hiển thị bảng;
chọn tên hoặc Chi tiết để mở ngăn kết quả. Monitor rỗng tự ẩn.

Trong Control Center, bảng **Hiển thị** có thanh đổi kích thước ô xem trước và màn
hình điều khiển; hình điện thoại phóng theo ô và giữ tỷ lệ. Ảnh xem trước Android
tự mở khi máy kết nối, kể cả trong chế độ dev nghiệm thu thủ công; không cần mở
từng máy. Việc tự mở ảnh không khởi chạy
Nuôi, Tương tác, Đăng bài hay lịch đã lưu. Rê chuột vào thẻ **Hiển thị** để bung bảng
nổi, đưa chuột ra ngoài để tự ẩn. Bấm **Ghim bảng Hiển thị** để giữ bảng
cạnh lưới; bỏ ghim để trở lại hover. Bàn phím mở bảng bằng Tab/Enter, Escape đóng
bảng chưa ghim. Dấu **+** cạnh **Nhóm thiết bị** mở quản lý để tạo và phân máy vào nhóm.
Khi mất kết nối stream hoặc bộ giải mã dừng, ảnh cũ không còn được tính là live;
khung xem được dựng lại và chỉ báo live khi có frame mới. Bản cài dùng tài nguyên
Android đi kèm; biến môi trường trỏ APK/scrcpy của bản dev không thay tài nguyên bản cài.
Đóng cửa sổ điều khiển giữ chất lượng cao thêm tối đa hai giây trước khi hạ về
chất lượng lưới; mở lại trong khoảng này tránh khởi động lại stream. Khi thiết bị
báo mất kết nối, thao tác bị khóa và phiên cũ được nhả trước khi mở lại sau kết nối.
Bấm tên nhóm để lọc lưới và bung/thu các số máy thuộc nhóm; số bên phải là đã chọn/tổng.
Bộ lọc Tất cả/USB/WIFI hiển thị số máy của nhóm đang chọn, kể cả khi nhóm trống;
đổi bộ lọc không tự chọn hoặc điều khiển máy.
Các lựa chọn hiển thị được giữ cho lần mở sau. Chất lượng/FPS Android áp dụng
ngay khi thả thanh trượt; lỗi đọc hoặc áp dụng hiển thị tại bảng. Bộ lọc USB/Wi-Fi, nhóm và bảng số
máy chỉ thay tập đang xem/chọn, không tự thay phạm vi chiến dịch đã cấu hình. Trên
màn hẹp, bảng này giữ bên trái với chiều rộng gọn hơn; sidebar có thể thu gọn thành icon.
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

Trên Android, chạm vào ô nhập trên màn hình điện thoại rồi dùng bàn phím PC để
gõ tại con trỏ. Backspace xoá phía trước, Delete xoá phía sau; phím mũi tên,
Home/End và Shift hỗ trợ di chuyển/chọn chữ. Ctrl+A chọn tất cả, Ctrl+C/Ctrl+X
đưa nội dung sao chép/cắt từ điện thoại về clipboard PC, Ctrl+V dán vào điện thoại.
Enter/Tab và Ctrl+Z/Ctrl+Y được gửi xuống ứng dụng; hành vi phụ thuộc ô nhập và
khả năng hoàn tác của ứng dụng đó. Giữ phím xoá/mũi tên để lặp thao tác.

Bộ gõ tiếng Việt trên PC gửi phần chữ đã hoàn thành; giữ bàn phím Samsung/Gboard
trên điện thoại, không cần đổi sang bàn phím ADB. Nếu còn thanh Clear Text/Switch
IME, mở **Đổi bàn phím** và chọn bàn phím thông thường đã cài. Nhập Unicode/dán
có thể thay nội dung clipboard điện thoại. Copy không có vùng chọn giữ hành vi
clipboard của ứng dụng; không có kết quả mới không có nghĩa đã sao chép thành công.

Phím chỉ đến điện thoại khi vùng màn hình của nó đang được chọn và phiên điều
khiển sẵn sàng. Chuyển sang ô nhập của Riviu thì gõ/xoá chỉ tác động ô đó. Khi đổi
máy, mất focus hoặc ngắt luồng hình, phần phím chưa gửi bị huỷ. Nếu báo kết quả
chưa rõ, kiểm tra điện thoại rồi bấm lại màn hình để tiếp tục; không tự phát lại.
Bàn phím trực tiếp hiện dành cho một máy Android. Khi đồng bộ nhóm, dùng ô
**Nhập chữ** và các phím điều khiển nhóm.

**PC → Điện thoại** mở hộp chọn một/nhiều tệp trên PC, sau đó mở thư mục điện thoại.
Chọn thư mục đích rồi bấm **Đưa vào thư mục này**. Nếu có tệp lỗi, bấm lại chỉ gửi
những tệp chưa thành công. **Điện thoại → PC** mở thư mục điện thoại để đánh dấu
tệp/thư mục, rồi **Lấy về máy tính** mở hộp chọn nơi lưu trên PC. Hai chiều có tiêu đề
và nút riêng. **Tệp trên máy…** vẫn mở bảng quản lý đủ cả đưa/lấy/xoá.
Bảng tệp căn trái tên, giữ khung gọn khi tải và có chuyển động mở/đóng.

Khi máy đang có tác vụ, lưới và bảng hiện tên tác vụ cùng bước hoạt động gần nhất
của lượt đang giữ máy. Giao diện đọc lại bản ghi có sẵn mỗi hai giây, không gửi thêm
thao tác hoặc truy vấn thiết bị để tạo nhãn. Bước hoàn tất chỉ được hiện theo kết quả
backend cung cấp; gửi lệnh không đồng nghĩa đã thích hay đã đăng. Khi lượt kết thúc
hoặc đổi chủ giữ máy, nhãn cũ được bỏ ở lần đọc tiếp theo. Chưa có hoạt động phù hợp
thì hiện chủ giữ máy: Tác vụ tự động, Điều khiển trực tiếp hoặc Đồng bộ nhóm.

### Riviu Helper trên Android

**Đã kết nối** chỉ xác nhận máy có kết nối, không có kiểm tra điều khiển đang chạy.
Rê vào trạng thái để xem giải thích: điều khiển được kiểm tra khi mở máy hoặc chạy tác vụ. Ảnh preview
hoặc trạng thái USB không chứng minh helper đã hoạt động. Android chỉ hiện **Sẵn sàng**
khi bản ghi phiên hiện tại xác nhận điều khiển và helper đã được kết nối. Mỗi chức năng
vẫn kiểm tra điều kiện riêng trước khi chạy; giao diện chỉ đọc bản ghi đã có, không tự
mở phiên kiểm tra trên toàn bộ máy lúc khởi động. Mất bản ghi hoặc lỗi đọc giữ trạng thái
chưa kiểm tra, không suy thành sẵn sàng.

Khi hiện **Cần khôi phục helper**, rê vào trạng thái để đọc lý do hoặc mở màn hình
điều khiển để xem thông báo đầy đủ. Bấm **Khôi phục helper** ngay trên đúng ô máy,
hàng thiết bị hoặc thông báo lỗi trong cửa sổ điều khiển. Lối vào này chỉ chuẩn bị
kế hoạch cho máy bị lỗi, không dùng phạm vi nhiều máy đang chọn. Đọc phạm vi và
xác nhận trước khi thực thi; phiên đang thuộc controller khác vẫn được bảo vệ.
Kế hoạch cũ đã thực thi chỉ được đối soát, không dừng helper lần nữa. Bản ghi và
nghĩa vụ chưa rõ vẫn được giữ; khôi phục không tự đăng/gửi lại, không xác nhận clipboard
cũ đã được khôi phục và không tự biến máy thành sẵn sàng. **Bảo trì → Khôi phục helper**
vẫn dùng phạm vi đã chọn; **Sửa Riviu Agent** là thao tác riêng.

Phiên helper đã lỗi có thể được khôi phục khi thao tác cũ đã kết thúc và không còn
clipboard cần đối soát; chỉ còn bản ghi trong bộ nhớ không có nghĩa máy đang bận.
Nếu helper vẫn đang hoàn tất thao tác, ứng dụng báo chờ trên đúng máy. Lần bị chặn
trước khi thực thi giữ nguyên kế hoạch để tiếp tục sau; kết quả chưa rõ sau thực thi
chỉ được đối soát. Khôi phục thành công khóa các kết nối helper cũ, giữ bản ghi và
trả máy về **Đã kết nối**; mở điều khiển hoặc chạy tác vụ để kiểm phiên mới.

Khi phát hiện điện thoại Android đã cho phép gỡ lỗi USB, Riviu Manager tự kiểm tra
**Riviu Helper**, cài nếu thiếu, cập nhật gói chưa phù hợp và xác minh runtime trước
khi báo hoàn tất. Khi một thao tác phát hiện helper hỏng giữa phiên, app xếp lại
việc chuẩn bị cho đúng máy sau khi tác vụ giữ máy kết thúc. App
hiển thị tên **Riviu Helper** cùng logo Riviu trong danh sách ứng dụng. Mở app để xem
chức năng và trạng thái dịch vụ; thao tác này không đổi bàn phím hoặc mở quyền mới.

Việc chuẩn bị chạy nền tối đa hai máy cùng lúc. Máy đang có tác vụ hoặc được mở điều
khiển sẽ chờ đến khi rảnh; stream vẫn giữ nguyên. Máy đã có bản phù hợp được dùng lại,
không cài lại mỗi lần quét. Cài xong phải đọc lại phiên bản, kiểm quyền sở hữu và
xác minh clipboard có khôi phục trước khi báo hoàn tất. Quyền cài USB bị ROM từ
chối hoặc bản ghi thao tác cũ chưa đối soát được sẽ hiện trên đúng máy; app không
lặp cài liên tục. Kế hoạch khôi phục đã thực thi chỉ được đọc lại kết quả. Nút
**Khôi phục helper** vẫn có để xử lý trường hợp cần người vận hành kiểm tra.

**Đọc và gán nick TikTok** đọc song song các máy đã chọn, rồi đối chiếu cả nhóm
trước khi lưu. Nếu tài khoản chuyển giữa các máy, Riviu cần đọc cả những máy đang
lưu tên trùng và hiển thị tên cũ, tên vừa đọc để xác nhận. Máy chưa đọc được giữ
nguyên dữ liệu; chỉ cập nhật tên trong Riviu, không chuyển tài khoản trên điện thoại.

Nếu điện thoại chặn **Cài đặt qua USB**, ứng dụng ghi rõ lỗi ở máy và **Hoạt động / nhật ký của tác vụ liên quan**. Bật quyền cài qua USB trên điện thoại rồi ngắt/kết nối lại để thử lại.
Mỗi kết nối chỉ có một lần thử cài, không lặp cài liên tục khi bị từ chối. Máy chưa
chấp nhận gỡ lỗi USB hoặc mất kết nối chưa bắt đầu cài.

Thanh công cụ đặt **Mở máy**, **Đồng bộ**, **Nhóm** và **Công cụ** cạnh phạm vi máy.
**Đồng bộ** mở bảng chọn máy chính, xem từng máy nhận cùng trạng thái phiên và bật/tắt
đồng bộ. Chọn ít nhất hai máy đang kết nối rồi bấm bật; cửa sổ máy chính tự mở. Phạm
vi được khóa cho lượt đó: đổi lựa chọn, máy chính hoặc roster sẽ tắt đồng bộ, giữ cửa
sổ máy chính ở chế độ một máy và yêu cầu kiểm tra rồi bật lại. Lỗi một máy chuyển nhóm
sang **Cần xử lý**; **Thử lại điều khiển** chỉ mở lại phiên, không phát lại thao tác cũ.
Nếu máy báo đang bận, bấm **Dừng tác vụ cũ và điều khiển** để dừng tác vụ giữ
máy bị lỗi, chờ nhả quyền rồi mở điều khiển. Bài đã đăng và nghĩa vụ kiểm tra link
được giữ lại. Tác vụ không thể tách còn chạy trên máy ngoài lựa chọn sẽ bị từ chối;
hãy dừng tác vụ đó trong Theo dõi trước.
Mục **Độ trễ và độ lệch thao tác** dùng chung cấu hình trong Cài đặt; bấm Áp dụng để
lưu. Máy chính luôn nhận ngay tại đúng tọa độ; độ trễ và độ lệch chỉ áp cho máy nhận.
Mở hoặc đóng bảng không tự bật đồng bộ.

Chuột phải trên ô máy hoặc ngay trên màn hình stream, chọn **Đọc và gán nick TikTok**
để mở Hồ sơ và lưu username vào danh sách thiết bị. Chuột phải trên một máy đã chọn
sẽ đọc đồng thời toàn bộ nhóm đang chọn; máy ngoài nhóm chỉ đọc riêng máy đó.
Mỗi máy có lượt đọc và lưu riêng. Bấm lại không tạo lượt trùng trên máy còn đang đọc;
kết quả nhanh được lưu và cập nhật ngay, không chờ máy chậm.
Username hiện thêm dưới tên máy, không thay số máy, tên máy hay trạng thái.
Máy đọc lỗi, mất kết nối, username trùng hoặc vừa được sửa sẽ báo riêng và giữ nick cũ;
các máy còn lại vẫn tiếp tục. Android đang mở điều khiển dùng lại phiên hiện có;
máy còn bài đăng cần giữ chưa được mở Hồ sơ để đọc nick. iOS chưa hỗ trợ thao tác này.
Nếu đã đọc được username nhưng chưa lưu vì trùng, thông báo ghi riêng kết quả đọc,
username muốn gán, số/tên máy đang giữ bản gán trùng và giá trị cũ của máy đích.
Đây là dữ liệu đã lưu, chưa chứng minh tài khoản đang đăng nhập trên máy kia;
ứng dụng không tự chuyển bản gán. Khi số máy trùng vượt 20, thông báo nêu danh sách
đã được giới hạn. Kiểm tra đúng máy trước khi sửa tài khoản đã gán.
Đọc nick và preflight dùng chung bước khôi phục đầu trang Hồ sơ. Popup đã đo có
nút **Not now** được từ chối trước khi đọc lại account; app không bấm **Save login**
hoặc đổi tài khoản. Không đọc được cùng username qua hai lần quan sát thì vẫn báo
chưa xác định, không tạo lượt đăng từ nội dung popup.
Lệnh **Sửa Riviu Agent** nằm trong **Bảo trì**; hộp xác nhận nêu rõ số máy và việc
khởi động lại stream. Nút quét thiết bị nằm bên phải toolbar. Trạng thái **Toàn hệ
thống** trên header khác phạm vi **Máy thực hiện** của từng tác vụ.

Các cửa sổ chi tiết giữ thao tác bàn phím bên trong; Escape đóng cửa sổ trên cùng
và trả focus về nơi mở. Cửa sổ tiến trình cho phép tiếp tục làm việc với trang chính.
Trong **Chi tiết thiết bị → Khả năng thiết bị**, **Chọn ứng dụng TikTok** liệt kê
các gói TikTok thực sự đang cài. Máy có cả Global và Trill phải chọn một lần trước
khi kiểm tra khả năng hoặc chạy; lựa chọn được lưu theo serial. Không đổi được lựa
chọn khi máy đang bị tác vụ giữ hoặc còn nghĩa vụ đóng ứng dụng/bài đăng cũ.

### Ghi Macro

**Bắt thuộc tính & ghi Flow:** bấm biểu tượng Inspector trên header để chọn một máy
Android đang kết nối, hoặc mở màn hình máy Android và chọn mục cùng tên trong menu
bên phải. Inspector đặt ảnh màn hình, bảng thuộc tính và cây giao diện ở ba vùng
riêng; trên cửa sổ hẹp, bảng thuộc tính và cây xếp cạnh ảnh. Rê chuột để tô phần tử;
chọn trên ảnh hoặc trong cây sẽ đồng bộ vùng tô và bảng thuộc tính. Cây giữ cả node
không bấm được, có tìm kiếm và bung/thu nhánh; bảng phân biệt giá trị `false` với thuộc
tính không có trong cây XML. Có thể sao chép nguyên XML của lần quan sát hiện tại.
Nhãn có số động được lưu bằng phần ổn định khi vẫn xác định duy nhất. Với node mang
nghĩa nhưng không nhận tap, Inspector chỉ dùng cha clickable khi quan hệ gần và có
định danh rõ; node trang trí không tự leo lên khung lớn. **Bấm phần tử** mới gửi thao
tác. Khi đang ghi, sau mỗi bấm hãy chọn phần tử chỉ xuất hiện trên màn kết quả và bấm
**Dùng làm kết quả của bước vừa bấm**; app không tự lấy một node mới sau 350 ms để
gọi là thành công. **Dừng ghi** rồi **Lưu thành Flow**. Mở Flow thiết bị để chỉnh và
chạy. Phiên còn bước chờ xác minh không được lưu.
Macro tọa độ bên dưới vẫn dùng cho thao tác cử chỉ; Flow Inspector tìm lại phần tử.

Agent có thể dùng MCP Riviu khi Local API được bật và token được cung cấp qua
cơ chế an toàn. Semantic v2 cần begin/end, ref duy nhất còn hạn và observation mới;
legacy record/recording giữ contract cũ. Xem [bản đồ controller](../../.claude/skills/riviu-phone-automation/references/control-routes.md).
Không gửi token vào chat, không tự bật API/server chỉ để chứng minh skill có mặt.

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
không được tự gán giờ hiện tại. Các tác vụ cũ hơn xem tại **Lượt chạy**.
Giữ và kéo trên thanh tiêu đề để di chuyển; bấm nhẹ mở/thu gọn, nút dấu trừ thu nhỏ
cửa sổ. Nút thùng rác
xoá bản ghi đã kết thúc khỏi cửa sổ theo dõi, không xoá lịch sử/bằng chứng trong
**Lượt chạy** và không dừng công việc. **Hoàn tác xoá** khôi phục lượt xoá gần nhất.
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
Giữ chuột trên màn hình máy Android để nhấn giữ trong ứng dụng; thả chuột để kết thúc,
kéo khi đang giữ để vuốt. Để nhập chữ, chọn ô trên màn hình máy, gõ vào ô **Nhập chữ**
trong cửa sổ điều khiển rồi bấm nút gửi hoặc Enter. Shift+Enter xuống dòng; ứng dụng
chỉ gửi chữ khi người vận hành xác nhận, không tự bấm tìm kiếm hoặc gửi bình luận.

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
