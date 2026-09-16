# Hướng dẫn phát triển

Stack giữ nguyên: Rust workspace, Tauri 2, React/TypeScript/Vite. `src/api.ts` là biên
IPC frontend. Không thêm một control plane riêng để đi vòng ownership/admission hiện có.

Google Sheets trực tiếp dùng OAuth Desktop PKCE S256 và callback loopback có
state, timeout và hủy; đăng nhập/Picker mở trình duyệt ngoài. Scope `drive.file`
được cấp theo file do Picker chọn. Refresh token và client config dùng SecretStore
hiện có; IPC chỉ trả identity/trạng thái, không trả token. Mỗi tab có writer UUID
và reportingEpoch trong developer metadata; ghi updateCells cùng receipt note rồi
đọc lại. Không dùng append không khóa để xử lý retry timeout. API không có CAS cho
sửa tay; app từ chối khi giá trị/công thức/receipt trước ghi đã thay đổi.

Cấu hình ứng dụng Google bắt buộc đi kèm binary release. `build.rs` đọc biến
`RIVIU_GOOGLE_OAUTH_CONFIG_JSON` (CI đọc GitHub Actions secret cùng tên tại cả bước
build deployment checker và đóng gói app). Build tại máy phát triển có thể dùng
`apps/desktop/src-tauri/google-oauth.local.json` đã gitignore, hoặc đường dẫn qua
`RIVIU_GOOGLE_OAUTH_CONFIG_FILE`. Biến JSON có ưu tiên cao nhất, kể cả khi rỗng.
JSON gồm
`clientId`, `clientSecret` (tùy chọn), `pickerApiKey`, `projectNumber`; client ID
phải thuộc OAuth Desktop. Không đưa access token, refresh token hay tài khoản vào
JSON này; các trường lạ bị từ chối. Đây là cấu hình phân phối trong binary, không
phải nơi giữ bí mật server. Cấu hình đã lưu trên PC luôn được ưu tiên; trước đăng
nhập app ghim cấu hình vào SecretStore để nâng cấp binary không đổi client của
phiên cũ. Build release dừng nếu cấu hình thiếu, sai hoặc chưa đủ Picker; debug
không cấu hình vẫn dùng được để phát triển phần khác. Đặt
`RIVIU_REQUIRE_GOOGLE_CONFIG=1` để kiểm tra cùng điều kiện trong bản dev.
Cấu hình được ghi vào OUT_DIR rồi nhúng vào binary, không in giá trị ra log.
Không đưa file cấu hình local vào source, log hay gói chứng cứ.
Chép `.exe` đã build đủ cấu hình không cần chép SecretStore; máy đích tự đăng
nhập tài khoản Google của mình. Build không tự đọc credential của người dùng.
Kiểm tra OAuth/Picker thật cần người vận hành tự đăng nhập.

Chuyển Apps Script sang direct ghi intent local trước, dừng nhận claim, drain,
gọi retirement dưới ScriptLock, rồi nhận writer trên tab và commit provider. Lượt
cũ giữ publicationId và đích/epoch; logout giữ provider để không fallback sang
webhook. Login mới thay authorization generation để mở lại nghĩa vụ ghi bị dừng
do hết quyền. API request chung trần hai slot và nhịp tối đa một request/giây;
delivery có deadline 90 giây, đọc theo trang giới hạn 16.000 ô. Backup/reset giữ
gid0, sao lưu toàn workbook, dừng epoch cũ và xác minh lại trước khi mở epoch mới.

Bình luận Android đọc tài khoản trong phiên điều khiển trước khi mở bài đích;
chỉ cập nhật trường handle, giữ các metadata khác. Reply giải username người được
tag tại thời điểm gửi và giữ identity của câu gốc để mở nhánh khi đọc lại.
Kịch bản dùng `device_meta.handle` và snapshot `roleBindings` trong request. Preview
trả `conversationAccounts`; tạo campaign kiểm cùng bộ validator trong transaction
SQLite, áp dụng cả lịch và Điều phối. Trước câu và tại gate Send kiểm lại metadata
của người gửi và các vai được tag; không sửa snapshot lịch sử.
Worker xác minh dùng chung `open_exact_target_by_hierarchy`, không mở URL lần nữa
trước resolver. Trong cửa sổ 30 giây, chỉ mở lại cùng link một lần nếu đọc được
Home/Profile và thẻ LIVE sau ít nhất 10 giây mà chưa có nút bình luận. Không mở
lại với sai package, bài báo không tồn tại hoặc lỗi clipboard. Android dựng lại
navigation task bằng NEW_TASK|CLEAR_TASK trong nhánh này để thoát SplashActivity
giữ intent cũ; không xóa dữ liệu ứng dụng. Một lượt quan sát tối đa 75 giây trong
ngân sách 120 giây đã lưu; lease 150 giây gồm mở phiên và cleanup. Link sao chép
phải khớp trước thao tác công khai. Lượt gửi chưa xác minh không được gửi lại.

