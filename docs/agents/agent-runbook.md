# Runbook tiếp nhận agent

## Thứ tự đọc

1. Root [`AGENTS.md`](../../AGENTS.md) và [`README.md`](../../README.md).
2. Toàn bộ [WDA safety](02-wda-doc-truoc-khi-sua.md) trước mọi thay đổi WDA/iOS.
3. [Kiến trúc](03-kien-truc.md), [runtime](08-unified-agent-runtime.md), [developer guide](../developer-guide.md).
4. [Operator guide](../operator-guide.md) và [UI contract](../ui-reference-matrix.md) khi chạm giao diện.
5. Các file còn lại trong [docs/agents](README.md) khi phạm vi đụng tới.

## Hợp đồng làm việc

- Sau mỗi thay đổi UI/Rust liên quan desktop: chạy Tauri dev và kiểm tra trước khi bàn giao.
  Chỉ đóng gói release/Setup khi cần giao bản cài. Lệnh dev nằm trong developer guide.
- Đọc `git status --short --untracked-files=all`; không reset/revert thay đổi không thuộc mình.
- Root AGENTS chỉ là cửa ngắn. Nội dung mới về hành vi người dùng đi vào README hoặc hướng dẫn sản phẩm; ràng buộc kỹ thuật đứng yên đi vào file đúng chủ đề dưới `docs/agents/`.
- Không ghi nhật ký thay đổi theo đợt vào kho tài liệu.
- Không lấy file lịch sử làm trạng thái hiện tại.
- Không thay manifest/device identity, reinstall, tap, Post hoặc retry ngoài phạm vi đang kiểm.
- Giữ stock/RT-MMO profile riêng, deadline trên request, session trước stream và helper/hash pin.
- `src/api.ts` vẫn là biên IPC; control plane chịu admission/ownership cho cả UI lẫn API/MCP.

## Dọn repository

Đọc [manifest dọn tài liệu](../archive/cleanup-2026-09-06.md). File không có reference
chỉ là ứng viên, không phải bằng chứng vô giá trị. Phân biệt asset đang bundle, fixture,
snapshot, log nghiệm thu, file phụ tái tạo được và cache compiler.

`.superpowers/baseline-*` có thể là Git worktree; không xoá bằng recursive cleanup.
`target/` chứa evidence và rollback ngoài cache. Không đụng stash hay dữ liệu vận hành
để làm `git status` đẹp hơn.

## Nghiệm thu Publish và bàn giao

### Đầu vào, ownership và candidate

Ghi receipt ngoài docs: media root tuyệt đối đã kiểm tồn tại/quyền đọc, manifest,
bundle/caption/media hash, mapping số máy–serial–account–bundle theo thứ tự,
package/version/locale, source revision/diff/hash, binary path/hash/version thực chạy,
helper hash và process/controller owner/CDP WebView. Source mới không cập nhật
process cũ; sau sửa Rust, ROOT build/chạy candidate và kiểm provenance trước phone.
Không dùng receipt của version/activation/run cũ để chứng nhận candidate mới.

Giữ nguyên media root đã duyệt; nó khác repo/evidence root. Khi quét lỗi, giữ exact
command/input/sourceRoot/bundle/stdout/stderr/exit và lý do đã chứng minh. Không đổi
root, bỏ bundle, caption hoặc serial để né lỗi. ROOT xác nhận sửa input rồi preflight
mới trước public effect. Account không đọc rõ giữ Unknown/Blocked, không suy handle
từ nickname, số máy hay vị trí. Ownership chưa rõ phải đưa owner/generation/journal/
serial cho ROOT, không đoán hoặc chạy controller cạnh tranh để né busy.

Task phân quyền là giới hạn thực thi: khi ROOT giữ Cargo, native/controller, ADB/
phone, DB/Sheet vận hành và commit/push, worker chỉ review source/sửa file được giao
và bàn giao lệnh/gate chưa chạy. Harness nối app cũng thuộc quyền ROOT. Preflight/
submit có phone/network; publish_sheet_check có thể lưu cấu hình, không là read-only.
Không tăng timeout, force-drop lease, xóa helper owner/journal chưa đối soát hoặc
replay Post/Send/Sheet request chưa rõ. Chỉ drain/nhả owner đã chứng minh an toàn.

