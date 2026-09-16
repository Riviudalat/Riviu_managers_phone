# Bản đồ controller và giới hạn

Các đường dẫn dưới đây tính từ **worktree Riviu hiện tại**, phải kiểm tra lại source trước sử dụng. Skill không đóng băng API và không cấp quyền thao tác.

## 1. Riviu MCP: adapter của backend đang chạy

Nguồn: `scripts/riviu_agent_mcp.mjs`, `apps/desktop/src-tauri/src/inspector_commands.rs`.

| Tool đã xác minh | Hành vi | Giới hạn |
|---|---|---|
| `riviu_devices` | Đọc danh sách device từ `GET /v1/devices` | Vẫn cần backend/API và token; không phải quét bằng controller khác |
| `riviu_observe` | Chụp mới và trả phần tử qua `POST /v1/inspector/observe` | Có session và ghi artifact host |
| `riviu_tap` | Resolve selector duy nhất, tap một lần, capture trước-sau | Cần `udid`, `selector.package` và thuộc tính chọn đúng; có thể đã tap dù hậu kiểm lỗi |
| `riviu_record` | Bật/tắt bản ghi persistent theo thiết bị | Ghi DB; có `udid`, `active`, tên theo schema/handler hiện hành |
| `riviu_recording` | Đọc các bước đã ghi | Không phải thực thi bản ghi |

Adapter hiện không expose `riviu_swipe`, `riviu_type`, `riviu_install` hoặc tool chạy Flow. Không bịa các tên đó.

Endpoint mặc định trong source là loopback, cấu hình qua `RIVIU_API_URL`; auth qua `RIVIU_API_TOKEN`. Token phải đến từ cấu hình hợp lệ đã có; không in giá trị, không hardcode, không gọi endpoint remote. Không khởi server chỉ để kiểm tra skill đã cài.

## 2. API/commands dùng chung ownership

Nguồn: `apps/desktop/src-tauri/src/local_api.rs`, `apps/desktop/src-tauri/src/commands/mod.rs`, `apps/desktop/src/api.ts`.

Các route manual đã thấy trong source: `POST /v1/tap`, `/v1/swipe`, `/v1/key`, `/v1/home`, `/v1/text`, `/v1/lock`. Đọc parser để lấy tên field, enum và hệ tọa độ đúng; không đoán kiểu payload. `locked=false` yêu cầu mở khóa thông thường, không phải bypass mật mã hay chốt xác thực.

`with_manual_session` giữ/mượn session và lease thích hợp. Nếu lease/admission từ chối thì dừng; không gửi trực tiếp xuống adb/WDA để lách. `observe` cũng chịu admission và ownership.

Manual API không thay engine nghiệp vụ. Request có thể đã dispatch khi client timeout, vì vậy retry phải dựa trên evidence và ngữ nghĩa effect, không chỉ mã HTTP.

## 3. Mobile MCP bên ngoài: canary riêng

Đọc `tools/mobile-mcp/README.md` và `scripts/run_mobile_mcp.mjs` trước dùng.

- Dependency đang được pin ở `apps/desktop/package.json`/lockfile; không dùng `npx @latest` thay wrapper của repo.
- Chỉ máy Android thử nghiệm riêng; dừng Riviu trên cùng thiết bị trước khi chuyển controller, có sự đồng ý của người vận hành.
- Inspection là mặc định. Tap/type/install/đổi app-state vẫn là effect ngoài lease và audit của Riviu.
- Không dùng Mobile MCP cho TikTok Like/Save/Comment/Follow/Post. Capture fixture rồi đưa selector vào đường production có verifier.
- Không bật remote/cloud tools. Không bật telemetry, sửa unsafe URL scheme hoặc chế độ listen.
- `mobile-mcp:check` có thể gọi adb version; `mobile-mcp:probe` khởi server, thêm `--devices` mới enumerate. Không gọi chúng là kiểm tra offline thuần túy.

Nguyên tắc tương tự áp dụng cho Appium/Maestro/agent-device: chỉ dùng khi đã xác minh nền tảng, dependency và quyền trên máy thử; không tự thay production controller. Skill có mặt không có nghĩa runtime đã cài.

## 4. Hồ sơ nền tảng

### Android

Đọc `docs/agents/09-fleet-android.md`, `sidecars/android/README.md` và source `crates/android-driver/` liên quan. Giữ adb đang có và precedence trong repo, hạn chế lệnh có thể restart server. Không tự cài/reinstall UiAutomator2/helper, đổi identity/root mode hay reset app. Một máy chỉ có một chủ điều khiển; stream còn chạy không chứng minh input path hoạt động.

### iOS

Trước sửa WDA/iOS đọc **toàn bộ** `docs/agents/02-wda-doc-truoc-khi-sua.md`. Không coi phần tóm tắt này thay tài liệu:

- Stock WDA: không `autoDismissAlerts`; session rồi prime `snapshotMaxDepth: 1`, stream sau.
- RT-MMO dùng profile/token/gesture riêng, không áp prime hay endpoint stock; không tự đổi profile/token để cứu phiên.
- Deadline trên request; không bọc request WDA bằng timeout hủy giữa chừng gây wedge relay.
- Chỉ recovery sau bằng chứng lỗi đúng lớp; `/status` hoặc screenshot sống không chứng minh session-scoped gesture sống.
- Kill chỉ đúng process/fingerprint khi được phép; không kill rộng hoặc cạnh tranh XCTest session.

## 5. Tham khảo GenFarmer

`docs/re/genfarmer/README.md` là khảo sát lịch sử kiến trúc, không trạng thái hiện tại. Chỉ học cách tách stream/control/perception và thiết kế hợp đồng hành vi. Không sao chép code/binary, không mở phạm vi licensing/DRM/private agent đã bị loại trừ, không sao chép bước kill-server thành fallback cho fleet Riviu.
