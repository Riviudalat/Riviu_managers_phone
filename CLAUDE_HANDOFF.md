# Riviu Manager — bàn giao từ cuộc trò chuyện Codex

Đây là bản tóm tắt ngữ cảnh và các đường dẫn bằng chứng, không phải bản xuất đầy đủ lịch sử chat. Kiểm tra source và hồ sơ hiện tại trước khi thực hiện hoặc khẳng định kết quả; các số liệu dưới đây thuộc những lượt đã ghi nhận.

## Dự án và cách làm

- Workspace: `C:/Users/cattfan/Desktop/Riviu_managers_phone`.
- Windows, Rust/Tauri/React/SQLite, một control plane chung cho điện thoại.
- Đọc `README.md`, `AGENTS.md`, `docs/developer-guide.md` và các hướng dẫn agent liên quan trước khi sửa.
- Người dùng yêu cầu giữ UI cũ cam/trắng. Không áp dụng lại đợt đổi UI đã bị phản đối.
- Làm việc qua CDP/IPC production khi phù hợp; không chiếm chuột hoặc bàn phím của người dùng.
- Không suy đoán tình trạng máy hay gọi ACK là thành công. Phân biệt source, fixture, runtime và kết quả máy thật.
- Không tự replay Post/Send khi thao tác trước có thể đã xảy ra. Giữ intent, ID lượt đăng, account và trạng thái chưa xác định.
- Khi viết/sửa/review tests, dùng skill `test-audit`; bảo vệ hợp đồng hoặc regression thật, không tạo test chỉ lặp implementation.
- AI phân loại/trích xuất của dự án dùng skill TypeSafe theo AGENTS.md; không tự thay bộ nhận diện bằng một controller cạnh tranh.
- Thư mục bản giao dùng `target/<version>/`, ví dụ `target/0.2.65/`; dùng cache Cargo chung, không tạo cache biên dịch riêng cho từng version.

## Source và các thay đổi chưa commit

Commit `2f48f24` đã được push lên `main` trong lượt trước. Sau đó đã có thêm thay đổi local; đọc `git status` và `git diff`, không reset/stash để làm sạch.

Các nhóm thay đổi local đã ghi nhận:

1. Khôi phục 11 file presentation về bố cục của `48de176`: thông tin máy phủ trên màn hình thay vì khối trắng 88px; header 56px, sidebar 196px; toolbar gọn, monitor trở lại góc trên phải.
2. Version app/checker/manifest/lock metadata lên `0.2.65`.
3. Sửa đóng gói frontend, thêm `packaged_frontend.rs`, lệnh `--verify-frontend`, kiểm frontend trước startup và gate trong `scripts/bundle_windows_installers.py`.
4. Cập nhật hướng dẫn build và quy ước thư mục version.

Không xóa các file untracked hiện có chỉ để làm git status đẹp; chúng bao gồm `.codex/`, `apps/desktop/pnpm-lock.yaml`, `apps/desktop/target/` ở lần kiểm gần nhất.

## Lỗi bản cài và bản mới

Người dùng báo bản cài `0.2.64` hiện Edge `ERR_FILE_NOT_FOUND`.

Nguyên nhân đã chứng minh bằng parser Tauri thật: overlay `frontendDist` dạng `C:/Users/...` bị deserialize thành URL scheme `c:`, không phải Directory; frontend vì vậy không được nhúng. Đã chuyển sang đường dẫn tương đối với `src-tauri`.

Bản `0.2.65` đã build/NSIS thành công. EXE compiled và EXE giải nén từ installer đều chạy `--verify-frontend` exit 0, báo `embeddedDirectory`, đủ `index.html`, 21 asset và cả 4 tham chiếu HTML. Hash frontend/binary/checker/runtime đã đối chiếu.

Bản giao hiện được sắp xếp ở:

`target/0.2.65/Riviu Manager Full_0.2.65_x64-setup.exe`

SHA-256 đã ghi nhận:

`5dd67cbe9c41364698697a8996c937a6a37c619e735e95a73fb0768f46f4b0be`

**Giới hạn còn lại:** chưa chứng minh cửa sổ bản đóng gói mở hoàn chỉnh. Lượt startup cô lập bị `failed to receive message from webview` trong môi trường quyền hiện tại. Full checker còn thất bại vì `yt-dlp` không tạo được thư mục PyInstaller tạm. Không gọi hai gate này là đã đạt. Clippy phạm vi lib/bin desktop đã đạt. Không suy lỗi startup môi trường này là cùng nguyên nhân với lỗi frontend đã chứng minh.

Bằng chứng chi tiết: `target/installed-ui-fix-20260930/` và `target/0.2.65/DELIVERY.json`.

## Đăng bài và lấy link — bằng chứng đã ghi nhận