Inspector dùng `ui_automation::inspector::ElementSelector` và `inspector_commands`.
UI và `scripts/riviu_agent_mcp.mjs` gọi cùng observe/tap/record qua Tauri hoặc
`POST /v1/inspector/{observe,tap,record,recording}`. MCP dùng URL loopback và token
API hiện có, không mở ADB server hoặc phiên driver thứ hai. Mỗi thao tác giải lại
selector duy nhất theo package; recorder ghi intent trước tap, lưu ảnh/cây trước
và sau. Bước chưa có phần tử kết quả mới giữ unverified và không xuất Flow tự chạy.
Flow tap dạng `selector` có hậu điều kiện `elementVisible`, được kiểm cả compiler,
runtime và ledger. Ledger nhận baseline `none` chỉ khi cấu hình là tap phần tử và
hậu điều kiện là `elementVisible` trong cùng package; tap theo ảnh vẫn giữ baseline
ảnh/generation. Cả kết quả đạt và không đạt đều phải khớp selector đã biên dịch.
ElementVisible chỉ chứng minh UI; không thay proof nghiệp vụ
Đăng/Gửi, canonical link hoặc tài khoản. Dữ liệu quan sát nằm trong artifacts/inspector.

Xác minh bài ảnh đọc caption trong viewer, lấy link bằng clipboard sentinel,
đối chiếu toàn bộ caption, tài khoản và content ID với metadata oEmbed công khai.
Nếu viewer đã ẩn caption, chỉ mở Share trên màn bài có nút Back và Comments đã
nhận diện; metadata công khai vẫn phải khớp toàn bộ nội dung. ID bài hỗ trợ đối chiếu
phiên: TikTok có thể cấp ID trước nút Đăng; mốc dưới lấy từ sự kiện `opening_app`
đã lưu của đúng campaign/máy, tối đa 30 phút trước `submittedAt`. Thiếu mốc ấy thì
chỉ dùng `submittedAt`, không tự nới khoảng thời gian. Caption, tác giả và content ID
phải cùng khớp; lỗi mạng giữ trạng thái chờ xác minh. Riêng timestamp suy ra từ ID
không đủ chứng minh xuất bản.

`Database::publish_device_guard` phân biệt upload cần bảo vệ và khoản thiếu link của
bài cũ. Chỉ bỏ giữ máy khi receipt có `state=posted`/`verdict=Posted`, xác minh đã
`needsReview`, không có pipeline, và `updated_at` đã qua ngân sách dài nhất hiện có
(240 phút). Với receipt Submitted cũ, lần kiểm tra chủ động được ghi thêm quan sát
hồ sơ của đúng tài khoản, hash nguồn và hash intent nếu đã qua 4 giờ từ submittedAt,
không có pipeline đang chạy. Quan sát có hiệu lực 24 giờ và chỉ giải phóng máy;
trạng thái xuất bản, outbox và quyền gửi lại không đổi. Intent thay đổi làm mất
hiệu lực quan sát; tuổi campaign riêng lẻ không đủ để bỏ giữ máy.
Preflight hiển thị khoản thiếu link riêng; clean-start và cleanup dùng cùng quyết định.
`adb_server::discover` chỉ đọc `host:devices-l` trên loopback 5037/5038, không gọi
kill-server. Ở chế độ tự nhận, inventory gộp hai server và giữ bảng serial → cổng
chung cho các bản clone của `AdbProgram`; lifecycle, shell, forward, instrumentation,
scrcpy/minicap đều định tuyến theo serial. Máy trùng ở hai server giữ cổng đang dùng;
máy mất kết nối vẫn giữ route phục vụ cleanup. Server đang quản lý máy mà lỗi đọc
inventory không được biến thành danh sách rỗng. Cổng cấu hình tường minh chỉ dùng
server đã chọn; không tự mở rộng phạm vi đó.

