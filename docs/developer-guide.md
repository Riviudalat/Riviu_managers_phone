# Hướng dẫn phát triển

Stack giữ nguyên: Rust workspace, Tauri 2, React/TypeScript/Vite. `src/api.ts` là biên
IPC frontend. Không thêm một control plane riêng để đi vòng ownership/admission hiện có.

## Bản đồ trách nhiệm

| Vùng | Chủ sở hữu mã | Đầu vào và đầu ra | Retry, cleanup và cổng |
|---|---|---|---|
| Shell và điều hướng | `apps/desktop/src/App.tsx`, `workspaceDraft.ts`, `deviceSurface.ts` | PageId, bản nháp, target riêng từng workspace -> vùng đang mở, dialog quyết định | stale response không ghi đè; test App/workspaceDraft, e2e pages |
| Thiết bị/preview | `useFleet.ts`, `viewStore.ts`, `DeviceTile.tsx`, `PhoneCanvas.tsx`; Rust `view_hub`, `commands`, drivers | roster/stream generation -> trạng thái và frame | cleanup theo generation/owner; viewProtocol parity, Android driver, stream-remount tests |
| Ownership/admission | Rust `lib.rs`, `state.rs`, `commands/`, core driver traits | actor/target/effect intent -> dispatch hoặc lỗi có kiểu | không duplicate effect; admission/concurrency tests trước khi đổi biên |
| Tương tác | `InteractionPopup.tsx`, core `interaction` | URL hiện tại, revision, prepared content -> campaign/assignment/evidence | uncertain không gửi lại mù; parser, effect và JobsPanel regressions |
| Nuôi | `NurturePopup.tsx`, core `nurture` | phiên, nhịp, effect plan -> quan sát/outcome/cost | budget hữu hạn, credential riêng; nurture tests và readiness UI |
| Đăng bài | `PublishPage.tsx`, core `publish`, desktop `publish_commands/{mod,preflight,execution,sheet,legacy,tests}.rs` | media/target/preflight -> durable projection và outbox | Post/URL/Sheet/cleanup tách scope; publish tests, preflight và restart recovery |
| Flow/điều phối | `components/flow`, `components/orchestration`, core `flow` | graph/node identity/revision -> execution history | không thay device Flow bằng fleet orchestration; validate/save/archive/import tests |
| App/nội dung | `AppsPage.tsx`, `MaterialPage.tsx`, library ledger | artifact snapshot + targets -> batch/item outcomes | queued cancel; restart running -> uncertain, không auto replay; ledger regressions |
| Lịch sử | `JobsPanel.tsx`, `DataPage.tsx`, aggregate query | source/filter/window -> total + page + hydrated detail | filter trước hydrate; >10.000 nguồn báo thu hẹp; projection/query tests |
| Settings/API | `SettingsPanel.tsx`, `ApiPage.tsx`, settings/local API commands | section draft/credential/listener config -> persisted value + actual listener | readback và stale-response guard; section save tests, Local API tests |
| iOS/WDA | `crates/ios-driver`, `sidecars/pymobiledevice3`, `sidecars/wda` | manifest/auth/session -> capability/transport | giữ thứ tự session-trước-stream; đọc toàn bộ §2; driver/Python gates |
| Android | `crates/android-driver`, helper APK và pinned tools | package/permission/hierarchy -> typed observation/effect | không tap theo toạ độ chưa đo; driver tests, hash/version gates |

Bảng này là ranh giới trách nhiệm hiện có, không khẳng định đã tách hết module lớn.
Khi tách module, giữ public contract và chuyển các test đọc `include_str!` cùng symbol.
Desktop Publish đã tách facade, preflight, execution, Sheet, legacy và tests; facade
giữ tên IPC. Pixel/composer live ở execution, suppression `dead_code` chỉ còn vùng
wrapper legacy. Không suy từ việc tách module này thành đã chia toàn bộ core.

Các contract bổ sung của đợt 06/09: Local API chỉ dùng read deadline cho parse;
device work đã admitted sống theo deadline riêng, response write có giới hạn riêng.
Migration token legacy phải ghi và đọc lại keyring trước khi xoá nguồn SQLite; lỗi
giữ source và báo runtime chưa chạy. Admission gate kiểm AST hàm thật với guard được
giữ trong scope. Quét nguồn và preflight dùng chung `spawn_blocking` hai slot, worker giữ
slot đến khi xong kể cả caller cancel. Các thay đổi này phải đi cùng regressions,
không dùng timeout HTTP ngoài để huỷ effect giữa chừng.