- Đợt 30 máy: 29 đạt, máy 13 bỏ qua vì chưa đăng nhập.
- Đợt kiểm lại 9 máy trên Trill 38.3.2 và Global 45.7.3: 9/9 đăng ảnh, đúng canonical URL/account, receipt, ô Sheet readback khớp và cleanup; gồm hai lượt hẹn giờ máy 5 và máy 30.
- Máy 25 từng bị chặn trước Post do writer khác giữ khóa Sheet; đã chờ khóa nhả tự nhiên và dùng request ID mới. Không chiếm khóa theo tuổi.
- Đã thử force-stop/mở lại máy 25 đang lấy link: PID cũ 30486 hết, PID mới 32435, intent hash không đổi. Verifier kế tiếp lấy link và ô D439 khớp. TikTok cũng có thêm thời gian xử lý nên không khẳng định restart là nguyên nhân duy nhất.
- Nhịp kiểm lại link là 300 giây tính từ cuối lần kiểm trước. Không phải mọi lần đều restart: cần đủ điều kiện; bài khác trên cùng máy có thể buộc giữ app; người dùng Dừng phải được tôn trọng. Ba lượt liên tiếp không có tiến triển chuyển sang cần kiểm tra, không tự replay Post.
- Nhánh restart riêng cho receipt uncertain mới có owner checks nhưng chưa được kích hoạt trong đợt live này, vì các receipt trả submitted.

Hồ sơ: `target/fleet30-20260930/aggregate.json`, `target/publish-finish-20260930/aggregate.json` và `FINAL_REPORT.md`.

## Nhận diện và timeout — người dùng đang hỏi toàn app

Cây trợ năng là nguồn dữ liệu. XPath chỉ là phương pháp truy vấn cùng cây, không phải fallback độc lập nếu cây treo. Android hiện ưu tiên semantic/content-desc/text/resource-id/class và quan hệ node.

- Chọn nhạc Global 45.7.3: probe XML có hạn; lỗi/timeout có thể chuyển OCR/ảnh đã đo. Cần đúng package/session và còn deadline. Một số trường hợp ảnh không đọc được có đường XML-only.
- Editor: có OCR render wait cho màn đã đo và phục hồi đọc cây mới có giới hạn ở một số nhánh.
- Nuôi: hierarchy trước; pixel khi backend không hỗ trợ và layout có calibration. Hierarchy bị từ chối trên Android không tự rơi sang layout pixel khác.
- Tương tác/comment/reply: vẫn cần bằng chứng đúng bài/tài khoản/comment và trạng thái. OCR/pixel không thay được mọi bằng chứng danh tính.
- Bộ semantic resolver chung: cây đọc được nhưng không có candidate có thể dùng reasoner; hierarchy timeout chưa tự chuyển OCR ở mọi nơi.
- Driver chỉ retry allowlisted reads với lỗi stale-tree cụ thể; timeout HTTP đơn thuần không replay tùy ý. Readiness phát hiện agent sống nhưng mù có recovery instrumentation và cooldown.
- Timeout sau tap/Send/Post phải đối soát kết quả, không bấm lại ngay.

**Chưa có fallback thống nhất toàn app “XML timeout → OCR → tiếp tục”.** Người dùng hiện đang tìm hiểu; chưa chốt một kế hoạch triển khai toàn bộ fallback mới.

## Cleanup đã thực hiện

Đã dọn compiler/dependency cache debug và các cache thử nghiệm cũ. Phần đọc được của `target` giảm 216.36 GiB xuống 87.26 GiB; dung lượng trống đĩa tăng 119.31 GiB (~128 GB).

Giữ release cache, app EXE/sidecars, dữ liệu vận hành, bằng chứng phone, source baseline và rollback. Build debug lần sau sẽ cần tái tạo cache.

Hai thư mục cache chưa xóa hết do quyền/đường dẫn dài. Lệnh xóa bổ sung extended-length path bị kiểm duyệt tự động chặn; không bypass. Hồ sơ: `target/cleanup-20260930/RESULT.json` và `REPORT.txt`.

## Bốn artifact xuyên suốt

- `target/conversation-accounts/MODIFIED_FILE.zip`
- `target/conversation-accounts/DIFF_FILE.patch`
- `target/conversation-accounts/VERIFICATION.txt`
- `target/conversation-accounts/ROLLBACK.sh`

ZIP/Patch bị quyền hiện tại từ chối đọc; ledger và rollback đã đọc lại, ledger có bổ sung các lượt sau. Không gọi source archive cũ là đã được refresh với toàn bộ sửa 0.2.65.

Người dùng yêu cầu bảo toàn artifact và rollback. Bằng chứng UI cũ giữ lại: BASELINE khối trắng, MODIFIED overlay; ROLLBACK archive pristine có hash khớp. Đây không phải phép rollback mới cho sửa đóng gói 0.2.65.

## Khi bắt đầu tiếp nhận

Đọc file này và source hiện tại, xác nhận các vấn đề còn lại; chờ yêu cầu mới của người dùng trước khi tự chạy thêm campaign, cài đặt app hoặc làm một cuộc refactor fallback toàn app. Không coi các báo cáo lịch sử là kiểm chứng runtime hiện tại.
