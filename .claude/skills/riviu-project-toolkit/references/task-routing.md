# Định tuyến công việc toàn dự án

Chọn theo **yêu cầu hiện tại**, không theo số skill đã cài. Một skill có thể hữu ích ở dự án khác nhưng không cần nạp cho Riviu. Kiểm tên trong danh sách khả dụng; skill từ plugin có thể có namespace khác, không đoán tên.

Bảng dưới là bản đồ theo năng lực, không phải toàn bộ dependency bắt buộc của clone. Các nhóm có sẵn, cần cài theo nhiệm vụ và fallback được mô tả trong [hướng dẫn thiết lập](../../../../docs/agent-toolkit-setup.md). Nếu thiếu skill chuyên môn, nói rõ và dùng hợp đồng/source/docs chính thức cùng kiểm thử tương ứng; không giảm cổng kiểm hoặc giả đã gọi skill.

## 1. Ma trận chọn skill

| Việc đang làm | Skill dẫn dắt | Bổ sung đúng lúc | Đầu ra cần có |
|---|---|---|---|
| Lỗi build/test/runtime chưa rõ nguyên nhân | `systematic-debugging` | Skill đúng ngôn ngữ hoặc `riviu-device-diagnostics` khi liên quan phone | Repro, lớp lỗi, giả thuyết có bằng chứng, regression |
| Viết/refactor Rust | `rust-best-practices` | `test-driven-development`; `senior-system-design` khi đổi contract/concurrency | Code theo convention, test hành vi, lint/build đúng phạm vi |
| Tauri IPC, state, capabilities, process lifecycle | `rust-best-practices` + tài liệu Tauri v2 | `senior-system-design`, skill chạy app khi cần nghiệm thu | Boundary frontend/backend rõ, ownership, lỗi/cleanup và kiểm Tauri thật |
| SQLite migration, scheduler, claim, CAS, restart, cancel | `senior-system-design` | Rust + TDD; sơ đồ khi quan hệ trạng thái khó hiểu | Transaction/identity invariant, test xung đột/restart/uncertain; không chỉ happy path |
| Thiết kế lại cấu trúc UI/state frontend | `senior-web-dev` | `vercel-composition-patterns` cho API component, `vercel-react-best-practices` cho hiệu năng | Component/state ownership hợp lý, test consumer-visible |
| Layout/states/forms/tables | `ui-implementation` | `ui-design-system` nếu token/theme đổi; `ui-quality-review` ở cuối | Empty/loading/error/disabled, keyboard, responsive đúng ngữ cảnh |
| Chọn visual/typography hoặc cải tiến thẩm mỹ | `frontend-design` hoặc `redesign-existing-projects` | `ui-ux-pro-max` để tra một vấn đề cụ thể | Hướng nhất quán, bảo toàn contract Riviu; không đổi font/màu tùy hứng |
| Landing/portfolio riêng | `design-taste-frontend` khi brief khớp | UI implementation/review | Không áp bố cục marketing vào bảng vận hành |
| Chart, KPI, visualization | `dataviz` trước khi viết chart | Token/UI/a11y cần thiết | Đúng dữ liệu, màu/axis/legend/tooltip và trạng thái thiếu dữ liệu |
| Hành động trên điện thoại | `riviu-phone-automation` | Diagnostics khi thiếu bằng chứng/không chắc target | Exact device, controller, precondition → action → postcondition |
| Macro/Flow, nhiều bước/máy, import GenFarmer | `riviu-interaction-flows` | Rust/TDD hoặc phone automation khi thực sự live | Draft/compile/save/run tách biệt, action catalog, deadline/cancel/evidence |
| Có hình nhưng không điều khiển được, hierarchy/OCR sai | `riviu-device-diagnostics` | `systematic-debugging` | Xác định discovery/transport/session/perception/input/verifier; fixture hoặc live có phép |
| Source helper/agent Android/iOS | `senior-mobile-dev` | Rust/diagnostics nếu boundary liên quan | Lifecycle/permissions/signing, capability đúng nền tảng; iOS phải đọc WDA safety |
| FastAPI/perception service Python | `fastapi` | TDD + debugging; không áp vào mọi script Python | API/types/timeouts, fixture OCR/vision, lint/pytest phù hợp |
| Google Sheets/OAuth/Apps Script | `senior-system-design` cho contract | `identity-federation` cho assessment OAuth; `api-security` khi đánh giá API | Receipt/readback, permission, migration/reconciliation, secret redaction |
| Review code/thay đổi | `code-review` | Chuyên môn đúng vùng; Codex độc lập nếu giá trị đủ lớn | Finding có failure scenario, xác minh và test; không coi ý kiến model là fact |
| Dọn code đã đổi | `simplify` | Test/regression | Giảm trùng/phức tạp, giữ behavior; không thay review correctness |
| Review security source/desktop/API | `security-review` hoặc `code-audit` | `thick-client`, `api-security`, `identity-federation` đúng phạm vi | Trust boundary, dữ liệu/effect, finding có bằng chứng và remediation |
| Dependency/sidecar/SBOM/provenance/signing/updater | `supply-chain-security` cho assessment | Skill build của dự án, review/TDD | Hash/license/version/runtime packaged; không suy source chạy được là installer chạy được |
| Build/run/screenshot/verify desktop | `run-riviu-managers-phone` | `verification-before-completion` | Bằng chứng desktop thực; không tự chạy tác vụ thật trên phone |
| README/runbook/API docs | `docs-generator` | `diagram-generator` khi sơ đồ làm rõ cơ chế | Tài liệu nhiệm vụ ngắn, nguồn đúng, lệnh không bịa |
| Cấu hình MCP/hooks/permissions | `update-config` | Tài liệu MCP chính chủ và source launcher | Merge không ghi đè, tối thiểu quyền, readiness đúng tầng |
| Tích hợp LLM/provider | Skill/tài liệu đúng provider đang làm | Đánh giá prompt/eval theo yêu cầu | Không đổi provider/model do sở thích; không gọi API tốn phí ngoài phạm vi |

