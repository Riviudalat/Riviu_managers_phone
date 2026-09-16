---
name: riviu-interaction-flows
description: >
  Dùng khi xây/sửa/nhập kịch bản automation điện thoại cho Riviu Manager: Flow,
  Macro, chuỗi tap/swipe/type, điều kiện/chờ, tìm ảnh/OCR, ghi-phát lại thao tác,
  nhóm đồng bộ nhiều máy hoặc luồng tương tác kiểu GenFarmer. Thiết kế theo trạng
  thái, chọn action catalog/engine hiện có, validate trước run, giới hạn concurrency,
  deadline/cancel và không replay effect chưa xác định. Không tự chạy campaign.
metadata:
  version: "1.0.0"
  origin: local
  project: riviu-manager
---

# Luồng thao tác điện thoại Riviu

Skill tự soạn cho Riviu. “Kiểu GenFarmer” được hiểu là khả năng mô tả công việc nhiều bước trên điện thoại, có điều kiện, lặp giới hạn, quan sát và kết quả từng máy; không là cam kết tương thích toàn bộ hoặc quyền copy code/binary của sản phẩm khác.

## Điểm bắt đầu

- Đọc `AGENTS.md`, runbook, phần Flow/automation của developer và operator guide trong worktree hiện tại.
- Đọc [hợp đồng luồng và cách kiểm tra](references/flow-contracts.md).
- Khi công việc đi tới thao tác live, dùng thêm `riviu-phone-automation`. Khi lỗi/thiếu selector, dùng `riviu-device-diagnostics`; đừng thêm tap/sleep để đoán đường đi.
- Tách rõ **soạn nháp → import/compile → lưu revision → chọn máy → chạy → đọc kết quả**. Mỗi bước sau có tác động lớn hơn; quyền viết nháp không tự cấp quyền lưu DB hay chạy máy thật.

## 1. Chọn lớp thực hiện

| Nhu cầu | Đường phù hợp |
|---|---|
| Một thao tác điều hướng có quan sát | Inspector/manual control qua Riviu, đúng lease |
| Chuỗi thao tác tổng quát có nhánh/chờ/biến | Flow + action catalog hiện hành |
| Luồng nghiệp vụ đã có như Nuôi/Tương tác/Đăng | Engine production tương ứng, không thay bằng macro tap/swipe |
| Điều khiển nhóm từ máy master | Group sync hiện có; khóa master/targets và quyền trước khi active |
| Chuyển bản ghi hoặc JSON bên ngoài | Preview/import subset → xác minh mappings → compiler, không chạy mã nguồn đầu vào |
| Kiểm selector/perception | Fixture/replay offline trước; canary khi được phép |

Không thêm `RawHttp`, `RawWda`, `Shell` hoặc tool trực tiếp để qua mặt catalog/capability/ownership. Không bịa action/field/version từ trí nhớ; đọc DTO và catalog thực tế.

## 2. Viết hợp đồng hành vi trước graph

Đối với mỗi bước, ghi:

- **Trạng thái vào:** app/màn hình, target/account nếu có, điều kiện lease/session, bằng chứng đủ mới.
- **Hành động:** action đã tồn tại, input có kiểu, tọa độ/profile hoặc selector rõ, lớp side effect.
- **Hậu điều kiện:** tín hiệu cụ thể chứng minh đã tới trạng thái mong muốn, không chỉ ACK.
- **Thời hạn:** deadline, số lần quan sát/scroll tối đa và điều kiện bỏ qua.
- **Lỗi:** fail/refused/skipped/chưa xác định; đường báo lỗi và cleanup.
- **Retry:** đọc lại thì thường được, toggle/submit không được lặp khi chưa biết effect; khả năng retry do engine/verifier quyết định.

Ví dụ trung tính: đang ở màn danh sách → tìm mục bằng ID → mở một lần → đọc tiêu đề chi tiết khớp ID → hoàn tất. Nếu hai mục cùng tên thì cần thêm điều kiện phân biệt, không chọn ngẫu nhiên. Nếu đã gửi tap nhưng không đọc được frame thì quan sát lại trước khi quyết định retry.

## 3. Nhận diện và chờ

