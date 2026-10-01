# Flow, thư viện và API

[Hướng dẫn vận hành](../operator-guide.md) · Chi tiết chức năng hiện hành; bảng khả năng và proof trên máy quyết định quyền chạy.

## Flow

**My Apps → Mở trình thiết kế** mở graph riêng của ứng dụng. Thư viện bước ở bên trái,
canvas ở giữa; chọn bước để chỉnh thông số bên phải. Kéo bước, nối cổng, xóa bước,
hoàn tác/làm lại và lưu tạo revision. Tìm node bằng tên/ID rồi Enter để đưa vào vùng nhìn.
Ở danh sách My Apps, tìm kiếm khớp tên quy trình hoặc loại ứng dụng; có thể sắp
xếp quy trình đã lưu theo mới nhất, cũ nhất hoặc tên. Ba ứng dụng có sẵn vẫn giữ
nguyên vị trí và thao tác mở riêng.
**Cấu hình ứng dụng** nhận hồ sơ đã lưu hoặc nguồn/link/nội dung đầu vào. **Chạy** yêu
cầu lưu trước và chọn phạm vi thiết bị; theo dõi và hủy trong bảng chạy.

Nuôi TikTok dùng thứ tự Thích/Lưu/Bình luận đã nối, tỷ lệ từng bước, giới hạn xem và
thời lượng. Các bước chuẩn bị/xác minh trong Tương tác/Đăng bài giữ thứ tự phụ thuộc
của engine; nối sai bị báo khi kiểm tra/lưu. Chờ và Ghi nhật ký đặt trước/sau pipeline;
lịch theo hồ sơ chưa nhận các bước Chờ/Ghi nhật ký bao quanh. Graph này chưa thay thế
thư viện thao tác thấp của **Flow thiết bị** và không chứng nhận tương đương mọi node GenFarmer.

**Tác vụ đã lưu** ghim ứng dụng/revision cùng máy thực hiện; tìm theo tên tác vụ
hoặc tên ứng dụng mà không gọi lại backend. **Lịch chạy** quản lý
lịch của các hồ sơ, hiển thị lần chạy kế tiếp và lỗi gần nhất. Bảng lịch và cấu hình
đặt cạnh nhau trên màn rộng; màn hẹp hiện tên, trạng thái, lần chạy, thao tác ở bảng
trên và cấu hình ở dưới. **Quản lý tài khoản**
giữ metadata, nhóm và máy; đọc tài khoản là thao tác riêng. Nhập/xuất JSON không gửi
lệnh vào điện thoại. **Mạng & Router** giữ hồ sơ kết nối, thử TCP và áp dụng/xóa proxy
HTTP không xác thực trên Android; kết quả phải đọc lại khớp trên từng máy.
Biểu mẫu sửa tài khoản/tác vụ đặt cạnh danh sách trên màn rộng; màn hẹp mở ngăn bên
phải có thể đóng bằng Escape, giữ thao tác Lưu trong vùng nhìn thấy. Đóng bản nháp
tài khoản chưa lưu vẫn yêu cầu xác nhận bỏ thay đổi.
Không có chợ ứng dụng trong giao diện hiện tại; My Apps quản lý các quy trình trong Riviu.

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

**Giới hạn hiện tại:** Flow Sheet vẫn dùng webhook Apps Script và token legacy,
không dùng phiên OAuth direct của Đăng bài. Kết nối OAuth thành công không đủ để
chạy node Sheet; UI mới không có form tạo webhook legacy. Chỉ dùng khi cấu hình
legacy hợp lệ đã có qua đường được hỗ trợ; nếu thiếu, node không khả dụng.
Không lấy token Google thay token Apps Script, không sửa DB để lách điều kiện.
Mỗi bước yêu cầu link bảng, tên tab và vùng A1 tối đa 1.000 ô. Cập nhật bản triển khai Apps Script
bằng [`publish-sheet.gs`](../apps-script/publish-sheet.gs) đi kèm để bật đọc/ghi vùng;
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

**Ghép hoặc chỉnh Flow con** có hai chế độ. **Bản sao cố định** lưu trọn nội dung
và đóng băng cả các liên kết Flow con bên trong; nguồn đổi sau đó không đổi bản sao.
**Bản công bố dùng chung** liên kết một Flow đã lưu: nạp đúng revision, công bố bằng
CAS rồi liên kết. Mọi lượt chạy mới tự resolve bản công bố hiện tại thành một revision
cha bất biến trước khi chạy; lượt đang chạy, retry và startup recovery giữ manifest
cũ. Công bố bị từ chối nếu tạo vòng phụ thuộc, thiếu nguồn hoặc làm consumer hiện tại
không compile. Chọn bước đã ghép rồi mở lại để sửa chế độ/body hoặc ánh xạ biến.
Muốn lưu trữ Flow nguồn, dùng **Bỏ công bố** trong hộp ghép; lệnh bị chặn cho tới
khi mọi Flow hiện hành/publication đã gỡ liên kết tới nguồn đó.
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

