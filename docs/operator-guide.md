# Hướng dẫn vận hành

Riviu điều khiển thiết bị thật. Chọn đúng máy, ứng dụng, tài khoản và nội dung trước
khi chạy. Việc mở trang, lưu bản nháp hoặc nhận ACK không chứng minh thao tác công
khai đã thành công. Không gửi/đăng lại một lượt chưa rõ kết quả.

## Bắt đầu

1. Xác nhận phiên bản app đang chạy; installer và source checkout có thể khác nhau.
2. Kiểm kết nối/authorization trong **Control Center → Quét lại thiết bị** và **Chẩn đoán**.
3. Chọn đúng phạm vi từng workspace; lựa chọn ở lưới không tự thay target chiến dịch.
4. Đọc preflight; sửa điều kiện bị chặn, không bỏ máy lỗi âm thầm hoặc vượt ownership.
5. Xác nhận thao tác có effect, theo dõi đúng operation/campaign/device ID.
6. Đọc bằng chứng trước mọi retry; dừng không hoàn tác effect đã gửi.

[Chuẩn bị máy mới, nâng cấp, chuyển máy và backup](operator/installation-and-data.md).

## Quy tắc chung

- Chỉ một controller sở hữu thiết bị. Không mở bản dev cạnh bản cài đang giữ cùng fleet.
- Preview cũ không là bằng chứng live; ACK không là hậu điều kiện; `uncertain` không là thất bại chắc chắn.
- DB/credential nằm trên PC, không theo tài khoản Orca hoặc Git. Không sao chép secret vào chat/report.
- Startup thường có thể chạy lịch và worker đã lưu. Chỉ kiểm UI bằng UI smoke cô lập khi cần.
- Lịch chạy cần máy tính/app đang mở, không ngủ; lịch bỏ lỡ không tự cấp quyền đăng bù.
- Tạm dừng Publish là dừng/hủy phần việc còn lại, không phải tự tiếp tục sau đó.
- IdleSweep nhường công việc foreground trong phạm vi đã nhận. Cửa sổ chờ không bảo đảm
  mọi session/gesture/cleanup đã kết thúc; nếu busy vẫn còn, đọc owner thay vì ép nhả.
- Sidebar có thể thu gọn thành icon. Monitor nổi mặc định trên phải, có thể kéo;
  toast/lỗi cạnh thao tác và Hoạt động bổ sung cho lịch sử bền trong **Lượt chạy**.

## Thiết bị

[Control Center, nhóm, đồng bộ, helper, tệp và Inspector](operator/devices.md).

**Sửa Riviu Agent** là lệnh có effect trong **Bảo trì**, không tự thực hiện chỉ vì
health/preview tạm lỗi. Máy có hai TikTok cần chọn ứng dụng trong Chi tiết thiết bị.
Không gán nick nhập tay như proof tài khoản đang đăng nhập.

## Chẩn đoán

