# Bộ công cụ agent cho Riviu

Bộ skill trong repository hướng dẫn chọn, dùng và kiểm chứng công cụ cho toàn dự án. **Clone repo không tự cài MCP/runtime, cấp quyền thiết bị hoặc chuyển credentials từ máy khác.** Không có hook tự chạy hay cấu hình MCP chứa token được phân phối cùng bộ này.

## Những gì đi cùng Git

| Thành phần | Vai trò |
|---|---|
| [riviu-project-toolkit](../.claude/skills/riviu-project-toolkit/SKILL.md) | Điểm vào điều phối; 5 references về định tuyến, MCP, kiểm chứng, readiness và tình huống mẫu |
| [riviu-phone-automation](../.claude/skills/riviu-phone-automation/SKILL.md) | Quan sát và thao tác điện thoại qua controller/session/lease đúng phạm vi |
| [riviu-interaction-flows](../.claude/skills/riviu-interaction-flows/SKILL.md) | Flow/Macro, import, điều kiện/chờ, nhiều máy, cancel/retry và bằng chứng |
| [riviu-device-diagnostics](../.claude/skills/riviu-device-diagnostics/SKILL.md) | Chẩn đoán transport/session/perception/input/verifier, fixture/headless/live |
| [run-riviu-managers-phone](../.claude/skills/run-riviu-managers-phone/SKILL.md) | Skill có sẵn của dự án cho build/run/screenshot/kiểm desktop |
| [Rule định tuyến](../.claude/rules/riviu-toolkit.md) | Nhắc chọn toolkit cho việc đáng kể, giữ quyền và đọc references theo nhu cầu |

Đây là **file thật**, không phải symlink/junction tới home của người tạo. Bốn skill `riviu-*` là hướng dẫn tự soạn cho dự án, không phải sản phẩm chính thức của GenFarmer hoặc nhà cung cấp MCP. Bản chuẩn để sửa và review là `.claude/skills/` trong repo; bản runtime `.agents/` của một số host không phải nguồn chuẩn.

Không đưa lên Git: `~/.claude.json`, settings cá nhân, `.installation.json`, token, `.env`, `node_modules`, DB vận hành hoặc raw screenshot/hierarchy có dữ liệu tài khoản. `.gitignore` chỉ cho theo dõi rule dự án được chỉ định, không mở toàn bộ `.claude/`.

## Mở trên máy mới

1. Clone/mở **checkout hoặc worktree đang làm việc**. Đọc [AGENTS.md](../AGENTS.md) và [runbook agent](agents/agent-runbook.md).
2. Với Claude Code, kiểm danh sách skill có `riviu-project-toolkit` và ba skill phone. Có thể gọi `/riviu-project-toolkit` kèm việc cần làm. Nếu client không hỗ trợ Skill, đọc trực tiếp điểm vào trong repo và references theo phạm vi; không tuyên bố đã gọi tool Skill.
3. Chỉ thiết lập toolchain cần cho nhiệm vụ theo [developer guide](developer-guide.md). Đọc Markdown không cần cài Rust, adb, browser hay MCP.
4. Kiểm dependency và quyền trước khi build/chạy. Test/mock/handshake không thay bằng chứng native, thiết bị thật hoặc bản cài.
5. Nếu cần MCP, nối từng server theo phần bên dưới; đừng cài cả catalog hoặc chạy phone để kiểm tra skill vừa nhận diện.

### Tránh bản personal che bản trong repo

