# Hướng dẫn phát triển

Stack giữ nguyên: Rust workspace, Tauri 2, React/TypeScript/Vite. `src/api.ts` là biên
IPC frontend. Không thêm một control plane riêng để đi vòng ownership/admission hiện có.

Helper Android được chuẩn bị sau inventory ổn định bằng worker
`android_helper_setup` trong `state`: tối đa hai máy, admission và lease Repair giữ
stream, một lần thử mỗi kết nối. `AndroidDriver::ensure_helper_installed` chỉ cài
gói/đọc lại versionCode và launcher; không mở Activity, đổi IME hay tạo session.
Inventory dùng dữ liệu máy đã đọc khi cùng máy đang giữ khóa chuẩn bị helper; lỗi
scan Android không được coi là disconnect để cấp lại lượt cài. Gói `com.riviu.agent`
có versionCode tối thiểu 5 cho launcher; APK, manifest và NOTICE phải khớp hash.

## Bản đồ trách nhiệm

### Nhận diện GUI và FastAPI

`ui_automation` sở hữu cây bất biến, DTO, resolver và revalidation; `app_automation`
sở hữu adapter/profile TikTok và Settings. Cây chung được dùng cho xác minh bài,
tài khoản, picker và bảng nhạc. Resolver chỉ trả mục tiêu; engine giữ tap/type,
effect ledger và verifier. Các đường mở thư viện, Hồ sơ, Chia sẻ và bình luận
có thể yêu cầu nhận diện dự phòng; các điều kiện nghiệp vụ trước Post/Send vẫn
được kiểm riêng. `controls_for_runtime` cho phiên bản chưa có resource set một
route semantic: album/corner ordinal/Next/caption/Send phải được tìm trong cây
hiện tại. Preflight đánh dấu các bước này Chưa quan sát, không coi là đã nghiệm
thu. `for_build` giữ API chứng nhận strict; `for_runtime` giữ đầy đủ proof tài
khoản/caption/thời gian/link. Nhạc dynamic chỉ chọn layout khi row/title/artist,
tab/viewport và hành vi chọn cùng khớp, rồi giữ effective plan xuyên readback.

FastAPI nằm trong `sidecars/gui-service`, protocol1. Pydantic và Rust cùng đọc
fixture `gui-request.json`. Service không mở ADB hoặc database chiến dịch.
Rust truyền bootstrap qua pipe, kiểm handshake và hash runtime, giữ cổng localhost
và token riêng. Runtime Python đóng gói riêng, không trộn với sidecar iOS.
Migration36 ghi request nhận diện theo run/assignment để giới hạn còn hiệu lực
qua restart. Đầu ra model sai vẫn ghi usage; phản hồi khác epoch/observation
bị loại. `check_gui_boundaries.py` kiểm ranh giới import/effect.

Gói tương thích có schema và revision, chỉ chứa dữ liệu; nhập gói phải kiểm
fixture trước khi chọn revision cho phiên mới. `cargo run -p riviu-core --example
gui_replay` đọc gói từ stdin và chạy fixture mà không mở điện thoại. Trạng thái
nghiệp vụ vẫn do database hiện có sở hữu; rollback profile không rollback ledger.

Python gates: `uv run --project sidecars/gui-service pytest sidecars/gui-service/tests`,
Ruff và ty. `build_gui_service.py` tạo runtime + manifest + overlay Tauri;
`test_gui_service_package.py` kiểm khởi động và đóng pipe cha trên PATH sạch.
Windows NSIS/MSI và WiX fragment cùng nhận overlay `tauri-gui-service.conf.json`.