`OperationSourceRef` giữ operation/kind/source/item/máy để Tác vụ mở đúng lịch sử;
navigation không được dispatch/retry. Libraries dùng semantic target rõ ràng, restore
monitor theo exact batch ID; retry hợp lệ theo ID mới chứ không ngầm mở latest batch.

UiAutomator2 10.6.2 dừng cả instrumentation khi nhận `DELETE /session`, nên phục hồi
cây accessibility trong `AgentClient::recreate_session` phải dùng `POST /session`
thay singleton, rồi prime settings trước một lần đọc lại. Chỉ lỗi root accessibility
chính xác trên route đọc được phép đi nhánh này; độ trễ 10 giây chỉ được ghi log,
không phải điều kiện recycle. `close()` vẫn gửi DELETE khi kết thúc thật. Không replay
tap/type/Post khi phục hồi. Log điện thoại và regression ở AGENTS.md §9.201.

Foreground Android dùng chung `AdbProgram::foreground_package` cho session và resolver
hai TikTok: thử `dumpsys window`, `windows`, rồi `displays`. Không dùng riêng `displays`
vì Galaxy S8 Android 9 có thể không trả focus ở dạng đó. Resolver chỉ nhận package
foreground thuộc tập đã đọc từ Package Manager; launcher, system window hoặc lỗi đọc
không được biến thành chọn package đầu tiên. Preflight UI trình bày issue theo máy và
phiên bản; `canExecute` vẫn do backend quyết định. Chỉ thêm tuple composer/nhạc khi đã
đo đủ bước chọn nhiều ảnh, nhạc và readback; một ảnh XML picker chưa chứng nhận Post.

`ComposerPlan::missing_for_carousel` kiểm đủ opening/tail/selection cho preflight
ảnh; `can_publish_carousel` là guard trước effect của pipeline ảnh có nhạc. Bảng
`MEASURED_SOUND_PICKERS` sở hữu layout/marker/back/viewport riêng từng tuple, không
suy hành vi từ ID entry. Global46.0.41 có fixture đọc thật; không alias sang bản khác.
Tabbed sound snapshot chờ tab có bounds ổn định và selected đọc được; tối đa hai
tap trong 60 giây, pool dùng 30 giây để đủ hai lượt source chậm. Không quay về
cửa sổ 8 giây trước khi đo chi phí hierarchy: §9.187 tái hiện 0-tap timeout khi
mỗi source tốn 11 giây. Hàng nhạc vẫn chỉ chọn một lần và xác nhận exact title.
§9.189 đo thêm45.4.3/46.1.3/46.4.3;45.4.3 có albumzx7 và chipzy1. Globalvideo
dùngcorner ordinal+Next(1) giống control đã đo, Trillgiữ đườngvideo cũ. 46.2.1
nhạc đãđo lại là TabbedSnapshot/inline nx3, Backbot; bỏ giảđịnh AutomaticClose.
AccountGlobal trausernameID theo exactversion, vẫn buộcEdit/Profilemenu/unique
handle cùngsnapshot và đọc hai lần. Chỉfixtuređãsanitize được allowlist tên.
Nuôi merge event nhận sau khi bắt đầu snapshot theo từng máy. Detail Publish tự
refresh bằng API đọc có ticket khi `publishUpdated`, không reconcile trong event handler.