My Apps dùng `AppWorkflowV1` (schema1) và revision bất biến; migration40 bổ sung bảng
document/revision. Graph được kiểm ở biên lưu rồi biên dịch thành cấu hình native và
điều phối hiện có. `workflowActionOrder` điều khiển thứ tự Like/Save/Comment trong
cả vòng Nuôi pixel và hierarchy. Chuỗi chuẩn bị/effect/xác minh của hai engine còn lại
giữ phụ thuộc hiện có; không coi việc vẽ node là bằng chứng đã tách từng stage executor.
Chờ/log ngoài pipeline thành node điều phối thật, graph cycle bị từ chối.

Migration39 bổ sung tài khoản, kết nối và tác vụ có revision; ghi CAS trong transaction,
import nguyên tử, credential chỉ là tham chiếu OS store. `set_http_proxy` là lệnh typed
qua control plane và Android driver, đọc lại `http_proxy` trước khi báo xác nhận.
Local API `/v1/apps`, `/v1/apps/{id}` và `/v1/apps/{id}/runs` gọi chung command với UI.

Cửa sổ điện thoại là non-modal và chỉ có một instance. `useDeviceWindows` thay UDID
khi đổi máy; key React giữ ổn định. Handoff chờ end của máy cũ trước begin của máy mới;
registry theo UDID giữ refcount cho điều khiển nhóm. Bộ test giữ ca begin/end chồng
nhau, đổi máy khi phiên chưa đóng, và chọn trang khi máy đang mở. Bảng tệp dùng portal
ngoài stage transform để giữ đúng modal/focus và không bị menu cuộn cắt nội dung.
Phiên Đồng bộ thuộc `App.tsx`: snapshot gồm máy chính và target bất biến, không lưu qua
restart. `FocusStream` duy nhất của máy chính mở toàn bộ control session, báo
preparing/active/degraded và chặn input khi chưa active. Mọi `group_input` từ cửa sổ
này mang `masterUdid`; backend khử trùng, đặt master đầu tiên, áp policy chỉ cho follower
và poll fan-out đồng thời. Đổi selection/master/roster tắt phiên; retry chỉ mở session,
không replay input không idempotent.

Ba workspace cũ vẫn là đích mặc định; graph chỉ mở qua Thêm Flow. Trạng thái ẩn từng
nhóm sidebar và bảng Hiển thị là preference cục bộ, không đổi cấu hình chiến dịch.
Rail lưu `riviu.control.displayPinned`; hover dùng overlay không đổi chiều rộng lưới,
delay rời 240 ms và giữ khi focus bàn phím ở trong. Nhóm dùng grid transition và inert.
`DeviceFilesPopup` có mode browse/upload/download; upload nhận danh sách từ picker PC,
ghi theo từng tệp và giữ danh sách hoàn tất để retry không gửi lại chúng. Bảng tồn tại
thêm 180 ms lúc đóng qua `useClosingTransition`, hỗ trợ reduced motion và trả focus.
Dropdown dùng stylesheet chung `operator-polish.css`, giữ select/option native và
thêm `appearance: base-select` trong `@supports`; WebView2 152 đã xác minh picker,
Escape và chọn option. WebView cũ giữ select native với khung theo token Riviu.

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

VerificationQueue dùng giới hạn máy chủ, mặc định4 observer, một observer mỗi máy.
Cleanup media chạy worker riêng; một máy đọc chậm không chặn dispatcher hoặc Sheet.
Candidates được phân trang hữu hạn, ưu tiên bài tới hạn và luân phiên giữa các máy.
Nhãn thời gian own-post chấp nhận EN và VI (`N phút trước` / `vừa xong`).
`evidence.verificationStatus` giữ reasonCode, attempts, readFailures, checkedAt và
nextCheckAt. Mọi bài đã gửi còn thiếu link được kiểm sau300giây tính từ cuối lượt
trước; không đặt tổng hạn chờ. Stop chờ task đã được cấp quyền hoàn tất.
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
Ô cover được cắt theo mép trên của thanh Home/Profile đang quan sát, kể cả khi
accessibility vẫn đánh dấu ô phía dưới là visible. Cả điểm chạm và điểm vuốt phân
trang dùng phần còn hiển thị đó. Bài có banner TikTok báo đã bị gỡ được bỏ qua trước
khi mở Share; không dùng nút chia sẻ của bài đã bị gỡ để tìm link bài khác.