Skill chung cho thay đổi hành vi là `test-driven-development`; trước tuyên bố hoàn tất là `verification-before-completion`. Không đọc lại chúng ở mỗi tool call nếu đã nạp và còn phù hợp.

## 2. Chuỗi sử dụng mẫu

### Sửa lỗi Rust timeout

`systematic-debugging` → repro + trace qua boundary → `rust-best-practices` phần lỗi/lifecycle liên quan → regression theo TDD → review → cổng phù hợp.

Không mặc định tăng mọi timeout hoặc bật retry. Với WDA, deadline nằm trên request theo safety guide, không hủy HTTP giữa chừng bằng wrapper timeout.

### Thêm tính năng thao tác điện thoại

`riviu-interaction-flows` để chốt contract và action/engine → test/fixture → Rust/frontend skill đúng phần → `riviu-phone-automation` chỉ khi tới live được phép → xác minh từng máy.

Không dùng Mobile MCP để chạy production hoặc dùng manual tap bỏ verifier của engine.

### Cải tiến một màn UI

Đọc UI contract → chọn `ui-implementation` → composition/performance chỉ nếu có vấn đề thật → `ui-quality-review` → unit/typecheck/lint/e2e theo repo → Tauri dev khi runbook yêu cầu và được phép.

Không cần design-system mới, full visual redesign, tất cả skill taste hay model review thứ hai cho một lỗi spacing.

### Phát hành bản cài

Skill chạy/build dự án → version/config/secret prerequisites → build/bundle/smoke → supply-chain checks phù hợp → báo artifact. Commit/push/tag/publish cần quyền riêng; không tạo tag vì “release checklist” của skill chung.

## 3. Phần không áp mặc định

- `go-rust-reverse` là phân tích binary Go/Rust, không phải viết Rust.
- `apk-reverse`, `ida-reverse`, `mobile-reverse`, `protocol-reverse` chỉ dùng khi thật sự phân tích artifact/giao thức trong phạm vi cho phép. Sửa Android driver không tự là reverse.
- `supabase-postgres-best-practices`, Prisma, NestJS, Turborepo, Docker/K8s/cloud chỉ dùng nếu phần việc có công nghệ tương ứng; Riviu đang dùng SQLite không được tự đổi DB.
- `payment-ledger` dành cho tiền/thanh toán, không mặc nhiên hợp effect ledger của task.
- Brandkit/imagegen/Stitch cần brief và công cụ tương ứng; có file skill không có nghĩa có khả năng sinh ảnh/Stitch.
- Bộ pentest/attack-chain/exploit/EDR không phải bộ nâng chất lượng mặc định. Giữ authorization và ranh giới an toàn của từng nhiệm vụ.
- `browser-automation` không thay controller điện thoại hay Win32 Tauri. Skill có thể chỉ ra tool chưa cài; không tự bootstrap ngoài phạm vi.

## 4. Đổi phạm vi giữa chừng

Khi phát hiện phần sửa lan sang mảng khác, dừng mở rộng tự phát: nêu ảnh hưởng, nạp đúng skill chuyên môn và điều chỉnh kiểm chứng. Nếu cần quyết định thực sự của người dùng thì hỏi; nếu chỉ là quy ước/lệnh tìm được từ repo thì xác minh và tiếp tục.

Không gọi workflow đa agent quy mô lớn chỉ vì toolkit nhắc review/parallel. Chỉ dùng Workflow khi người dùng hoặc skill được phép rõ ràng yêu cầu; một subagent mục tiêu cụ thể thường đủ. Sau delegate, không tự tìm lại cùng nguồn để tốn token; chỉ kiểm độc lập kết luận quan trọng cần chứng minh.