Tách các lớp: discovery → transport → session/ownership → observation → input →
verifier. Khi một máy offline, kiểm đúng serial/cáp/hub/authorization trước;
không restart ADB toàn fleet. [Xử lý lỗi và thu bằng chứng](operator/installation-and-data.md#khi-có-lỗi).

## Nuôi TikTok

[Thiết lập, lịch, nguồn video, AI và kết thúc phiên](operator/automation.md#nuôi-tiktok).
Bật hành động không có nghĩa mọi build TikTok hỗ trợ; thiếu capability thì từ chối.
Trần video là giới hạn, không ép bù tương tác để đủ số.

## Tương tác

[Link, tài khoản, hành động, hội thoại và đọc lại](operator/automation.md#tương-tác).
Bình luận sau Send chưa có proof vẫn giữ uncertain; chỉ đọc lại, không gửi lại.

## Đăng bài

[Hướng dẫn Đăng bài đầy đủ](operator/publish.md): nguồn ảnh/video, caption/đối tác,
nhạc, ghép bài–máy, hẹn giờ, liên kết, Sheet, dừng và phục hồi.

**Kiểm tra & đăng** quan sát điều kiện; handoff tác vụ cũ diễn ra sau xác nhận.
**Xác nhận đăng công khai N bài** là xác nhận cuối, không có popup thứ hai.
Theo dõi mở từ khi gửi yêu cầu; status timeout giữ nguyên request ID để đối soát.

Kết nối Sheet mới dùng Google OAuth trực tiếp. Proof quyền ghi có thể dùng lại tối
đa năm phút cho đúng binding local; thay đổi quyền/đích remote trong cửa sổ đó có
thể được phát hiện khi delivery. **Kiểm tra kết nối** làm mới proof ngay. Lỗi Sheet
sau Post không cho phép Post lại; outbox giữ đích/revision/epoch đã chốt.

### Tiếp tục xác minh bài đã gửi sau khi Dừng

[Điều kiện và phạm vi resume](operator/publish.md#tiếp-tục-xác-minh-bài-đã-gửi-sau-khi-dừng).
Resume chỉ mở quyền quan sát đúng bài đủ identity, không đăng lại hoặc giải mọi guard thiết bị.

### Đọc cảnh báo nguồn trước khi xác nhận

[Cảnh báo caption, đối tác và mapping](operator/publish.md#đọc-cảnh-báo-nguồn-trước-khi-xác-nhận).

### Đọc báo cáo nghiệm thu mà không chiếm chuột

[Inspect/observe/preflight/submit và các lớp proof](operator/publish.md#đọc-báo-cáo-nghiệm-thu-mà-không-chiếm-chuột).
Các mode có quyền khác nhau; headless không đồng nghĩa read-only.

## Flow

[Flow thiết bị, My Apps, composition, connector và API](operator/flow-api.md#flow).
Flow Sheet **chưa dùng OAuth direct**: nó cần webhook/token Apps Script legacy.
Không có cấu hình legacy hợp lệ thì node Sheet chưa dùng được dù Publish đã kết nối Google.

## Tác vụ

Mở **Lượt chạy** để lọc theo nguồn/trạng thái/thời gian và vào đúng chi tiết. Mở lịch
sử không dispatch. [Quy tắc đọc kết quả](operator/flow-api.md#tác-vụ).

## Kho nội dung

[Import, chuyển nội dung và uncertain qua restart](operator/flow-api.md#kho-nội-dung).

## Trung tâm ứng dụng

[Cài ứng dụng theo phạm vi và kiểm kết quả](operator/flow-api.md#trung-tâm-ứng-dụng).

## Dữ liệu

View dữ liệu còn trong source nhưng không có lối vào sidebar hiện tại. Dùng
**Lượt chạy**, monitor nguồn và **Hoạt động** để chẩn đoán; không hướng người dùng tìm một menu bị ẩn.

## API

[Local API, auth, Flow run/status/cancel](operator/flow-api.md#api). Listener mặc
định tắt; chỉ loopback với token. MCP semantic cần begin/end và ref còn hạn; không
có quyền Post/Send từ một ref UI. [Bản đồ MCP](../.claude/skills/riviu-phone-automation/references/control-routes.md).

## Cài đặt

[Cấu hình theo vùng, credential và runtime status](operator/flow-api.md#cài-đặt).
Lưu config khác listener đang chạy; đọc yêu cầu restart, không suy từ checkbox.

## Kiểm bình luận bằng TypeSafe

[Phạm vi dữ liệu, credential và giới hạn bằng chứng](operator/flow-api.md#kiểm-bình-luận-bằng-typesafe).

## Khi có lỗi

Giữ operation/request ID, thời gian, phiên bản app và lỗi từng máy. Không xóa DB,
intent, khóa Sheet hay reinstall để thử. [Runbook sự cố](operator/installation-and-data.md#khi-có-lỗi).
Các báo cáo lịch sử không chứng nhận build/thiết bị hiện tại.