- Ưu tiên selector duy nhất có package/version/locale đã đo. Fallback ảnh/OCR phải có frame mới, geometry/orientation/profile và ngưỡng/fixture phù hợp.
- Chờ state, visible/enabled/focused hoặc hậu điều kiện; không coi delay cố định là bằng chứng sẵn sàng.
- Scroll phải có điều kiện tiến triển và giới hạn; vị trí các hàng lặp lại không đủ chứng minh đã đến cuối danh sách. Dùng nội dung/identity khi khả dụng.
- Nhập text: kiểm focus, bàn phím, nội dung trước/sau. Không nối chữ hoặc submit lại khi kết quả chưa xác định.
- Không dùng random delay/gesture làm “chống phát hiện”; thời gian chờ nhằm ổn định và phản hồi UI.

## 4. Đa thiết bị và nhóm

- Chụp danh sách target cố định trước run; một serial chỉ thuộc một chủ điều khiển trong cùng thời điểm. Không fan-out mặc định ra toàn fleet.
- Chốt phụ thuộc trong graph: bước cần kết quả/identity của bước trước phải chờ bằng chứng trước, không chạy song song cho nhanh.
- Giới hạn concurrency theo capacity thực tế của control plane/stream/session. Không giả capacity refusal là hàng đợi đang chờ.
- Dữ liệu/biến/evidence phải có scope theo run/device; không trộn màn hình máy A với thao tác máy B. Kiểm yêu cầu cấp toàn run trước khi tách thành task độc lập.
- Group sync cần master/targets rõ và trạng thái active. Đổi máy hoặc reconnect không tự replay thao tác cũ; những bấm gây effect công khai không được broadcast chỉ vì bật sync.
- Lỗi một máy không được bị che bởi thành công của máy khác. Stop/cancel phải đợi trạng thái terminal hoặc báo đang hủy; `cancellationRequested` không phải “đã dừng”.

## 5. Tác vụ công khai hoặc không đảo ngược

Chỉ dùng tài khoản/thiết bị và mục tiêu được phép. Không tự suy yêu cầu tạo tương tác giả, spam, né rate limit/challenge hoặc vượt quyền từ tên GenFarmer.

Với Comment/Post/Follow/Like/Save hay ghi/xóa dữ liệu thật:

- Cần phạm vi người dùng chấp thuận rõ; không dùng luồng canary/Mobile MCP ngoài controller production.
- Xác minh tài khoản, target, caption/nội dung nếu liên quan trước dispatch.
- Dùng engine và identity/evidence/reconciliation đã có. Một state UI thay đổi hoặc màn soạn đóng không chứng minh effect đúng target.
- Sau timeout/restart, phân biệt chưa gửi/đã gửi xác nhận/đã dispatch nhưng chưa xác định. Trạng thái cuối không được ép thành failed rồi chạy lại để làm sạch màn hình.
- Nếu challenge, khóa, permission mới hoặc mismatch identity xuất hiện: dừng nhánh và báo; không sửa định danh máy/tài khoản hoặc hạ verifier.

## 6. Kiểm chứng theo từng tầng

1. Preview/import: chỉ xác nhận chuyển đổi dữ liệu, liệt kê node/field chưa hỗ trợ.
2. Compile: schema/graph/types/catalog hợp lệ; chưa chứng minh capability/runtime.
3. Unit/fixture: hậu điều kiện, timeout, duplicate/ambiguous selector, stale geometry và cancel đã được kiểm trong phạm vi test.
4. Canary được phép: một thiết bị với tác vụ vô hại, bằng chứng trước-sau.
5. Fleet được phép: mở rộng có giới hạn, status/evidence theo từng máy; không bỏ bước 4 chỉ vì compiler xanh.

Dùng `test-driven-development` khi thay logic, `systematic-debugging` khi fail, `verification-before-completion` trước bàn giao. Lượt chỉ cài skill không phải dịp chạy 4–5.

## Đầu ra cần có

Nêu rõ lớp thực hiện, graph/contract đã thay, mappings không hỗ trợ, phần đã validate/test, revision nếu thực sự lưu, thiết bị đã/chưa chạy, hậu điều kiện đã/chưa chứng minh và giới hạn còn lại. Không tuyên bố “tương thích GenFarmer 100%” hay “chạy được iOS” từ một JSON preview Android.