`publish_commands/verification.rs` chạy warm session trong control plane; DB CAS ghi
proof, outbox và snapshot cùng transaction. `verified_cleanup.rs` xử lý riêng media
đã xác minh theo delete policy và importId, giữ khả năng thử lại qua restart.
Worker lưu `nextCheckAt` bằng thời điểm kết thúc quan sát cộng 300 giây cho mọi bài
đã gửi còn thiếu link, kể cả lỗi đọc. Không đặt tổng hạn chờ liên kết; mỗi lần quan
sát vẫn có deadline và lease riêng. Restart giữ nguyên mốc gửi và lần kiểm tiếp.
Android ở trạng thái Connected sau restart vẫn được kiểm khi agent đã sẵn sàng;
worker mở warm session qua control plane, không đòi mở điều khiển bằng tay để đổi
trạng thái thành Ready. Máy Busy/Preparing/Error/offline vẫn chờ; iOS giữ điều kiện Ready.
Review cũ có cause `verificationDeadline` và ngân sách 30/240 phút được tiếp tục
bằng CAS khi effect intent còn đủ tài khoản/thời điểm gửi; review vì lý do khác giữ
nguyên, kể cả sau một lần kiểm chủ động thất bại. Đổi trạng thái không tái phát Post
hoặc tạo outbox trước khi có proof. Kiểm link chủ động giữ scope LinkAndSheet. Một số nháp nhìn
thấy trên hồ sơ chỉ là quan sát, không là bằng chứng assignment đã thành nháp.
Photo viewer đã khớp toàn caption trả lỗi sao chép có kiểu; verifier kết thúc lượt
với nguyên nhân đó, không để navigation sau ghi đè. OCR cục bộ kiểm vùng phía trên
ảnh chụp trước/sau Copy, chỉ nhận câu xử lý chính xác với confidence >= 0.9, không
có ở ảnh trước và đúng binding session/hash. Mã `tiktokProcessing` chỉ là quan sát
chờ, không tạo link hoặc thay thế proof tài khoản/caption/content ID. Clipboard
có link thật vẫn được ưu tiên và đi qua verifier như cũ.
Trill 38.3.2 khi header bị cuộn mất dùng bảng Switch account đã đo để đọc hàng
`selected=true`: description hàng và text con phải trùng qua hai snapshot mới.
Chỉ mở và đóng bảng, không bấm hàng tài khoản; đóng xong phải về hồ sơ. Nhiều hàng
được chọn, sai package hoặc text lệch đều không tạo bằng chứng tài khoản.
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

### Flow: dữ liệu, nhập kịch bản và API tác vụ

Catalog có thêm `SetVariable`, `ReadText`, `IfValue`, `IfVisible`, `Log` theo schema
Flow V2. Biến hiện có kiểu chuỗi, tên ASCII tối đa64byte, nội dung tối đa4096ký tự.
Compiler yêu cầu biến đã được ghi trên mọi đường vào của bước đọc. Gán biến giữ
nguyên chuỗi literal; connector nội suy tham chiếu có kiểm tra, không thực thi biểu
thức. Writer ghi output vào transaction Succeeded
của attempt. Consumer dựng lại biến từ các predecessor đã thành công trên đúng
đường đi, device-run và revision; không có dictionary chung xuyên thiết bị.
Database kiểm output theo loại writer khi ghi và khi đọc lịch sử. Hai nhánh
`matched`/`notMatched` dùng chung successor/admission với IfVision.