Theo [tài liệu Claude Code về skill](https://code.claude.com/docs/en/skills), khi cùng tên thì mức ưu tiên là **enterprise → personal → project**. Vì vậy `/riviu-project-toolkit` có thể nạp bản cũ ở `~/.claude/skills/` thay cho repo. Frontmatter `paths` không giải quyết xung đột này.

- Kiểm **base directory** khi skill được nạp. Điểm vào của project phải ở `.claude/skills/` trong checkout đang làm việc.
- Nếu có bản personal trùng tên, so sánh trước. Người dùng có thể sao lưu/di chuyển bản personal ra ngoài thư mục skill được quét, rồi mở phiên mới. Với junction, xử lý entry liên kết, không recursive-delete thư mục đích.
- Agent không tự xóa/đổi liên kết cấp tài khoản chỉ để giành precedence. Trong lúc chưa xử lý, đọc trực tiếp bản repo và tuân contract dự án, báo rõ nguồn đang dùng.
- Enterprise policy có thể chặn project skills; không tìm cách vượt policy. Dùng đường tài liệu được cho phép hoặc nhờ quản trị viên cấu hình.

## Skill phụ thuộc và fallback

**Bộ tối thiểu để đọc và dùng hướng dẫn là các file đi cùng Git ở trên.** Những tên còn lại trong bảng định tuyến là skill chuyên môn theo nhiệm vụ, không phải tất cả đều được vendored hoặc tự cài khi clone.

| Nhóm | Khi nên có | Nếu chưa có |
|---|---|---|
| `systematic-debugging`, `test-driven-development`, `verification-before-completion` | Điều tra lỗi, đổi hành vi và nghiệm thu | Theo quy trình bằng chứng/regression/cổng trong toolkit và runbook; không giả đã gọi skill, không hạ chuẩn kiểm chứng |
| `rust-best-practices` | Viết/review Rust | Convention repo, Cargo/lints, tài liệu Rust/Tokio/Tauri/rusqlite đúng phiên bản |
| UI/design/React: `ui-*`, `frontend-design`, `vercel-*`, `ui-ux-pro-max` | Sửa giao diện, token, component hoặc hiệu năng | [UI contract](ui-reference-matrix.md), code/component hiện có và test; không đổi stack để hợp skill |
| `senior-system-design`, `senior-mobile-dev`, `fastapi` | Kiến trúc/contract, source helper mobile hoặc Python API tương ứng | Tài liệu module/framework hiện hành; chỉ cài bộ có nguồn xác minh nếu cần lặp lại |
| `code-review`, `simplify`, `security-review`, `dataviz`, `update-config`, `run` | Chức năng do host/plugin cung cấp trong một số môi trường | Xem danh sách khả dụng; dùng review/công cụ/tài liệu đúng nhiệm vụ thay vì bịa tên skill |
| `docs-generator`, `diagram-generator`, skill security chuyên biệt | Tài liệu/sơ đồ/assessment thật sự cần | Hướng dẫn docs/review của dự án; không tự cài bộ pentest cho phát triển thông thường |

Một skill được coi là chuyên môn cần dùng khi nhiệm vụ khớp, nhưng việc nó vắng không cho phép agent tự mở rộng phạm vi cài đặt. Nêu thiếu phần nào, dùng fallback tương đương nếu đủ; nếu thiếu công cụ thiết yếu thì báo blocker thay vì báo hoàn tất.

### Nguồn bên ngoài đã được chọn

Những liên kết sau là nguồn, **không phải script cài tự động và không khóa phiên bản cho repository này**:

| Nguồn | Phần hữu ích |
|---|---|
| [Apollo skills](https://github.com/apollographql/skills/tree/main/skills/rust-best-practices) | `rust-best-practices`, gồm `SKILL.md` và `references/` |
| [Superpowers](https://github.com/obra/superpowers/tree/main/skills) | `systematic-debugging`, `test-driven-development`, `verification-before-completion` |
| [Anthropic skills](https://github.com/anthropics/skills/tree/main/skills/frontend-design) | `frontend-design` |
| [Vercel agent skills](https://github.com/vercel-labs/agent-skills) | React best practices, composition patterns, web design guidelines |
| [UI UX Pro Max](https://github.com/nextlevelbuilder/ui-ux-pro-max-skill) | Tra cứu UI/UX bằng dữ liệu và script |

Khi cài thủ công, chọn và lưu **commit/tag cụ thể**, đọc nội dung và license trước; giữ nguyên references/scripts/data cần thiết và license. Standalone cần thư mục skill + `SKILL.md` trong vị trí mà client hỗ trợ; plugin có cách cài/namespace riêng. Không chỉ copy entrypoint rồi bỏ tài liệu phụ, không cài nguyên plugin kèm hooks/permissions khi chỉ cần hướng dẫn.

Bản cài ngoài có thể yêu cầu đường dẫn plugin, namespace `superpowers:`, Python hoặc công cụ khác. Xác minh và ghi lại local adaptation; không hardcode home/worktree của người khác. Đặc biệt:

- Không làm theo câu “xóa code để viết lại TDD” đối với code người dùng/phiên khác.
- Không bật `allowed-tools`, hooks, all-features, cài CLI bổ sung hoặc bỏ guard chỉ vì upstream đề xuất.
- Mẫu Next.js/marketing không thay contract Tauri/Vite hoặc UI vận hành của Riviu.
- Các bộ skill cá nhân không rõ nguồn không được tự động coi là dependency chuẩn của dự án.

## Nối Mobile MCP khi thực sự cần

Đọc [runbook Mobile MCP](../tools/mobile-mcp/README.md) trước. Nó là dev-only, chỉ dành cho **Android canary riêng** theo phạm vi cho phép; không phải production controller và không được dùng cho TikTok Like/Save/Comment/Follow/Post ngoài Riviu.

Các bước này **cài dependency/khởi server**, không phải chỉ đọc tài liệu. Chạy từ gốc checkout/worktree hiện tại khi đã quyết định thiết lập:

```powershell
npm ci --prefix apps/desktop
node --test scripts/test_mobile_mcp.mjs
node scripts/run_mobile_mcp.mjs --check --require-adb
node apps/desktop/scripts/mobile_mcp_probe.mjs
```

`npm ci` chạy lifecycle scripts mặc định và ghi dependency host; xem package/lock trước. `--check` có thể gọi `adb version`, probe khởi stdio server nhưng không gọi device tool nếu không có `--devices`. Không thêm `--devices` để chứng minh handshake.

Sau khi các bước phù hợp đạt, đăng ký **cấu hình cá nhân/local**, không ghi `.mcp.json` chứa cấu hình máy lên repo:

```powershell
$node = (Get-Command node -ErrorAction Stop).Source
$launcher = (Resolve-Path "scripts/run_mobile_mcp.mjs").Path
claude mcp add --scope local --transport stdio mobile-mcp -- "$node" "$launcher" --require-adb
```

Hoặc shell POSIX từ cùng gốc worktree:

```bash
claude mcp add --scope local --transport stdio mobile-mcp -- node "$PWD/scripts/run_mobile_mcp.mjs" --require-adb
```

Nếu tên server đã tồn tại, kiểm entry hiện hành và xin phép trước khi thay, không remove/add mù. Tùy client, local config của các worktree có thể chuẩn hóa về cùng project; entry tuyệt đối cần kiểm lại khi dời/xóa worktree. Không `cd` về checkout khác để đăng ký cho tiện.

Mở `/mcp` để kiểm tra/reconnect trong Claude Code hoặc mở phiên mới nếu host chưa nhận server. `claude mcp get mobile-mcp` kiểm trạng thái và có thể khởi server để health-check; **Connected không chứng minh tool đã xuất hiện trong cuộc trò chuyện hay phone đã sẵn sàng**. Runtime/device behavior vẫn phải kiểm theo nhiệm vụ được phép.

Muốn gỡ khi người dùng yêu cầu, kiểm entry và scope trước rồi dùng `claude mcp remove mobile-mcp -s local`. Không xóa server của scope khác hoặc runtime dùng chung.

## Riviu MCP và các MCP tùy chọn

| MCP | Điều kiện thiết lập | Giá trị riêng và fallback |
|---|---|---|
| Riviu MCP | Backend/API loopback đang chạy, token hợp lệ qua cơ chế credential được chấp thuận, adapter [`scripts/riviu_agent_mcp.mjs`](../scripts/riviu_agent_mcp.mjs), client expose tool | Dùng chung ownership/evidence của Riviu; chưa đủ điều kiện thì deferred, không fallback sang controller cạnh tranh |
| Codex | CLI/auth/MCP được phép, đúng project/cwd và phạm vi dữ liệu | Review độc lập thay đổi đáng kể; thiếu thì review trực tiếp hoặc subagent phù hợp, không gửi bí mật hay gọi model chỉ cho đủ lượt |
| [Context7](https://github.com/upstash/context7) | Auth/hạn mức/coverage phù hợp; chấp nhận query đi ra dịch vụ ngoài | Tra docs đúng version; thiếu thì dùng docs/source chính thức |
| [Playwright MCP](https://github.com/microsoft/playwright-mcp) | Runtime Node/browser, profile thử riêng và client đã nối | DOM/console/network frontend; test suite có sẵn có thể đủ, không thay Tauri thật |
| [GitHub MCP](https://github.com/github/github-mcp-server) | OAuth/PAT scope tối thiểu, phạm vi thao tác được phép | Chỉ thêm khi có lợi hơn `gh`; CLI có sẵn vẫn phải kiểm auth |

Không yêu cầu dán token vào chat, không tìm token trong DB/secret store ngoài phạm vi, không lưu `-e TOKEN=...` vào lệnh được chia sẻ. Các MCP optional phải theo tài liệu chính chủ hiện hành; không dùng credential của người đã tạo repo.

## Kiểm tra khi sửa bộ hướng dẫn

- Giữ entrypoint và references cùng thư mục; dùng link tương đối, file UTF-8, không thêm đường dẫn home/serial/token.
- Xác minh file mới không bị ignore, là file thật và không mang `.installation.json` hay trạng thái PASS của máy cá nhân.
- Khi file đã được Git theo dõi, chạy cổng docs từ gốc repo:

```powershell
py -3.12 -m unittest scripts.test_check_docs -v
py -3.12 scripts/check_docs.py
```

Trên môi trường không có Windows Python launcher, dùng interpreter được runbook chấp thuận. Cổng docs quét file Git theo dõi; không lấy kết quả trước khi thêm file mới vào index để khẳng định file mới đã được kiểm. Có thể dùng index tạm cho phép kiểm đầy đủ mà không stage thay người dùng.

- Chạy [tình huống mẫu](../.claude/skills/riviu-project-toolkit/references/scenarios.md) ở mức rehearsal khi đổi logic định tuyến/quyền. Không cần khởi app/điện thoại để nghiệm thu thay đổi tài liệu.
- Phân biệt đã chuẩn bị file trong working tree, đã stage, đã commit và đã push. Việc đưa hướng dẫn vào repo không tự cấp quyền commit/push.