### Start, dev scope và rotation

Đối chiếu scripts/publish_acceptance.mjs và [hướng dẫn harness](../development/publish-acceptance.md).
Lượt mới dùng --protocol start; report Create/Execute cũ chỉ observe. Cần Node và
Playwright hiện có ở apps/desktop/node_modules. CDP loopback thuộc đúng Tauri WebView
đã duyệt; không dùng browser mock hoặc tự mở app khác. --real-android true kiểm
roster USB connected/ready của AppState, không tự loại serial bị chặn.

ROOT tạo scope JSON UTF-8 tuyệt đối, activationId mới (16–128 ký tự [A-Za-z0-9_-])
và đúng deviceIds. Debug controller khởi động với các biến sau; startup thực hiện
qua developer guide trên candidate đã xác minh, không khởi động app thứ hai:

```powershell
$env:RIVIU_DEV_MANUAL_ACCEPTANCE = '1'
$env:RIVIU_DEV_ACCEPTANCE_SCOPE = $scopeFile
$env:RIVIU_DEV_ACCEPTANCE_ACTIVATION = $activationId
```

Immediate: trước startup, campaignIds: [], capabilities false hoặc absent. Preflight
lưu UUID ở preflight-request.json và thay scope atomic sang UUID ấy cùng ba cờ true:
publishVerification, sheetDelivery, publishCleanup. Scheduled: reserve offline trước
startup; lấy reservation.requestId từ preflight-request.json, tạo scope tĩnh chứa
UUID ấy, toàn bộ serial duyệt và cả bốn cờ true (thêm publishSchedule). Startup trước
runAt, giữ scope nguyên vẹn. Backend pin device list lúc startup và campaign/cờ lúc
active đầu tiên. Đổi cohort/campaign/cờ phải ROOT đối soát, drain/nhả owner rồi xoay
controller/activation mới; không xóa unresolved debt để cho rotation qua gate.

Report directory riêng cho từng cohort, nhưng không tạo directory mới để né intent
chưa rõ. Preflight giữ digest/preparationId/writer/epoch/account và confirmation.
Submit fsync start-intent.json trước một publish_start; mất ACK chỉ observe đúng
report/requestId. Không submit lại hay đổi UUID. Observe đọc status/readback, không
kích verifier, resume hoặc retry Post/Sheet. Scope đã active nhưng preflight lỗi:
ROOT đối soát owner/intent trước lượt mới, không đổi campaign trong process cũ.

### Lệnh ROOT thực hiện theo thứ tự

Chạy từ repo candidate đã xác minh. ROOT điền slot từ evidence trước chạy:
$udids/$bundleIds là CSV cùng thứ tự một-một; $sheetId là ID, $sheetGid là số nguyên;
$scopeFile/$reportDir là path tuyệt đối; $cdp là HTTP IP loopback/cổng; $runAt là giờ
local YYYY-MM-DDTHH:mm:ss không Z/offset; $waitSeconds là ngân sách observe đã duyệt,
không deadline engine. Nếu cần chọn WebView, thêm cùng --page-url $pageUrl vào mọi
lệnh cohort. Không dùng số máy làm UDID/bundle. Kiểm exit từng bước; không submit
nếu preflight chưa đạt và ROOT chưa duyệt mapping/content/Sheet target/epoch.