`flow::connectors` thực thi FileRead/FileWrite/HttpRequest/SheetRead/SheetWrite;
UI cấu hình nằm trong FlowInspector. `validate_template` kiểm tên biến và mẫu,
executor nội suy `${name}` bằng output predecessor đã thành công rồi `validate`
kiểm lại toàn bộ cấu hình trước dispatch; không đánh giá mã. Payload/output giới
hạn 4.096 ký tự, I/O 16 KiB. Tệp bị giới hạn trong `Database::flow_connector_root()`
(`flow-data` cạnh DB), chặn đường dẫn thoát, symlink/reparse và tên hệ thống; khóa
file ngăn các máy cùng ghi, thay tệp nguyên tử rồi đọc lại/băm SHA-256. Đoạn I/O
cục bộ hữu hạn không tạo worker có thể sống sau khi node hủy. HTTP dùng HTTPS
(HTTP chỉ localhost), Bearer từ OS secret store, timeout tối đa 30 giây, không
redirect hoặc retry; credential echo không đi vào ledger. FileWrite, HTTP và
SheetWrite ghi intent trước effect; mất phản hồi giữ Uncertain, không tự phát lại.
Connector ghi giá trị và receipt vào cùng ledger với các node khác.

Sheet dùng cấu hình publish hiện có, không thêm scheduler/outbox. Apps Script
`flowRead`/`flowWrite` protocol1 buộc đúng spreadsheet ID/tab/vùng A1 và token,
giới hạn 1.000 ô; ghi bằng `stringValue` rồi xác minh toàn vùng dưới script lock.
Client gửi một POST, chỉ chấp nhận content redirect GET của Google không mang
credential. Cần triển khai lại script đi kèm trước khi dùng endpoint này; nâng
ứng dụng không tự sửa bản triển khai hay nội dung bảng. `flow_connector_info`
chỉ trả đường dẫn/tên tham chiếu; lệnh lưu token và nhập tệp giữ admission hiện có.

`IfVisible` đọc một cây hoàn chỉnh và phân biệt thiếu bằng chứng với không thấy
phần tử; cần capability hierarchy thực tế. iOS chưa có primitive cây đầy đủ nên
node này báo thiếu capability. `ReadText` dùng request deadline trong driver;
Android không chọn chuỗi rỗng khi response text sai kiểu.

Importer ngoài chuyển dữ liệu thành bản nháp, giữ diagnostics/ID nguồn. Những
node chưa có ánh xạ tương đương chặn chuyển đổi, không bị bỏ âm thầm. Macro cũ
không có profile hình học không được gán một profile giả khi nhập tọa độ.
Repeat mở rộng body tuyến tính thành các node riêng với ID mới (1..50 lượt,
tối đa500node), có Undo nguyên tử. Runtime vẫn chạy DAG và mỗi copy có attempt
độc lập; Flow cũ không đổi schema hoặc semantics khi phục hồi.

`Subflow` và `Repeat` native giữ snapshot Flow V2 trong config, kèm ánh xạ biến
inputs/outputs. Compiler mở rộng tối đa8cấp,50lượt và2.000node; UUID, tên biến và
đường đi nguồn được tạo xác định theo scope. Start/End của Flow con thành Join,
CopyVariable nối biến ở biên; mọi nhánh trong body vẫn dùng cổng matched/notMatched.
Compiled plan ghim sourcePaths/revision và actionDefinitionVersions; FlowRunDetail
trả đường đi này để monitor hiển thị lượt lặp. Các bước đã hoàn thành được đọc từ
ledger, không chạy lại body để dựng biến sau restart. `Transform` hỗ trợ xử lý
chuỗi/dòng, JSON pointer và regex có ngân sách bộ nhớ, đầu ra tối đa4.096ký tự.

Local API task routes gọi cùng Flow command handler của UI; không tạo scheduler
riêng. POST chạy Flow không tự retry khi mất response. API status/cancel luôn dùng
run ID trả về; contract/schema ở trang API và module `local_api/tasks.rs`.

GUI Service có endpoint template-match cục bộ dùng OpenCV/NumPy; thao tác xem thử
trong FlowVisionCapture không chạm điện thoại. Snapshot, crop, hash, epoch và
generation được ràng buộc hai phía. Tối đa2worker native; client timeout vẫn giữ
slot đến khi công việc native kết thúc. Các tùy chọn thử đa tỷ lệ chỉ áp dụng phép
thử này; Flow TapVision/IfVision vẫn dùng matcher Rust và config đã lưu.