> View này còn trong source nhưng bị ẩn khỏi sidebar. Không có đường vận hành menu
> Dữ liệu hiện tại; dùng Lượt chạy, monitor và Hoạt động. Phần dưới mô tả projection
> còn tồn tại, không yêu cầu mở một menu không có.

Trang tách **Năng lực hiện tại** khỏi **Tác vụ trong 24 giờ qua**. Các số tác vụ dùng
đúng cửa sổ 24 giờ được ghi trên màn hình. **Nhật ký thao tác** hiển thị tối đa 200
bản ghi gần nhất; ô tìm kiếm chỉ lọc trong tập đã tải, xuất danh sách cũng chỉ xuất
kết quả đang lọc. Tra cứu lần chạy theo nguồn, trạng thái và khoảng ngày ở **Lượt chạy**;
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
đọc [ràng buộc WDA](../agents/02-wda-doc-truoc-khi-sua.md) và không chạy harness đồng thời
với desktop đang sở hữu thiết bị. Hướng dẫn này mô tả hợp đồng; nghiệm thu có ngày ở
[kho lịch sử](../archive/README.md), không tự cấp chứng nhận cho mọi thiết bị/bản cài.


Khi dọn Sheet bị gián đoạn, kết nối hiển thị chưa sẵn sàng nhận bài. Tiếp tục cùng
đợt dọn để giữ bản sao và hoàn tất; các dòng mới được nhận sau khi đọc lại xác nhận
đã dọn. Gửi lại yêu cầu tạo bài bị mất phản hồi sẽ mở đúng chiến dịch đã tạo.
## Kiểm bình luận bằng TypeSafe

Trong **Nuôi TikTok → AI → TypeSafe**, lưu khóa TypeSafe rồi bấm **Kiểm tra bằng mẫu
chữ** để kiểm kết nối. Phép kiểm này gọi dịch vụ bằng mẫu tiếng Việt, không bấm hoặc
gửi nội dung trên điện thoại. Khóa nằm trong kho mật khẩu hệ điều hành; màn hình chỉ
báo đã có khóa, không đọc lại giá trị. **Xóa khóa** không làm mất cấu hình Nuôi.

Khi bật TypeSafe, các lượt tạo bình luận AI mới của Nuôi và Tương tác đối chiếu câu
ứng viên với caption/lời thoại đã thu được. Phiên Nuôi lấy cấu hình TypeSafe lúc bắt
đầu; muốn đổi cho phiên đang chạy, Dừng rồi chạy phiên mới. Bằng chứng chữ thiếu không
thể thay kiểm ảnh. Bản chỉ có chữ phải đạt cả kiểm TypeSafe lẫn kiểm sinh nội dung;
lỗi dịch vụ dừng lượt bình luận, không tự bỏ qua lớp kiểm để Gửi.

Dịch vụ nhận phần chữ cần đối chiếu, không nhận khóa Google, ảnh màn hình hoặc thông
tin USB. Số token và độ trễ TypeSafe được ghi riêng trong log backend; tổng USD của
gateway sinh nội dung chưa bao gồm TypeSafe. TypeSafe không chứng minh đã Gửi, đúng
account hoặc đã ghi Sheet; các bằng chứng đó vẫn do controller và verifier hiện có
kiểm tra.

## Lỗi tài khoản và Inspector semantic

Riviu tự đọc tài khoản đang mở; không cần nhập username thủ công khi hồ sơ có đủ
bằng chứng. Thông báo phân biệt yêu cầu đăng nhập, lời nhắc che hồ sơ, tài khoản khác
lượt đã gán và lỗi đọc/kết nối. Xử lý đúng màn trên máy rồi thử lại bài chưa Đăng;
bài đã gửi chỉ kiểm link và Sheet, không gửi lại.

Inspector semantic/MCP cần mở phiên riêng trước khi observe. Thiết bị đang bận bị
từ chối, không tự giành từ tác vụ. Quan sát trả ref ngắn hạn; sau thay đổi màn hãy
observe lại. Kết quả dispatched chỉ là gửi thao tác, không chứng minh nghiệp vụ đã
thành công. Đóng phiên khi xong; phiên nhàn rỗi hết hạn sau 60 giây.
