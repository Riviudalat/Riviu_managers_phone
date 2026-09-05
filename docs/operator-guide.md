# Hướng dẫn vận hành

Riviu Manager quản lý điện thoại thật bằng một control plane. Mỗi thao tác phải có
phạm vi máy rõ ràng và kết quả có thể đọc lại. Chọn hồ sơ chỉ nạp cấu hình; không tự chạy.

## Quy tắc chung

- Kiểm tra số máy, nhóm, ứng dụng và tài khoản đang hiển thị trước khi chạy.
- Mỗi workspace automation giữ phạm vi riêng; đổi trang không biến một máy thành toàn bộ fleet.
- Khi có bản nháp, `Lưu`, `Bỏ thay đổi`, `Ở lại` là ba kết quả khác nhau. Nạp/polling không được làm bẩn bản nháp.
- `queued` còn đang chờ; `running` đã bắt đầu; `uncertain` cần đối chiếu bằng chứng. Không thử lại thao tác công khai chỉ vì thiếu ACK.
- Trạng thái ngắn hạn nằm trong vùng hoạt động; lịch sử bền nằm ở Tác vụ/Dữ liệu và monitor nguồn.
- Mất thiết bị hoặc lỗi quyền phải hiện lỗi ở vùng liên quan. Không dùng ảnh preview còn lưu làm bằng chứng máy đang sẵn sàng.

## Thiết bị

**Đầu vào:** kết nối USB, nhóm, bộ lọc trạng thái, từ khoá và các máy được chọn.
Danh sách/lưới phải dùng cùng tập sau lọc; tile, canvas, mật độ và cử chỉ điều khiển giữ
nguyên hợp đồng hiện có.

**Thao tác:** làm mới roster; chọn nhóm; chọn máy; mở máy hoặc drawer chi tiết; thao tác
hàng loạt qua menu ngữ cảnh. Đọc số máy đích ngay tại lệnh. Dùng Chẩn đoán khi trạng thái
quyền/transport khác trạng thái kết nối.

**Kết quả:** roster, trạng thái work owner, preview và tiến độ từng máy. Máy rời fleet
đóng các vùng thao tác của chính máy đó. **Tiếp theo:** mở máy cần kiểm tra hoặc chuyển
đến automation với phạm vi đã review; không coi đang hiển thị preview là đã pass action.

## Chẩn đoán

**Đầu vào:** thiết bị và nhóm cần kiểm tra. **Thao tác:** đọc transport, helper, quyền và
bằng chứng foreground/stream; làm mới phép đo có chủ đích. **Kết quả:** từng điều kiện
sẵn sàng, lỗi cụ thể và bằng chứng đo được. **Tiếp theo:** sửa đúng điều kiện thất bại,
rồi kiểm tra lại. Repair/cài helper là lệnh riêng, không suy ra từ lỗi preview thoáng qua.

## Nuôi TikTok

**Đầu vào:** phạm vi máy, hồ sơ, thời gian xem, giới hạn phiên, nhịp, hành động và lịch.
Credential AI được lưu riêng, không nằm trong JSON hồ sơ.

**Thao tác:** nạp hoặc sửa hồ sơ; lưu phần credential bằng vùng lưu riêng; kiểm tra cấu
hình và readiness; chạy hoặc lên lịch; chuyển sang Theo dõi để xem tiến độ từng máy.
Trần video là giới hạn trên, không phải mục tiêu bắt buộc.

**Kết quả:** phiên, số card quan sát, effect/bằng chứng và kết thúc từng máy. **Tiếp theo:**
đọc lý do partial/uncertain trước khi tạo phiên mới; không bù số lượng bằng cách lặp
comment. Dừng phiên không chứng minh các effect đã gửi được hoàn tác.
`Lưu hồ sơ` không áp mặc định toàn cục. Chỉ lệnh áp mặc định riêng mới thay cấu hình
dùng chung; lệnh đó không được tự làm sạch bản nháp hồ sơ chưa lưu.

## Tương tác

**Đầu vào:** URL bài, hành động, nội dung/AI, actors và hồ sơ revision. **Thao tác:** parse
đúng chuỗi URL hiện tại; sửa lỗi parse trước khi chạy; review assignment và số bài/số máy;
chạy rồi theo dõi kết quả từng hành động.

**Kết quả:** campaign, assignment, prepared content và trạng thái effect. Đổi URL rồi
parse lỗi không được dùng target cũ. **Tiếp theo:** mở bằng chứng cho uncertain; retry chỉ
phần được hệ thống xác định còn hợp lệ, không gửi lại một comment chỉ vì nhận ACK không rõ.

## Đăng bài

**Đầu vào:** một MP4 H.264/AAC hoặc 1–35 ảnh, caption, âm nhạc, hồ sơ, Sheet và máy được
gán cho từng bài. **Thao tác:** preflight trước dispatch; đối chiếu một bài với một máy
đã gán và toàn bộ hậu quả cleanup; chạy pipeline rồi xem projection trong Theo dõi.

**Kết quả:** bằng chứng Post, URL, nhạc, Sheet và cleanup riêng biệt. Đăng thành công
nhưng thiếu link/Sheet vẫn là Partial. **Tiếp theo:** retry chỉ phạm vi metadata/outbox
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