`OcrReadText` lấy frame từ generation đang sở hữu rồi gọi OCR cục bộ qua reasoner
được tiêm vào FlowRuntime. Screenshot/hash/epoch được kiểm hai phía. Tesseract và
model vie/eng được ghim SHA-256 và đóng gói; máy chạy không tải model ngầm.
Output OCR giữ văn bản, engine, confidence, bounds và binding trong ledger; lỗi
đọc/mất generation không thành văn bản rỗng giả. Worker OCR chạy process riêng,
timeout/hủy thu hồi process, dùng chung trần2slot với các phép nhận diện khác.

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

Đọc lại composer Android dùng một hierarchy snapshot và chỉ chọn EditText đang có
focus, enabled, hiển thị và không phải hint. Hai ô trùng resource-id (thanh thu gọn
và ô soạn thật) không được chọn theo thứ tự. Sau chọn tag, chỉ đọc lại tối đa 4 giây,
cách nhau 400 ms; lỗi transport giữ nguyên nguyên nhân, hủy/hết giờ ngắt chờ.
Tag phải khớp nguyên username và nội dung gốc phải còn trước khi đi qua Send gate.
Mở nhánh reply xác minh nội dung/tác giả gốc, giới hạn nút mở trước nội dung bình luận
kế tiếp và tìm vùng cha clickable của nhãn. Nhãn `View N replies` có thể không
clickable; nhiều bình luận cùng tác giả không thay thế bằng chứng nội dung gốc.

## Xác minh bình luận Android

Migration37 thêm `interaction_comment_verification` và lịch quan sát lại parent.
Trước khi nâng từ schema36, DB được sao lưu bằng SQLite `VACUUM INTO` sang file
`.pre-comment-verification-v36.db`. Công việc xác minh được chuẩn bị trước Send;
thời điểm/hạn đọc lại được kích hoạt trong transaction ghi intent. Android sau Send
giữ `uncertain` đến khi worker quan sát bài, nội dung, tác giả và nhánh; các lượt
`uncertain` không trở lại đường gửi. Hàng đợi giữ attempts, lease/revision qua restart.

Worker tối đa hai thiết bị; ba lần đọc tại các mốc 5/20/60 giây, mỗi lần tối đa
45 giây quan sát và hết ngân sách sau 120 giây từ Send. Thời gian bootstrap/cleanup
dùng giới hạn của control plane. Chờ không giữ thiết bị. Shutdown dừng và thu hồi
worker trước khi dọn thiết bị. Hết giờ chiến dịch chỉ cho hoàn tất đọc lại, không gửi.
`interaction_verify_comment` là lệnh xếp quan sát lại có giới hạn; gọi lặp khi pending
không đặt lại ngân sách. Lệnh Tim/Lưu `interaction_readback` giữ hợp đồng riêng.

Tìm parent và nội dung dùng snapshot chung, mở vùng `View folded comments` hoặc
`Community-flagged comments` qua hitbox cha đã đo. Không thấy parent được quan sát
lại tối đa hai lần trong giờ chạy; link khác tiếp tục. Lịch sử thiếu identity chỉ
hiện cần kiểm tra; người vận hành chủ động yêu cầu đọc lại, không tự phát lại lịch sử.


Điều phối Đăng bài dùng `publish_dispatch_jobs` trong SQLite (migration41), cùng
claim theo thiết bị và giới hạn giai đoạn. Worker chỉ tạo task sau admission;
media được tải theo bài đã nhận, hàng chờ chỉ giữ ID. `publish_attempts` giữ lịch
sử lần thử; `publication_id` được backfill bằng assignment ID và bất biến, không
nhóm lại dữ liệu cũ theo caption, thư mục hoặc tên máy. Intent và revision của
pipeline vẫn kiểm tại nút Đăng. Worker đã lỗi sau effect chỉ chuyển sang xác minh.

`publish_get_limits`/`publish_set_limits` nhận `{transfer,compose,verify,deviceTotal}`,
lưu ở `publish.dispatch.limits` theo database máy chủ, mỗi giá trị từ 1 đến 64.
Giảm giới hạn chặn cấp lượt mới đến khi số đang chạy xuống mức mới; không cắt request
thiết bị đang thực hiện. Worker đọc lại giới hạn verify ở vòng điều phối tiếp theo; phiên đang chạy được hoàn tất.
Lịch quá 30 giây được CAS sang missed khi chưa bắt đầu; shutdown giữ hàng chưa chạy
và restart không tự phát lại effect đã có intent. Trước migration40→41 tạo và đọc
lại bản sao SQLite `pre-publication-v40.db`.