```powershell
# Immediate: cohort 1–10, scope inactive trước startup đã xác minh.
$common = @('--protocol','start','--report-dir',$reportDir,'--udids',$udids,
  '--source',$source,'--bundle-ids',$bundleIds,'--sheet-id',$sheetId,
  '--sheet-gid',"$sheetGid",'--dev-scope',$scopeFile,'--real-android','true','--cdp',$cdp)
node scripts/publish_acceptance.mjs --mode inspect @common
node scripts/publish_acceptance.mjs --mode preflight @common
$confirmation = (Get-Content -Raw -Encoding UTF8 (Join-Path $reportDir 'preflight.json') | ConvertFrom-Json).confirmation
node scripts/publish_acceptance.mjs --mode submit @common --confirm $confirmation
# Có durable intent: chỉ observe, kể cả ACK chưa rõ hoặc submit báo lỗi.
node scripts/publish_acceptance.mjs --mode observe @common --wait-seconds "$waitSeconds" --poll-seconds 10

# Scheduled: cohort 11–20, sau rotation đã đối soát; report/scope/activation mới.
# Điền lại $common cho cohort này; cùng runAt cho cả 10 máy.
$scheduled = $common + @('--run-at',$runAt,'--schedule-case','publish')
node scripts/publish_acceptance.mjs --mode reserve @scheduled
$requestId = (Get-Content -Raw -Encoding UTF8 (Join-Path $reportDir 'preflight-request.json') | ConvertFrom-Json).requestId
# ROOT tạo scope tĩnh đúng $requestId + 4 cờ true, startup trước runAt.
node scripts/publish_acceptance.mjs --mode inspect @scheduled
node scripts/publish_acceptance.mjs --mode preflight @scheduled
$confirmation = (Get-Content -Raw -Encoding UTF8 (Join-Path $reportDir 'preflight.json') | ConvertFrom-Json).confirmation
node scripts/publish_acceptance.mjs --mode submit @scheduled --confirm $confirmation
node scripts/publish_acceptance.mjs --mode observe @scheduled --wait-seconds "$waitSeconds" --poll-seconds 10
```

Nếu có caption/sound override, thêm cùng --content-snapshot JSON đã duyệt vào $common
trước reserve/preflight, giữ nguyên tới submit. Inspect/reserve/preflight exit 0 chỉ
chứng minh bước ấy, chưa có bài public. Pending/hết hạn observe không dừng worker.

### Admission và proof nghiệm thu

claim_publish_pipeline (crates/core/src/db/publish_pipeline.rs) truyền admitted_at
vào enqueue_publish_jobs (db/publish_dispatch.rs). Contract hiện tại bỏ deadline_ms
cho cohort admission trong runAt tới runAt + 30s; resume trễ giữ deadline. Defect
đáng tin của đường cũ: job imported đi thẳng compose, chờ capacity khi started_at_ms
còn NULL rồi expire_publish_dispatch_deadlines ghi schedule_capacity_deadline. Job
đã chạy transfer giữ started timestamp khi chuyển compose; không gộp hai đường.
Không khẳng định defect cũ còn tồn tại khi candidate đã có fix. ROOT kiểm binary/
hash, admission và queued rows sau +30s cùng gate late-resume/deadline/no-effect;
không tăng cửa 30s hoặc stagger cohort để thay chứng nhận same-runAt.

Mẫu số cố định theo roster: blocked/offline/helper-recovery/missing proof vẫn còn
hàng với lý do riêng. Máy 1–10 immediate, 11–20 cùng runAt là 20 nghĩa vụ, không là
hai wave 20 máy. PASS cần 20 bài public đúng media/caption/account/time, canonical
link theo publication identity, 20 authenticated Sheet receipt + cell readback đúng
assignment/publication/revision/target/reportingEpoch, cleanup đúng importId và
owner nhả có proof. publicationVerified, URL, sheetSent, urlReadback và mediaCleaned
là lớp riêng. Screenshot/HTTP 200/composer đóng/source-test GREEN/URL trần không
thay proof. Harness dùng publish_sheet_readback qua OAuth backend; không lấy token
hoặc CSV thay readback. Source review không chứng nhận phone/scheduled/Sheet gate.

Bàn giao một lần: file/patch/hash, source sites/risk, command/input/stdout/stderr/exit
của gate đã chạy, baseline/modified/rollback, serial-keyed outcomes, owner/intent
chưa rõ và gates chưa chạy. Run context (source root/version/evidence root) giữ trong
receipt ngoài docs, không ghi incident log hoặc số PASS tạm vào runbook đứng yên.