Ba workspace dùng phần trình bày riêng: `nurture/NurtureSessionSetup`,
`interaction/InteractionWorkspaceSetup` và `publish/PublishQuickSetup`; API/state owners
vẫn ở Popup/Page. `AutomationWorkspace` đưa `scopeControl` vào Nuôi/Tươngtác để
không duplicate selector. Native popup giữ đường cũ; page xác nhận trước dispatch.
Các khung chọn máy dùng chung `MachineChoice` và `machine-choice.css`: luôn hai cột,
cuộn toàn danh sách, không phân trang máy. State vẫn thuộc workspace; riêng Đăng bài
checked phản ánh assignments, tick là ghép bài còn trống và bỏ tick là gỡ ghép.
Toolbar máy chứa select phạm vi controlled bằng targetRef, không có selector phụ
dưới lưới. Ba trang dùng Thiết lập/Hẹn giờ/Theo dõi. Từ0.2.21 Nuôi/Tương tác lưu
thiết lập trực tiếp, bỏ UI Hồ sơ. Lịch nhận snapshot cấu hình/target trong transaction
qua automation_schedule_from_settings; lịch cũ giữ definitionrevision. Xem AGENTS.md §9.208.
Đăng bài dùng bản nháp local, bỏ UI hồ sơ và lịch theo hồ sơ. `PublishSheetConnection`
đặt link/kiểm tra ngay trong Thiết lập; Hẹn giờ dùng `PublishSchedulePlanner`.
Theo dõi tách danh sách có bộ lọc với chi tiết một chiến dịch, đóng/đổi lựa chọn hủy
response cũ. Xem AGENTS.md §9.204.
Hẹn giờ nhận `selectedIds/assignments/eligible/active/sourceReady` từ Page; chỉ seed
lần mở đầu khi source đã xong, ưu tiên draft lịch hiện có. Draft version2 giữ key cũ,
commonTime và timeMode từng row; migrate legacy time thành custom giữ ID/requestId.
`publishScheduleAllocation` cho phép phân một phần, giữ cặp cũ; không đổi helper
`fillAssignments` cũ. Pointer hook tính lại đích/roster lúc thả, commit đồng bộ qua
draftRef rồi React render; mọi chỉnh sửa tăng generation để loại preflight cũ.
IPC và scheduler giữ nguyên; cùng giờ hẹn không đổi cơ chế thực thi. Xem AGENTS.md §9.205.
Chọn nhanh dùng cùng allocator: chọn mặc định nguồn/máy chỉ khi tập tương ứng
trống, tôn trọng tập máy explicit kể cả máy đã mất kết nối; không tự mở rộng.
Direct drop mang originId và được chuyển bài đã có máy sang đích trống, group
drop giữ cặp cũ. Storage error riêng khỏi notice; footer focus dựa field/row ID,
callback về Setup focus link Sheet. Xem AGENTS.md §9.206.
Stream budget tăng theo roster scan sau startup trong trần32; explicit thấp xếp
hàng tác vụ và không tự nâng. Capacity wait xảy ra trước session/terminate, hỗ trợ
Dừng của Nuôi và cancel campaign của Publish/Tương tác.
Installer nội bộ0.2.21 mang bootstrap Sheet chung trong resource ignored riêng;
startup lưu pair vàoOSSecretStore và xóa tokenSQL cũ. publish_sheet_prepare tự tạo
header khi tab rỗng, khác publish_sheet_check chỉđọc. Deployment4 đã cập nhật.
Manual Nuôi page truyền durationMinutes thật, comment preflight đọc enabled+prob.
45.7.3 có fixture exacttuplechoảnh/nhạc/Send/account; phiên Send chưađược phát trong
đợtđo. Account đòi uniqueEdit/Profilemenu/username cùngsnapshot rồiđọclặp; giữ
Trillpath riêng. FixtureXML chỉ allowlistfilenameđãreview, dump gốc ởtarget.

Nhật ký Publish lấy `PublishProgress` từ composer và lưu thành `publishStep`
trong `operation_device_events`, theo campaign/assignment/UDID. Số bước và thời
gian được ghi lúc thực thi, giữ qua restart; log không có quyền thay verdict
hoặc ghi ý định Post. `resolve_preflight_target` chụp số máy của toàn fleet để
lịch sử không phụ thuộc số thứ tự của riêng nhóm đã chọn. Xem §9.188.

## Xác minh Publish và kết quả qua restart

Dev §9.209 dùng VerificationQueue tối đa hai observer khác UDID, không chặn chéo
khi một máy đọc chậm. evidence.verificationStatus giữ reasonCode, attempts,
readFailures, checkedAt và nextCheckAt; legacy thiếu nextCheckAt kiểm ngay.
Pending lỗi đọc backoff30/60/120giây, quan sát khác30giây; giữ submittedAt và hạn
review30phút. Hoàn tất observer đánh thức chọn máy tiếp; stop drain task đã admitted.
AccessibilityReadUnavailable chỉ dành read-only sau Android recovery; sound observer
cho một lần đọc bổ sung/phase trong ngân sách và xóa snapshot cũ trước readback.


`Submitted` chỉ chứng minh thao tác Đăng đã qua biên hiệu lực và TikTok trở lại feed.
`Verifying` giữ TikTok/media đang tải; `Succeeded` đòi canonical link, đúng tài khoản
đã đọc trước Post, caption và khoảng thời gian xuất bản sau intent. `effect_intent`
giữ `expectedAccount`/`submittedAt` để phép xác minh sau restart không dựa đồng hồ lúc
retry. Thiếu bằng chứng không được cold-start hoặc bấm Post lại.

