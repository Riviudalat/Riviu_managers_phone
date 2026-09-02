# Ma trận parity clean-room cho mẫu tham chiếu Android

## Nguồn và ranh giới

Đợt khảo sát tĩnh ngày 02/09/2026 chỉ đủ để lập bản đồ hành vi: bundle giao diện đã
minify, Java decompile và pseudo-C; không có source map hay mã nguồn Rust/Vue gốc có thể
build. Riviu vì thế chỉ tái hiện hợp đồng sản phẩm từ mô tả hành vi và test fixture độc
lập. Không file thực thi, APK, asset, giao thức, bytecode, mã dịch ngược hay nhãn vận hành
của mẫu tham chiếu được đưa vào runtime hoặc installer.

- SHA-256 báo cáo đã đọc: `71133f9972e16c292bbfbcf1086e9632645c0ba016334fac23f0a0d807b919e4`.
- SHA-256 inventory command đã đọc: `289fdb2fe1934f9bf202d0ec7551ddffcf2d825f4e6022dcd4d141ad89a5dd3a`.
- Inventory ghi nhận đúng 158 tên command và 0 source map.
- Ma trận sinh lại ở [`xiaowei-parity-matrix.csv`](xiaowei-parity-matrix.csv); mỗi command
  có đúng một trạng thái trong năm trạng thái đã duyệt.

## Quy tắc quyết định

- `existing`: Riviu đã có năng lực typed tương đương qua control plane hiện hữu.
- `implement`: tái hiện clean-room trong đợt này: thư viện gói Android/cài split.
- `commercial-excluded`: tài khoản, SMS, license/VIP/activation, thanh toán, cloud phone,
  telemetry, branding hoặc updater của nhà cung cấp.
- `security-excluded`: Auto.js/script tùy ý, bridge plaintext, xwdb/Magisk, nhập `adbkey`,
  broadcast exported, AOA/HID hoặc Accessibility transport của mẫu tham chiếu.
- `not-applicable`: chi tiết shell/host không tạo thành năng lực sản phẩm cần parity.

`AutoSwipe` là mở rộng mới của Flow Riviu theo yêu cầu vận hành, không phải bản sao command
hay giao thức của mẫu. Gate `scripts/check_xiaowei_provenance.py` quét runtime, frontend đã
build và installer; thư mục provenance này là nơi duy nhất được phép lưu tên định danh để
review quyết định.

## Nguồn thư viện độc lập

- `apk-info = 1.0.12`: parser Rust độc lập, giấy phép Apache-2.0.
- Bundletool 1.18.3: chỉ chọn và giải nén APK phù hợp từ `.apks`; cài thật luôn do
  `DeviceControlPlane` của Riviu thực hiện.
- Eclipse Temurin JRE 21.0.12.1+1: runtime Windows đóng gói riêng, kiểm SHA-256 trước giải nén.

Không dữ liệu tài khoản, khóa ADB, secret hoặc định danh người dùng từ mẫu khảo sát được dùng
làm fixture hay tài nguyên build.