| Vùng | Chủ sở hữu mã | Đầu vào và đầu ra | Retry, cleanup và cổng |
|---|---|---|---|
| Shell và điều hướng | `apps/desktop/src/App.tsx`, `workspaceDraft.ts`, `deviceSurface.ts` | PageId, bản nháp, target riêng từng workspace -> vùng đang mở, dialog quyết định | stale response không ghi đè; test App/workspaceDraft, e2e pages |
| Thiết bị/preview | `useFleet.ts`, `viewStore.ts`, `DeviceTile.tsx`, `PhoneCanvas.tsx`; Rust `view_hub`, `commands`, drivers | roster/stream generation -> trạng thái và frame | cleanup theo generation/owner; viewProtocol parity, Android driver, stream-remount tests |
| Ownership/admission | Rust `lib.rs`, `state.rs`, `commands/`, core driver traits | actor/target/effect intent -> dispatch hoặc lỗi có kiểu | không duplicate effect; admission/concurrency tests trước khi đổi biên |
| Tương tác | `InteractionPopup.tsx`, core `interaction` | URL hiện tại, revision, prepared content -> campaign/assignment/evidence | uncertain không gửi lại mù; parser, effect và JobsPanel regressions |
| Nuôi | `NurturePopup.tsx`, core `nurture` | phiên, nhịp, effect plan -> quan sát/outcome/cost | budget hữu hạn, credential riêng; nurture tests và readiness UI |
| Đăng bài | `PublishPage.tsx`, core `publish`, desktop `publish_commands/{mod,preflight,execution,sheet,legacy,tests}.rs` | media/target/preflight -> durable projection và outbox | Post/URL/Sheet/cleanup tách scope; publish tests, preflight và restart recovery |
| Flow/điều phối | `components/flow`, `components/orchestration`, core `flow` + `orchestration` | graph/node identity/revision -> execution history | không thay device Flow bằng fleet orchestration; validate/save/archive/import tests |
| App/nội dung | `AppsPage.tsx`, `MaterialPage.tsx`, library ledger | artifact snapshot + targets -> batch/item outcomes | queued cancel; restart running -> uncertain, không auto replay; ledger regressions |
| Lịch sử | `JobsPanel.tsx`, `DataPage.tsx`, aggregate query | source/filter/window -> total + page + hydrated detail | filter trước hydrate; >10.000 nguồn báo thu hẹp; projection/query tests |
| Settings/API | `SettingsPanel.tsx`, `ApiPage.tsx`, settings/local API commands | section draft/credential/listener config -> persisted value + actual listener | readback và stale-response guard; section save tests, Local API tests |
| iOS/WDA | `crates/ios-driver`, `sidecars/pymobiledevice3`, `sidecars/wda` | manifest/auth/session -> capability/transport | giữ thứ tự session-trước-stream; đọc toàn bộ §2; driver/Python gates |
| Android | `crates/android-driver`, helper APK và pinned tools | package/permission/hierarchy -> typed observation/effect | không tap theo toạ độ chưa đo; driver tests, hash/version gates |

**Platform vs mạng xã hội vs flow.** `DevicePlatform` là OS thiết bị (iOS/Android).
`SocialNetwork` (`tiktok` | `instagram` | `threads`, mặc định TikTok) là app mục tiêu — seam
dispatch package/link; Instagram/Threads từ chối rõ, chưa implement. Orchestration fleet
(`OrchestrationDocumentV1`) là đồ thị gọi vào engine Nuôi / Tương tác / Đăng hiện có; nút
“Tạo mẫu 3 chức năng” seed 3 hồ sơ + một điều phối Nuôi→Tương tác→Đăng. Engine vẫn là
source of truth — không thay bằng node Flow V2 tap/swipe.

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
Installer nội bộ nhận resource publish-sheet/connection.json chỉ gồm webhookUrl/token;
resource nằm ngoài tracked source. sheet_bootstrap khôi phục cặp khi chưa có cấu hình,
không ghi link bảng. Migration có marker một lần xóa đúng link mặc định cũ bằng digest;
không đổi target chiến dịch/outbox hoặc link người dùng lưu sau migration. Startup di
chuyển credential đã có của máy vào OSSecretStore và xóa token SQL cũ. `publish_sheet_prepare` tự tạo
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

Dev §9.209 dùng VerificationQueue mọi UDID đang due/Ready (một observer mỗi máy,
không trần số máy), không chặn chéo khi một máy đọc chậm. Cleanup media đã xác minh
chạy mỗi tick worker, không chờ queue rỗng. Candidates ưu tiên `is_due` trước trần 1000.
Nhãn thời gian own-post chấp nhận EN và VI (`N phút trước` / `vừa xong`). evidence.verificationStatus giữ reasonCode, attempts,
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