Nhãn thời gian own-post Global45.4.3/en dùng `:id/zj1`, đo trên hai máy và bài mới
trong §9.209. Global45.7.3/en dùng `:id/zwj`, đã đối chiếu bốn bài thật;
các đường còn lại giữ `:id/tv_post_time`. Caption đúng và nhãn phút mới có thể
chờ ngay trên bài tối đa65giây tới cửa sổ phân biệt được thời điểm; đọc lại cả
caption/thời gian sau chờ, không nới điều kiện interval-after-submission. Phép đọc phải có đúng một nhãn thời gian
và caption hợp lệ; đổi ID không nới khoảng thời gian hoặc nhận caption của bài cũ.
Hồ sơ Global46.2.1/en dùng chung `:id/cover` cho bài đăng và bản nháp. Khi lấy link,
đọc badge nháp `:id/zq_`, loại cover chứa badge trước giới hạn ba ứng viên. Lỗi đọc
badge phải dừng trước tap cover; mở nháp rồi Back không bảo đảm quay về lưới hồ sơ.

`publish_commands/verification.rs` chạy warm session trong control plane; DB CAS ghi
proof, outbox và snapshot cùng transaction. `verified_cleanup.rs` xử lý riêng media
đã xác minh theo delete policy và importId, giữ khả năng thử lại qua restart.
Worker chuyển bài có submittedAt hợp lệ quá30phút sang Uncertain/needsReview trong
một transaction, kể cả máy offline hoặc đang backoff. Bỏ khỏi hàng tự kiểm tra;
kiểm link chủ động giữ scope LinkAndSheet, không tái phát Post. Một số nháp nhìn
thấy trên hồ sơ chỉ là quan sát, không là bằng chứng assignment đã thành nháp.
Nếu receipt cũ thiếu tài khoản hoặc thời điểm gửi hợp lệ, worker yêu cầu kiểm tra
thủ công với lý do thiếu bằng chứng, không đoán tuổi của bài từ createdAt/updatedAt.
Xem AGENTS.md §9.202.
Scope LinkAndSheet/SheetOnly không được dispatch fresh sibling trong campaign.
Clean-start guard đọc hàng chờ dưới lease để Nuôi/Publish không đóng upload khác;
manual viewing vẫn được, IdleSweep đứng ngoài máy đang chờ.

Sheet hỗ trợ mẫu compact qua [Apps Script](apps-script/README.md): `postedAt` lấy từ
intent, đối tác trải ngang, khóa idempotency nằm trong note ô Link cùng atomic update.
Sheet riêng thêm E:H Máy/Tài khoản TikTok/Trạng thái/Lỗi hoặc ghi chú, đối tác từ I.
Opt-in cấu hình `internalReporting`; worker đọc projection durable, revision theo
assignment+campaign, chỉ gửi hàng thay đổi và đòi ACK đúng phiên bản/revision.
Báo cáo trước Post không đi vào outbox link canonical. Xem AGENTS.md §9.203.
`publish_sheet_check` xác thực URL Google Sheets, đọc CSV có hạn mức và hỏi webhook
bằng `rowKind: check`. Đọc CSV thành công chỉ xác nhận readable; connectionVerified
đòi ACK đúng spreadsheetId/gid. Lưu link sau kiểm tra hợp lệ, giữ nguyên credential
và internalReporting. Handler Apps Script kiểm tra không ghi hàng hoặc sửa header;
bản triển khai cũ chưa hỗ trợ check không được coi là kết nối đã xác minh.
Webhook và token cấu hình theo host; không đóng phiên Google/credential máy phát triển
vào bộ cài. CI chạy `node --test scripts/test_publish_sheet.mjs`.

## Bộ cài Windows

Binaries chẩn đoán cần feature `diagnostics`; checker cần `deployment-check` và được
build/stage riêng trước bundle. Bundle mặc định không mang các exe cũ dưới src/bin.
Mọi verifier kiểm hash/version app/checker, report mới, PATH hệ thống và temp cwd.
Java staged launcher dùng manifest UTF-8 đã hash-pin để chạy đường dẫn có dấu;
Windows tối thiểu10 build18362. NSIS hook kiểm trước cài. Đối số `/D` của NSIS phải
ở cuối và không quote cả khi đường dẫn có khoảng trắng; dùng CreateProcess không shell.
WiX per-user fragment cũng kiểm build18362 trước cài: AppSearch đọc CurrentBuildNumber
từ HKLM64, LaunchCondition so với số nguyên và giữ `Installed` để maintenance/uninstall
không bị chặn. Thiếu hoặc sai định dạng build không được coi là đạt điều kiện.

## Chuẩn bị

