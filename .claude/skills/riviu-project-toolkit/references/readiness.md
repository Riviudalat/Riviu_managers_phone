# Readiness và bảo trì bộ công cụ

## 1. Kiểm trạng thái trên máy đang làm việc

Repo chỉ mang theo các file được liệt kê trong [hướng dẫn thiết lập](../../../../docs/agent-toolkit-setup.md). Không mang trạng thái Connected/PASS, cấu hình tài khoản, dependency đã cài hay quyền thiết bị của một máy khác. Xác minh lại khi clone, đổi worktree/PATH/package/config; báo đúng tầng đã kiểm.

| Thành phần | Cách xác minh khi cần | Không được suy ra |
|---|---|---|
| Toolkit, ba skill phone, skill chạy app | Kiểm file trong `.claude/skills/` và danh sách skill phiên | Không chứng minh tool phụ hoặc runtime đã sẵn |
| Skill UI/Rust/debugging và skill host | Kiểm danh sách khả dụng, bản cài và nguồn; xem nhóm phụ thuộc trong hướng dẫn thiết lập | Không mặc định chúng đi kèm clone; thiếu thì dùng fallback tài liệu/test đúng phạm vi |
| Codex MCP | Kiểm CLI, entry MCP và auth theo cách không in secret; chỉ handshake/review khi được phép và có ích | Có cấu hình không có nghĩa tool đã hiện hoặc đã có review mới |
| Mobile MCP | Đối chiếu pin/launcher trong repo, chạy test/check/probe khi được phép | Tool list/Connected không chứng minh thao tác phone hoặc iOS |
| Mobilecli | Kiểm dependency/binary đúng nền tảng; version check khi cần | Không chứng minh quyền/capability thiết bị |
| Riviu MCP | Đối chiếu adapter, backend/API loopback, auth và danh sách tool thực tế | Token vắng trong shell không chứng minh app không có token; source có tool không có nghĩa đã đăng ký |
| Context7 / Playwright MCP / GitHub MCP | Kiểm từng server khi nhiệm vụ cần, gồm runtime/auth và tool exposure | Không mặc định đã cài/nối; browser runtime và API permission phải kiểm riêng |
| CLI cơ bản | Kiểm lệnh đúng nền tảng trên PATH và toolchain trong runbook | Không chứng minh auth, build hoặc driver thiết bị |
| Inventory/tool-index khác | Kiểm ngày và phép đo thực tế; coi là gợi ý tìm tool | Không tự bootstrap hoặc tin “npx có” nghĩa MCP sẵn |

Nếu cài dependency bằng `npm ci --ignore-scripts`, nói rõ lifecycle scripts chưa chạy. Một package kiểm được riêng không chứng minh toàn bộ desktop đã sẵn. Chỉ xử lý script thiếu khi có bằng chứng và phạm vi cho phép, không tự bật mọi lifecycle script để sửa suy đoán.

## 2. Chú ý worktree, scope và skill trùng tên

Mỗi máy tự đăng ký MCP qua launcher của checkout/worktree đang dùng theo hướng dẫn thiết lập:

- Không nói local-scope chắc chắn cô lập riêng một worktree nếu chưa kiểm config thực tế; CLI có thể chuẩn hóa worktree về cùng project key.
- Nếu entry local trỏ đường dẫn tuyệt đối và worktree bị dời/xóa, entry có thể hỏng. Kiểm executable, script, package pin và project key; không tự quay cwd sang checkout gốc.
- Skill personal cùng tên có thể được ưu tiên hơn skill project. Đọc phiên bản trong repo để áp dụng contract hiện hành; nếu muốn `/name` chạy bản project, người dùng cần bỏ xung đột personal theo hướng dẫn thiết lập. Không tự xóa hay thay liên kết cấp tài khoản.
- Thay entry chỉ trong phạm vi người dùng cho phép, kiểm target cũ và giữ MCP khác, không ghi đè toàn registry.
- Không lưu token vào lệnh `-e TOKEN=...`, transcript hoặc file tracked; dùng cơ chế credential được chấp thuận.