Lượt mới ghi `verificationContractVersion: 1` và `verificationBuilds` từ preflight.
`PublishSubmissionProof` bắt buộc tại transaction claim: tài khoản, submittedAt,
caption hash, bundle/media và đúng package/version/locale đã duyệt. Helper phải qua
probe clipboard có khôi phục trước composer. `publish_account_reservations` khóa
tài khoản chuẩn hóa xuyên thiết bị; trước Post được nhả khi dừng, sau Post giữ qua
restart tới canonical proof. `publish_post_identities` giữ URL riêng cho từng
assignment ngay cả khi tắt Sheet. Worker mới chỉ tự quan sát/dọn lượt mang marker;
dữ liệu lịch sử vẫn đọc được.

`capture_submission_link` dùng một snapshot cho caption và thời gian; chỉ nhận
caption đầy đủ sau chuẩn hóa. Vùng caption rút gọn phải tự có clickable/enabled,
chỉ bấm một lần rồi đọc lại toàn bộ proof. Link ứng viên chỉ được nhận sau khi
kiểm các ô còn lại trong viewport không có bài thứ hai cùng khớp. Recovery chờ
60 giây/tối đa ba điều hướng; một lượt tìm tối đa 12 ô qua ba viewport, ngân sách
180 giây, không phát thao tác mới khi hết hạn. Primitive đang chạy được hoàn tất
để giữ khôi phục IME. Cuộn dùng container scrollable chứa ô bài từ snapshot;
Copy dùng hitbox cùng cây và tối đa hai sentinel độc lập. Diagnostic lưu tuple,
generation, reason và chi phí; snapshot thành công thuộc đúng bài được nhận.

Nhãn thời gian own-post Global45.4.3/en dùng `:id/zj1`, đo trên hai máy và bài mới
trong §9.209. Global45.7.3/en dùng `:id/zwj`, đã đối chiếu bốn bài thật;
các đường còn lại giữ `:id/tv_post_time`. Caption đúng và nhãn phút mới có thể
chờ ngay trên bài tối đa65giây tới cửa sổ phân biệt được thời điểm; đọc lại cả
caption/thời gian sau chờ, không nới điều kiện interval-after-submission. Phép đọc phải có đúng một nhãn thời gian
và caption hợp lệ; đổi ID không nới khoảng thời gian hoặc nhận caption của bài cũ.
Hồ sơ Global46.2.1/en dùng chung `:id/cover` cho bài đăng và bản nháp. Khi lấy link,
đọc badge nháp `:id/zq_`, loại cover chứa badge trước khi chọn ứng viên. Lỗi đọc
badge phải dừng trước tap cover; mở nháp rồi Back không bảo đảm quay về lưới hồ sơ.

`publish_commands/verification.rs` chạy warm session trong control plane; DB CAS ghi
proof, outbox và snapshot cùng transaction. `verified_cleanup.rs` xử lý riêng media
đã xác minh theo delete policy và importId, giữ khả năng thử lại qua restart.
Worker chuyển bài có submittedAt hợp lệ quá 30 phút (đăng ngay) hoặc 240 phút (hẹn giờ)
sang Uncertain/needsReview trong một transaction, kể cả máy offline hoặc đang backoff. Bỏ khỏi hàng tự kiểm tra;
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
Preflight xác minh `deliveryVersion: 2`, đúng spreadsheetId/gid và chế độ báo cáo,
đưa `sheetDelivery: SheetDeliveryTarget` vào digest và request đã lưu của chiến dịch.
Đổi credential không thay đích đã chốt. Dữ liệu lịch sử thiếu target v2 không được
tự gán đích từ settings hiện tại hoặc đưa lại vào worker mới.

Worker Sheet chạy độc lập observer liên kết, tối đa hai request, trong đó tối đa
một báo cáo tiến độ. Claim 120 giây trong SQLite dùng chung theo assignment cho
gửi chủ động và worker, có token fencing và revision CAS khi hoàn tất. Canonical
outbox giữ identity bất biến; báo cáo có lịch gửi, số lần thử, lỗi và revision riêng
theo assignment+campaign. Lỗi transport retry từ 30 giây, tăng tới 15 phút; lỗi
payload/token/header/đích/ACK tạm dừng tới khi sửa kết nối hoặc thử lại chủ động.
Khởi động lại phục hồi lịch đến hạn; một hàng lỗi không chặn các hàng khỏe.

