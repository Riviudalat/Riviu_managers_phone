# Bằng chứng và cổng kiểm thiết bị

Đường dẫn tính từ worktree hiện tại. Đọc source trước chạy lệnh; các ví dụ không cấp quyền thao tác.

## Bằng chứng đủ dùng

Một phép đo nên liên kết được:

- exact device/UDID và controller/session/owner;
- package/bundle, version/locale, orientation và kích thước frame;
- thời gian/snapshot ID trước và sau;
- mục tiêu/selector hoặc geometry/profile; số kết quả match;
- intent trước dispatch, action result, error/deadline/cancel;
- hậu điều kiện được quan sát và trạng thái cuối: xác nhận, từ chối, bỏ qua hoặc chưa xác định.

Log “OK” không thay được ảnh/identity/readback mà verifier nghiệp vụ cần. Ảnh khác pixel cũng không đủ: spinner, clock, counter/video có thể đổi khi action không có tác dụng.

## Phân loại cổng thật hiện có

| Cổng | Làm gì | Không được suy ra |
|---|---|---|
| `node --check scripts/riviu_agent_mcp.mjs` | Parse syntax JS, không chạy adapter | Không kiểm kết nối, auth, tool behavior hay phone |
| `node --test scripts/test_mobile_mcp.mjs` | Test launcher/pin/env bằng fixture/temp | Không là behavioral test Riviu MCP, không chứng minh phone |
| `cargo test --locked -p riviu-script-engine` | Test compiler/runtime logic theo suite hiện hành | Không chứng minh capability của máy thật |
| `cargo run --locked -p riviu-core --example gui_replay` với CompatibilityPack hợp lệ trên stdin | Verify fixture local, không nối phone/provider, báo `deviceActions:0` | Không phải validator Flow hay live smoke |
| `lease_conflict_gate` | Bắt buộc truyền serial; gọi `list_devices()` thật để dựng route rồi acquire/release lease trong coordinator riêng. Main không gửi input hay khởi stream | Không offline, không kiểm lease của desktop đang chạy; không coi comment “mid-shift” là quyền chạy cạnh tranh USB; xác minh config/driver hiện hành và xin phép live probe |
| `root_route_gate` | Hỏi đường root/guard trên device, đọc là chính theo source hiện hành | Không offline; bỏ serial có thể chọn mọi máy; không chứng minh quyền factory reset |
| `device_files_gate` | Có thao tác ghi/xóa trên device | Không read-only, cần phép riêng |
| Mobile MCP check/probe | Có thể gọi adb version, khởi server, dùng SDK shim; `--devices` enumerate | Không thuần offline; không thay production control plane |

Không có căn cứ cho file `scripts/test_riviu_agent_mcp.mjs`; đừng tạo lệnh giả chỉ vì tên có vẻ đúng. Test fixture có thể ghi temp/build cache trên host, vẫn cần mô tả đúng.

## Dùng fixture replay

Nguồn: `crates/core/examples/gui_replay.rs`, `crates/core/src/ui_automation/profile.rs`.

CLI đọc JSON CompatibilityPack từ stdin, giới hạn input và gọi `verify_fixtures()`. Phải tìm fixture hợp lệ hoặc dựng theo DTO hiện hành; không bịa shape. Đọc `--help` của cargo không thay đọc contract của example. Nếu dùng file input, giữ dữ liệu nhạy cảm cục bộ; không ghi đè pack đã có.

Khi cần câu lệnh chuyển stdin trên Windows, ưu tiên subprocess của Python với `input=bytes` đọc từ fixture và `cwd` ở worktree hiện tại, hoặc shell redirection đã xác minh. Không đổi cwd sang checkout gốc. Chỉ chạy khi phạm vi công việc cần và cho phép test.

## Nguồn để lần theo lỗi

- `scripts/riviu_agent_mcp.mjs`: adapter và danh sách tool thật.
- `apps/desktop/src-tauri/src/inspector_commands.rs`: capture, lưu observation, resolve và intent/tap/readback.
- `apps/desktop/src-tauri/src/commands/mod.rs`: manual session/ownership.
- `crates/core/src/ui_automation/inspector.rs`: model/selector Inspector.
- `crates/core/src/ui_automation/profile.rs`: compatibility/fixture contract.
- `crates/core/src/device_control/leases.rs`: lease trong production.
- `crates/android-driver/examples/lease_conflict_gate.rs`, `root_route_gate.rs`: đọc flags và side effect trước dùng.
- `docs/agents/09-fleet-android.md`, `docs/agents/08-unified-agent-runtime.md`: Android/runtime contract.
- `docs/agents/02-wda-doc-truoc-khi-sua.md`: bắt buộc đọc toàn bộ trước sửa WDA/iOS.
- `tools/mobile-mcp/README.md`: chỉ canary Android disposable, không remote/cloud, không TikTok Like/Save/Comment/Follow/Post ngoài Riviu.

Nếu source/tài liệu khác tên hoặc đã đổi hành vi, báo và xác minh lại. Không áp quan sát lịch sử của memory/report như trạng thái live.
