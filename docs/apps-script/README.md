# Ghi kết quả đăng bài lên Google Sheet

Mẫu báo cáo nội bộ dùng đúng thứ tự:

`STT | Người air | Ngày | Link | Máy | Tài khoản TikTok | Trạng thái | Lỗi hoặc ghi chú | Đối tác | Đối tác 2 | ...`

Bốn cột nội bộ nằm ở E:H; mọi đối tác bắt đầu từ I, mỗi tên một cột. Script mở rộng
khối đối tác khi cần, tối đa 100 tên; cột riêng sau khối được dịch sang phải cùng
giá trị, công thức và ghi chú. Script không chèn cột kỹ thuật. Mẫu compact
cũ `STT | Người air | Ngày | Link | Đối tác | Đối tác 2 | ...` vẫn được hỗ trợ.

1. Trong spreadsheet đích, mở **Tiện ích mở rộng → Apps Script** và dán
   [publish-sheet.gs](publish-sheet.gs).
2. Điền `SPREADSHEET_ID`, `SHEET_GID` và token riêng trong CONFIG. `SHEET_GID: 0`
   chọn đúng tab gid0; `null` mới dùng `SHEET_NAME` hoặc tab đầu. Múi giờ trong
   cài đặt spreadsheet quyết định cột Ngày, ví dụ `Asia/Ho_Chi_Minh`.
3. Trong Apps Script, **Dịch vụ → + → Google Sheets API → Thêm**. Hai mẫu này cần
   dịch vụ này để mở rộng hàng/cột, ghi header, giá trị và khoá chống trùng trong
   cùng một lệnh nguyên tử. Dữ liệu được kiểm tra đầy đủ trước lệnh ghi.
4. Triển khai ứng dụng web với quyền thực thi của người có quyền sửa spreadsheet.
   Dùng URL kết thúc `/exec` và cùng token trong cấu hình Sheet của Riviu Manager.
   Khi cập nhật script, sửa phiên bản của deployment hiện có để giữ URL.
5. Bật ghi Sheet cho chiến dịch cần báo cáo. Bản cài trên máy khác cần nhập lại URL
   và token trong app; bộ cài không mang tài khoản Google, dữ liệu hay credential
   từ máy phát triển. App gửi hàng đợi đã lưu, không cần giữ trình duyệt đăng nhập
   hoặc dùng chuột.

`LAYOUT_MODE: 'auto'` nhận mẫu nội bộ hoặc compact bằng header. Chọn `'internal'`
hoặc `'compact'` để bắt buộc đúng mẫu; `'legacy'` dùng B/D/K..AG/AH. Header nội bộ
sai hoặc cấu hình chế độ không khớp sẽ bị từ chối trước khi ghi. Trong mẫu nội bộ,
`POSTER_COLUMN=2` và `LINK_COLUMN=4`; khối đối tác được xác định từ header I trở đi.
Payload báo cáo nội bộ không được ghi vào mẫu compact hay legacy.

Kết nối mới cần `deliveryVersion: 2`. Nút kiểm tra gửi `rowKind: 'check'` với
`checkVersion: 1` và yêu cầu phản hồi quảng bá `deliveryVersion: 2`, đúng mã bảng
và gid. Đường kiểm tra chỉ đọc. Mẫu legacy quảng bá phiên bản 1; app mới cần mẫu
compact hoặc nội bộ. Script vẫn nhận payload phiên bản cũ để tương thích các máy
chưa cập nhật. Chuẩn bị bảng trống là thao tác riêng, ghi header nguyên tử và giữ
nguyên bảng đã có dữ liệu.

Mỗi lần ghi mới lấy STT lớn nhất cộng 1. Ngày lấy `postedAt` đã lưu khi gửi bài,
không lấy ngày kết nối lại. Báo cáo trước khi gửi cho phép `postedAt: null` và Link
trống. Sau khi có thời điểm gửi, cập nhật giữ nguyên thời điểm ấy. Cùng assignment
cập nhật đúng một dòng và STT khi chuyển qua `Chưa đăng`, `Đã gửi`, `Đã xác minh`
hoặc `Cần kiểm tra`. Chỉ `Đã xác minh` có canonical link; bài chờ không nhận link
đoán. Mẫu compact vẫn yêu cầu ngày gửi; payload cũ thiếu `postedAt` chỉ dùng legacy.

Khoá assignmentId nằm trong **ghi chú ô Link**, kể cả khi Link còn trống. Báo cáo
nội bộ dùng ghi chú JSON có phiên bản, revision và fingerprint của hàng đã ghi.
Google Sheets API cập nhật giá trị và ghi chú cùng batch. Request cũ được xác nhận
bằng revision hiện tại; cùng revision nhưng khác nội dung bị từ chối. Nếu người
dùng sửa giá trị hoặc công thức trong hàng, script báo xung đột trước khi ghi đè.
Link đã xác minh không được thay bằng link khác hoặc xóa bởi báo cáo cũ.

Mỗi chiến dịch chốt `spreadsheetId`, `sheetGid` và chế độ báo cáo. Mọi request mới
gửi thêm `deliveryVersion: 2`, `deliveryRevision` và đích đã chốt; webhook trỏ nhầm
bảng hoặc tab bị từ chối trước khi mở rộng cột hay ghi dữ liệu. ACK chỉ được tạo
sau khi đọc lại dòng và ghi chú đã commit, gồm `ok`, `deliveryVersion`,
`spreadsheetId`, `sheetGid`, `assignmentId`, `deliveryRevision`, `postUrl` và `row`.
ACK canonical phải khớp chính xác revision giao nhận và link bài. Một phản hồi chỉ
có `ok: true` không đủ để đánh dấu đã giao.