Canonical chỉ hoàn tất khi ACK khớp deliveryVersion, spreadsheetId/gid,
assignmentId, deliveryRevision và postUrl đã gửi. Báo cáo nội bộ đòi reportVersion
và rowRevision; một revision mới hơn phải trả Link thực tế đang lưu, không echo
Link trống của request cũ. ACK thành công cùng settlement cập nhật outbox và
projection trong transaction; lỗi DB sau remote commit giữ khả năng gửi lại cùng
identity. Báo cáo trước Post không đi vào outbox canonical.
Apps Script kiểm tra toàn bộ payload và row trước mutation, rồi mở rộng grid,
header, giá trị và note trong một Sheets batchUpdate. ACK đọc lại dữ liệu đã commit.
Link canonical bất biến, ghi chú chống trùng và cột riêng sau đối tác được bảo toàn.
`publish_sheet_check` xác thực URL Google Sheets, đọc CSV có hạn mức và hỏi webhook
bằng `rowKind: check`. Đọc CSV thành công chỉ xác nhận readable; connectionVerified
đòi ACK đúng spreadsheetId/gid và capability deliveryVersion2. Mẫu legacy chỉ
quảng bá phiên bản 1 cho client cũ. Lưu link sau kiểm tra hợp lệ, giữ nguyên credential
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
python -m unittest scripts.test_check_docs -v
python scripts/check_docs.py
```

Packaging, sidecar và Python: chạy đúng danh sách ở
[Desktop CI/CD](../.github/workflows/desktop-ci-cd.yml); [README](../README.md) giữ lệnh
điểm vào. Kiểm tra lock/version/hash, Python unit, Gate 0, audit và `cargo deny` theo
phạm vi. Không bỏ một cổng vì unit của ngôn ngữ khác đã xanh.

## Quy trình sửa và xác nhận

1. Ghi `git status`, đọc symbol liên quan; phân biệt thay đổi của người dùng với phần đang làm.
2. Viết regression thể hiện đúng lỗi. Khi thay contract, chạy baseline và mutant trên bản sao, rồi restore test.
3. Sửa trong module sở hữu; giữ deadline, cancellation, persistence và uncertainty semantics.
4. Chạy focused gate trước, full gate theo blast radius; phân biệt compile/unit/e2e/mock/live/installer.
5. Xem screenshot desktop/laptop, kiểm tra keyboard/focus/contrast/scroll; không chấp nhận snapshot lỗi làm chuẩn.
6. Cập nhật hướng dẫn sản phẩm hoặc README khi hành vi người dùng đổi. Đọc lại artifact trước bàn giao.

Không chạy harness song song desktop trên cùng USB. Thao tác công khai/cài app/chuyển
nội dung không thuộc smoke điều hướng read-only. Số lượng test và kết quả live là số
đo của một lần chạy, không chép thành năng lực tuyệt đối.

## Hội thoại theo phiên

`ThreadCampaignRequest.scriptedConversation` giữ schema1, targetScripts, roleBindings,
seed, durationMinutes và tùy chọn startsAt/endsAt. Planner dùng ordinal riêng 0–63 mỗi
bài; parent là câu trước cùng topic. Automation template bảo toàn trường này. Nội dung
AI trả về draft có cấu trúc; executor chỉ dùng câu đã duyệt. Preview ràng buộc cả script.

Migration35 thêm interaction_conversation_sessions/turns. Coordinator có owner token,
giờ kết thúc bất biến và cursor link; mỗi lượt gọi lại đường gửi có assignment scope
của engine hiện có. Chờ không giữ device lease. Deadline được kiểm trong transaction
begin_interaction_comment_action_effect. Sau restart owner bị thu hồi, effect armed
vẫn uncertain; resume không reset endsAt. P90 được tính từ thời gian turn thực thi,
không từ updated_at chứa thời gian đợi. Reply strict đọc parent bằng một snapshot,
mở replies của root và dùng picker mention cho cả root/reply; literal không qua Send.