Đọc `rust-toolchain.toml`, `Cargo.lock`, `apps/desktop/package-lock.json` và CI đang
theo dõi. Node phải thỏa `apps/desktop/package.json#engines`; Windows cần MSVC/Windows
SDK và WebView2. Sidecar/tool pin nằm trong manifest, không đổi theo bản cài ngẫu nhiên.

Tại `apps/desktop`: `npm ci`, sau đó `npm run tauri:dev` để chạy Tauri với backend.
`npm run dev` chỉ chạy web frontend; không dùng kết quả mock để kết luận thiết bị thật.
Theo yêu cầu người dùng ngày 08/09/2026, mặc định thử thay đổi trên **Tauri dev trước**.
React/CSS cập nhật nóng; thay đổi Rust cần biên dịch lại bản debug. Chỉ tạo release/Setup
khi cần giao bản cài hoặc kiểm tra packaging, không đóng gói sau từng thay đổi giao diện.

Để chạy cùng chế độ Full đang dùng trên máy chính, tại `apps/desktop`:

```powershell
$env:RIVIU_AGENT_MODE = 'full'
Remove-Item Env:RIVIU_MOCK_DEVICES -ErrorAction SilentlyContinue
npm run tauri:dev -- --config src-tauri/tauri.full.conf.json
```

Dev dùng backend/driver thật khi không bật mock, nên thao tác điện thoại, Nuôi, đăng bài
và hẹn giờ có tác dụng thật. Giữ dev đang mở nếu kiểm lịch. Trước khi chạy dev với cùng
fleet/dữ liệu, kết thúc phiên đang chạy và đóng bản cài để tránh hai tiến trình tranh thiết bị.
Lần đầu tạo debug cache hoặc đổi dependency vẫn có thể mất thời gian; không hứa thời gian
khởi động cố định. Khởi chạy dev không tự cho phép thực hiện một tác vụ đăng/nuôi mới.

Skill được theo dõi tại `.claude/skills/run-riviu-managers-phone/SKILL.md`; `.agents`
là bản sao runtime ignored, không phải nơi sửa nguồn chuẩn.

## Cổng theo thay đổi

Frontend, tại `apps/desktop`:

```powershell
npm test
npx tsc -b --pretty false
npm run lint
npm run build
npm run test:e2e
```

Rust, tại gốc repo:

```powershell
cargo fmt --all -- --check
cargo test -p riviu-core --locked
cargo test -p riviu-managers-phone --locked
cargo test -p riviu-android-driver --locked
cargo clippy -p riviu-core --all-targets --locked -- -D warnings
cargo clippy -p riviu-managers-phone --all-targets --locked -- -D warnings
cargo clippy -p riviu-android-driver --all-targets --locked -- -D warnings
```

Tài liệu:

```powershell
python -m unittest scripts.test_build_agents_index scripts.test_check_docs -v
python scripts/build_agents_index.py --check
python scripts/check_docs.py
cargo test -p riviu-managers-phone every_agents_section_citation_resolves --lib --locked
cargo test -p riviu-managers-phone agents_md_stays_a_door --lib --locked
```

Packaging, sidecar và Python: chạy đúng danh sách ở
[Desktop CI/CD](../.github/workflows/desktop-ci-cd.yml); [README](../README.md) giữ lệnh
điểm vào. Kiểm tra lock/version/hash, Python unit, Gate 0, audit và `cargo deny` theo
phạm vi. Không bỏ một cổng vì unit của ngôn ngữ khác đã xanh.

## Quy trình sửa và xác nhận

1. Ghi `git status`, đọc symbol và § liên quan; phân biệt thay đổi của người dùng với phần đang làm.
2. Viết regression thể hiện đúng lỗi. Khi thay contract, chạy baseline và mutant trên bản sao, rồi restore test.
3. Sửa trong module sở hữu; giữ deadline, cancellation, persistence và uncertainty semantics.
4. Chạy focused gate trước, full gate theo blast radius; phân biệt compile/unit/e2e/mock/live/installer.
5. Xem screenshot desktop/laptop, kiểm tra keyboard/focus/contrast/scroll; không chấp nhận snapshot lỗi làm chuẩn.
6. Cập nhật tài liệu chủ đề, nhật ký số mới và index. Đọc lại artifact trước bàn giao.

Không chạy harness song song desktop trên cùng USB. Thao tác công khai/cài app/chuyển
nội dung không thuộc smoke điều hướng read-only. Số lượng test và kết quả live là số
đo của một lần chạy, ghi ở nhật ký có ngày, không chép thành năng lực tuyệt đối.