`publish_sheet_reset_reporting` nhận UUID `resetId`, dừng cấp claim Sheet mới và
đợi claim đang chạy kết thúc. Apps Script backup/readback workbook, đóng epoch cũ
rồi xóa userEnteredValue/note dưới header của gid0; giữ định dạng/header/tab khác.
Reset bị mất phản hồi phải tiếp tục bằng cùng ID. `reportingEpoch` nằm trong target
đã chốt, payload, note và ACK; ACK sai epoch/publication không settle. Local reset
đánh dấu nghĩa vụ cũ superseded, không sửa thành sent và không xóa post evidence.
URL, credential, bản sao DB/Sheet và media nghiệm thu chỉ thuộc dữ liệu vận hành.


Picker Trill 38.3.2 có thể cắt hàng ordinal đã chọn ở mép trên khi tự cuộn.
`visible_selection` chỉ bỏ phần prefix này khi ordinal liên tiếp, x/width và mép
đáy khớp grid gốc qua shift của một ordinal đã chọn còn hiển thị đầy đủ. Không
bỏ ô chưa chọn hoặc dùng tọa độ extrapolate để tap. Test giữ ca chọn 12→13 ảnh.
Chế độ dev `RIVIU_PUBLISH_PICKER_TRACE` nhận đường dẫn tuyệt đối và chỉ giữ một
XML cuối cho mỗi album; bản phát hành không ghi trace này.


Android mở ứng dụng đọc lại foreground. Nếu launcher chỉ đưa một task chứa ứng dụng
chia sẻ ngoài lên trước, driver giải launcher component của đúng package rồi mở
với NEW_TASK|SINGLE_TOP|CLEAR_TOP và đọc lại. Đường này giữ tiến trình TikTok, không
force-stop hoặc CLEAR_TASK; lỗi không xác nhận foreground dừng trước thao tác UI.
Ca SMS recipient picker nằm trên TikTok46.2.42 giữ nguyên PID khi phục hồi.


Migration42 lưu receipt tạo chiến dịch theo requestId và fingerprint đầu vào bất
biến; UI giữ UUID khi mất ACK, backend trả chiến dịch cũ trước khi chạy preflight
lại. Đổi nội dung với cùng requestId bị từ chối; nhấn tạo lượt mới dùng UUID mới.
Bộ điều phối cấp claim theo transaction và giữ quyền giữa hai stage cùng máy,
tránh danh sách candidate cũ nhận hai bài. Công việc paused không bị kết thúc khi
shutdown; restart cấp token mới và đối soát missed tới cấp chiến dịch.
Reset Sheet gián đoạn giữ `reportingReady=false`; check đọc vẫn hoạt động để tiếp
tục cùng resetId nhưng preflight mới và các đường ghi bị chặn tới khi clear/readback
xong. Mọi HTTP Sheet (kể cả check/Flow/CSV) dùng chung semaphore2.


Xác minh video chỉ lấy Copy từ viewer có nhãn Video, đúng package và caption
khớp đầy đủ hoặc prefix rút gọn đã đo. Prefix không xác minh xuất bản: link phải
qua metadata công khai khớp toàn caption/tác giả/content ID và cửa sổ gửi đã lưu.
Khay bình luận Trill38.3.2 được nhận diện bằng header Comments/Likes và ô cnd;
chỉ Back khỏi đúng khay rồi phân loại lại, không đoán nút hoặc nhập bình luận.


Trill38.3.2/en có hai nút Post tùy trạng thái bàn phím: description Post ở dưới và
Button text Post/mr6 ở trên. Nhánh mr6 chỉ nhận khi caption eej đầy đủ, nút duy
nhất và actionable. Sau kiểm tra nhạc phải chờ nút hiện ổn định, đọc lại caption
rồi giải lại nút trước intent. Không giữ tọa độ từ màn bàn phím trước đó.
Video sau chạm ô hồ sơ có thể trả cây profile cũ một lần; chỉ Copy khi cây viewer
Video/caption/Share đã hiện. Global…more là prefix cho quyền đọc link, không là
proof caption; metadata công khai vẫn cần toàn văn, tác giả, content ID và thời gian.