Khi nâng mẫu compact, chèn đúng bốn cột E:H trước toàn bộ đối tác. Ghi chú
`riviu-publish:v1:assignmentId` ở D được nhận sang JSON nếu người đăng, ngày, link,
đối tác vẫn khớp và bốn ô mới trống. Dòng ngoài app chỉ được nhận theo một link
khớp duy nhất cùng danh tính; Link trống không đủ để nhận một dòng chờ không có
khóa. Request outbox cũ thiếu metadata giữ bốn cột nội bộ và revision hiện tại,
có thể điền Link của assignment đã biết nhưng không tự nâng trạng thái báo cáo.

Wire nội bộ: `rowKind: 'internalReport'`, `reportVersion: 1`, `rowRevision` là số
nguyên an toàn không âm tăng theo backend, cùng `assignmentId`, `postUrl`, `poster`,
`postedAt`, `machine`, `tiktokAccount`, `status`, `stateNotes`, `partners` và token.
Outbox canonical có metadata dùng `rowKind: 'canonical'` với trạng thái đã xác minh.
ACK báo cáo phải có `ok: true`, `reportVersion: 1`, đúng `assignmentId` và
`rowRevision` ít nhất bằng request; ACK của script cũ không chứng minh đã cập nhật
báo cáo nội bộ.

Với báo cáo phiên bản 2, `deliveryRevision` bằng `rowRevision`. Báo cáo cũ được
xác nhận bằng revision mới hơn đang lưu và `postUrl` thực tế trong ô D, kể cả khi
request cũ còn trống Link. Revision canonical nằm riêng trong ghi chú, nên cập
nhật tiến độ không đổi danh tính giao nhận. Mất phản hồi sau commit có thể gửi lại
cùng request; khoá trong ô D giữ đúng một hàng. Xung đột dữ liệu, token, đích,
header hoặc ACK cần sửa cấu hình; lỗi mạng, giới hạn tốc độ và lỗi máy chủ được
thử lại theo hàng đợi.

Script Lock phối hợp request của cùng project; người sửa Sheet hoặc project khác
có thể sửa đồng thời. Dùng một deployment ghi cho mỗi Sheet và tránh sửa dòng đang
được gửi. Các cột riêng sau đối tác cuối được giữ nguyên.

## Nâng deployment đang dùng

1. Lưu bản source và phiên bản deployment hiện tại để phục hồi. Thay code bằng
   `publish-sheet.gs`, giữ nguyên toàn bộ `CONFIG` của project đang dùng. Không
   chuyển token hoặc phiên đăng nhập vào source được commit hay bộ cài công khai.
2. Xác nhận dịch vụ nâng cao **Google Sheets API** đang bật. Giữ header compact
   hoặc nội bộ hiện tại; nếu cần nâng compact sang nội bộ, chèn đúng E:H trước
   khối đối tác và giữ ghi chú D. Với legacy, chuẩn bị một tab compact/nội bộ
   riêng rồi kết nối cho các chiến dịch mới; không chuyển các hàng chờ cũ sang tab đó.
3. Trong **Triển khai → Quản lý bản triển khai**, sửa deployment đang dùng, chọn
   **Phiên bản mới**, giữ nguyên quyền thực thi/quyền truy cập rồi triển khai.
   Cập nhật deployment hiện có giữ URL `/exec`; không tạo URL mới nếu không cần.
4. Trong Riviu, dùng **Kết nối Sheet** để xác nhận đúng bảng/tab và phiên bản 2.
   Với tab đã có dữ liệu, kiểm tra không thêm hàng hay sửa header. Nếu chỉ cần
   đọc xác nhận kỹ thuật, gửi `rowKind: 'check'`, `checkVersion: 1` cùng token,
   `spreadsheetId`, `sheetGid` và kiểm ACK có `deliveryVersion: 2`.
5. Sau khi nghiệm thu kết nối, chạy một lượt mới được phép. Đối chiếu canonical
   Link, tài khoản, thời điểm gửi và một dòng/STT trên Sheet; gửi lại cùng
   assignment phải trả `duplicate` mà không thêm dòng. Lịch sử cũ giữ nguyên.

Nếu cần phục hồi, trỏ deployment hiện có về phiên bản trước để giữ URL và phục
hồi source đã lưu. Client v2 sẽ báo chưa hỗ trợ giao nhận thay vì ghi nhận thành
công bằng ACK cũ; hàng chưa được xác nhận vẫn nằm trong hàng đợi. Sau khi triển
khai lại phiên bản 2, kiểm tra kết nối và dùng **Ghi lại Sheet** cho hàng cần xử lý.

Kiểm tra local bằng `node --test scripts/test_publish_sheet.mjs`. Bộ test mô phỏng
API và các điểm lỗi trước/sau commit; cần thêm một lượt qua `/exec` thật sau khi
triển khai để nghiệm thu quyền Google và kết nối máy mới.

Nguồn hợp đồng nguyên tử: [Google Sheets batchUpdate](https://developers.google.com/workspace/sheets/api/reference/rest/v4/spreadsheets/batchUpdate).