## 3. Kiểm tra nhẹ trước, không tự health-check mọi server

1. Xem danh sách skill/tool thực sự khả dụng của phiên.
2. Kiểm local paths/config/metadata cần thiết, chỉ in tên server và trạng thái/thiếu file, không in env/args nhạy cảm.
3. Nếu nhiệm vụ cần khởi/handshake và được phép, dùng command/launcher đã ghim. Chỉ gọi `tools/list` khi đủ; không gọi device enumeration để chứng minh server sống.
4. Nếu host chưa expose tool, hướng dẫn `/mcp` kiểm tra/reconnect hoặc mở phiên mới theo phiên bản CLI. Không giả có generic reload/tool-search command.
5. Auth/status không phải tự login/đổi scope. Thiếu quyền thì báo; không lấy credentials từ DB/secret store ngoài phạm vi.

Kiểm inventory theo file không chạy được handshake/auth thay người dùng. Đừng ghi một cột “ready=true” chỉ vì thấy package.

## 4. Những phần đáng bổ sung khi có điều kiện

### Ưu tiên 1: nối Riviu MCP cho công việc production

Giá trị riêng: dùng chung backend/session/lease/evidence của sản phẩm. Cần API loopback, credential hợp lệ, process/device scope và tool exposure. Chưa đủ điều kiện thì ghi deferred, không dựng fallback controller. Đây là nối adapter có sẵn, không cần cài thêm framework mobile.

### Ưu tiên 2: Context7 cho API drift

Chỉ thêm khi nhu cầu tra cứu lặp lại đáng kể và người dùng chấp nhận query đi ra dịch vụ ngoài; xác minh auth/rate limit/version coverage. Docs chính thức/local source vẫn là fallback tốt.

### Ưu tiên 3: Playwright MCP cho tương tác browser trực tiếp

Có lợi khi cần lặp chu trình DOM/console/network/screenshot ngoài test suite. Đã có Playwright tests không đồng nghĩa có MCP/browser runtime. Giữ Tauri native acceptance riêng; không lấy browser tool làm cớ bỏ skill chạy app.

### Không vội bổ sung

GitHub MCP nếu `gh` đủ; Filesystem/Git/Memory trùng công cụ nền; SQLite MCP nối DB thật; controller phone khác; Figma/Stitch/Sentry/cloud khi chưa có tài nguyên và nhiệm vụ tương ứng.

Khoảng trống chuyên môn **Tokio cancellation/process lifecycle, SQLite restart/transaction và package provenance** được giải bằng tài liệu/contract/test hiện hành trước. Không cần cài một skill mơ hồ có chữ “expert” để lấp tên; nếu một playbook lặp lại nhiều lần thì mới tách skill chuyên sâu dựa trên failure modes đã đo.

## 5. Chu trình bảo trì không lãng phí

- Sau thay đổi pin/toolchain/worktree/MCP config: kiểm phần liên quan, không cập nhật cả kho theo `latest`.
- Giữ source URL/commit, license và local adaptations của skill bên ngoài; giữ hash để phân biệt đã sửa với upstream. Hash khớp chứng minh byte, không chứng minh an toàn/hành vi.
- Skill tự soạn phải ghi `origin: local`; không gắn tên upstream/official nếu không phải.
- Không dùng một file trạng thái đã cũ làm bằng chứng live; cập nhật chỉ sau phép kiểm mới và ghi rõ phạm vi.
- Không có log “số lần dùng thấp” nào đủ để tự xóa skill hay MCP. Có thể ít dùng nhưng cần đúng lúc. Gỡ/tắt khi người dùng muốn hoặc có lý do rõ và quyền phù hợp.
- Chỉ tạo hook nếu có yêu cầu tự động theo sự kiện cụ thể; thiết kế hẹp, không network/device/API bất ngờ, có test và cách tắt. Toolkit và rule là hướng dẫn, không cưỡng chế thực thi.
