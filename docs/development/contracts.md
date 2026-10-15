# Hợp đồng thực thi và dữ liệu

[Hướng dẫn phát triển](../developer-guide.md). Tài liệu này mô tả invariant, không chứng nhận live. Mọi thay đổi phải đối chiếu owner source và regression.


Stack giữ nguyên: Rust workspace, Tauri 2, React/TypeScript/Vite. `src/api.ts` là biên
IPC frontend. Không thêm một control plane riêng để đi vòng ownership/admission hiện có.

Đăng bài dùng `publish_start` để lưu receipt trước chuẩn bị chậm và trả ACK;
`publish_start_status` đối soát cùng request ID sau mất phản hồi. Migration 48
giữ receipt khởi chạy và yêu cầu loại assignment. Restart không tự phát lại
Post cho receipt chưa rõ kết quả. Preflight phát `publishPreflightProgress`
theo từng máy; các máy đã chọn được quan sát song song và trả kết quả theo thứ tự
assignment. Bản chuẩn bị tối đa 30 giây chỉ được
tái dùng sau kiểm source, owner, transport, readiness và binding hiện hành.

`scripts/dev_compile_cache.ps1 -Action plan` chỉ ra một Cargo target chuẩn của
checkout chính cho mọi worktree. Lệnh Cargo dùng cache chung phải chạy tuần tự;
compiler cache phụ có giới hạn 8 GiB. Cleanup mặc định dry-run, chỉ nhận danh
sách output cũ có hash và bản thay thế đã kiểm. Không xóa toàn bộ `target`, DB,
runtime đang chạy hay bộ evidence/rollback. Dựng dev để nghiệm thu; chỉ bundle
installer khi có yêu cầu bàn giao bản cài.

Semantic Inspector v2 dùng IPC `inspector_v2` hoặc Local API `POST /v2/inspector`.
`begin` trả sessionToken gắn caller; `end` nhả phiên qua control plane hiện có.
Phiên idle 60 giây hết hạn; tác vụ đã dispatch giữ lease tới khi drain. Ref thuộc
một observation/device/app/session epoch và bị bỏ sau mutation hoặc observe mới.
Legacy Inspector/recording giữ contract cũ. MCP bổ sung begin/end, type/swipe/press,
wait_for/expect/screenshot qua cùng admission, không gọi ADB riêng.

Lệnh IPC mở phiên thiết bị luôn trả lời trong thời gian hữu hạn và nhả lease/slot khi hết
hạn. `interaction_read_account` chờ lượt điều khiển tối đa 600 giây, mở phiên tối đa 90
giây và đọc Hồ sơ tối đa 90 giây. Lệnh chạy trong task riêng có trần tổng, đóng phiên trên
mọi lối ra và trả `AccountReadTimedOut` bằng tiếng Việt. Một lượt đọc treo không giữ slot
mà các lượt khác đang chờ. `with_manual_session` (tap/swipe/gõ/Inspector/Local API) giới
hạn 90 giây cho mở phiên và 90 giây cho thao tác, rồi trả `ManualSessionTimedOut`. Thao
tác hết hạn có thể đã chạy, nên không tự lặp lại.

`UiSession::observe` giữ missing là unknown. Partial/Unknown không chứng minh
phần tử vắng mặt. Positive readback chỉ nhận match quan sát duy nhất và thuộc tính
có thật. Password được che khỏi semantic output. Chế độ
`RIVIU_ANDROID_OBSERVATION_MODE=legacy|enriched` cố định lúc mở session; legacy
mặc định cho đến khi phép đo cùng máy chứng minh enriched đạt.

Account diagnostic lưu trong evidence hiện có, không migration mới.
`RIVIU_TRILL_SECURITY_CLOSE_CANARY=1` chỉ bật thử Close của reminder Trill38.3.2/en;
không dùng Continue hoặc tự đăng nhập. Phải nghiệm thu hình học và profile readback
trước khi bật vận hành.

Seeding dùng `ThreadCampaignRequest.seeding`, planner và ledger Tương tác hiện có;
migration 46 thêm Share và khoảng ordinal cho lượt hành động độc lập bên cạnh tối
đa 64 bình luận. Lịch cũ không có `seeding` giữ hành vi cũ. Bù Tim/Lưu phải claim
ngân sách trong transaction; armed/uncertain vẫn chiếm lượt. Reply chờ bằng chứng
câu cha và không replay Send khi chưa rõ kết quả.

Deployment checker được build từ crate `riviu-deployment-checker`, dùng lại mã
kiểm package của desktop mà không link Tauri. Stage đúng checker sau lần compile
cuối và đối chiếu hash trước bundle; EXE/MSI dùng chung app đã compile một lần.

Thư mục bản giao đặt theo version, ví dụ `target/0.2.65/`, chứa bộ cài và hồ sơ
build của bản đó. Các bản tiếp theo dùng `target/<version>/`; cache Cargo dùng
chung trong `target`, không tạo cache biên dịch riêng cho từng version.

Trước đóng gói, chạy binary release với `--verify-frontend <report.json>` bằng
đường dẫn report tuyệt đối. Report phải chứng minh `embeddedDirectory`, có
`index.html` và đủ các asset được HTML tham chiếu. Bundler Windows kiểm điều kiện
này trước khi tạo installer. Overlay `frontendDist` dùng đường dẫn tương đối với
`src-tauri`; chuỗi Windows dạng `C:/...` bị Tauri giải mã thành URL `c:` và không
nhúng giao diện. Kiểm hash resource không thay thế phép kiểm frontend hoặc phép
mở WebView của bản đã đóng gói.

DTO thiết bị, khả năng, Dừng, timeline và Sheet proof được sinh bằng ts-rs 12.0.1:
`cargo run --locked -p riviu-core --example export_ipc -- apps/desktop/src/generated-ipc.ts`.
`python scripts/check_generated_ipc.py` kiểm không có drift. TanStack Query chỉ
giữ read model persisted theo run/device/phạm vi; metadata thiết bị cache 2 giây,
revision bất biến có thể cache lâu, còn danh sách thay đổi nhanh luôn stale. Mutation
làm stale đúng family liên quan. Retry, focus và reconnect refetch bị tắt. Credential,
preflight, probe thiết bị và effect không dùng query cache.
TanStack 5.103 chạy một timer `refetchInterval` riêng cho mỗi observer và đặt lại timer ở mỗi
lần query cập nhật. Vì vậy một component đọc cùng key với interval ngắn hơn sẽ âm thầm đè nhịp
của nơi sở hữu poll. View phải dùng dữ liệu của lần đọc đó, không thêm observer. Cache đọc một
lần theo phiên bản cần `gcTime` hữu hạn, cộng một observer tắt (`enabled: false`) giữ phiên bản
đang dùng; chỉ có `gcTime` hữu hạn thì mỗi chu kỳ gc lại đọc lại tất cả.
Lệnh `#[tauri::command]` đồng bộ chạy trên luồng UI của WebView (tauri-macros 2.6.3). Lệnh đọc
bị poll hoặc nặng phải là `async` và đọc qua `storage_read`, như `read_for_view` trong
`publish_commands/execution.rs`; lần đọc quá 500 ms (tính cả thời gian chờ lane) ghi cảnh báo
`Đọc chậm <command> …`.
SQLite async đi qua `StorageExecutor`: tối đa một writer, hai reader và 64 yêu cầu
chờ; admission xảy ra trước `spawn_blocking`. Các monitor/guard đọc theo lô trong
executor; transaction không được giữ qua await HTTP hoặc thiết bị.

Publish recovery dùng migration 45 và dispatcher hiện có. `publish_retry_assignment`
nhận `assignmentId`, `confirmed`, `expectedRevision`, `requestId`; replay cùng request
chỉ trả ACK, không tạo attempt khác. Retry tự động tối đa năm lần mỗi bước
(`PRE_POST_RETRY_BUDGET`), thủ công chỉ tạo một lượt bài nhưng vẫn cho tối đa năm retry
tạm thời trong từng bước của lượt đó. Retry trong một bước chờ 2/5/10 giây; requeue cả
lượt mới chờ 30/90/180/300/300 giây (r122 15/10/2026: 8 dòng tiêu hết ba lần trong 23 phút
đầu khi 10 máy cùng soạn, dòng thử lại sau đợt đó thì lên ngay). Lượt thủ công không tự requeue cả bài khi worker kết thúc. Counters,
checkpoint, account và ứng viên nhạc được giữ trong SQLite.
Reconnect đúng serial tối đa 120 giây nhường work permit; startup không tự chạy lại
lượt recovery gián đoạn. Có Post intent thì chỉ xác minh; `publish_retry_sheet_assignment`
chỉ mở lại outbox của bài đã có canonical proof, không đi vào composer.
Recovery state lưu thêm `lastErrorCode` và `lastErrorKind`. Các đường mới truyền
`RecoveryFailure` có kiểu; parser chuỗi chỉ là compatibility boundary cho journal/lỗi
cũ. Thay chữ hiển thị không được đổi retry policy. Retry vẫn chỉ trước Post và tối đa
năm lần mỗi bước.

Publish observation dùng lỗi deadline có kiểu và một lượt đọc lại transient trong
cùng phase/deadline; tạo cursor mới không làm mới ngân sách. `SessionEpochChanged`
giữ nguyên kiểu để chủ phase/dispatcher bỏ target cũ và xác minh lại account, không
chấp nhận observation của phiên mới dưới binding cũ. Caption Unknown được đọc lại;
nội dung khác thật hoặc mơ hồ vẫn từ chối. Bước nhạc có một deadline 180 giây chung
cho các attempt/backoff; hết tổng budget không tự requeue để cấp lại cửa sổ mới.
Lần đọc lại nhạc, caption và nút Đăng ngay trước Post dùng một deadline hữu hạn, đặt
một lần: 1,5 lần thời gian chuẩn bị đo được của cùng phiên, tối thiểu 12 giây, tối đa
16 giây. Thiếu dữ liệu, mơ hồ hoặc hết hạn vẫn là lỗi thử lại trước Post: chưa có intent
và không bấm Đăng.

Mỗi lần chờ trên đường tới màn camera (nút Create, màn camera sau khi bấm, nút chụp, lối
vào thư viện và lần đọc lại để lối vào đứng yên) có cửa sổ 16 giây, vừa hai lần đọc p99 đo
ở r91/r93. Toàn bước thư viện vẫn nằm trong tổng 49 giây cũ. Sau mỗi lần chạm ô ảnh, picker
có 20 giây cho lần đọc lại hoàn tất đầu tiên: r105 có `/source` hoàn tất dài tới 18,3 giây
khi khoảng 30 máy dùng chung 12 slot adb. Tổng thời gian chọn n ảnh tối đa là n cửa sổ như
vậy. Cửa sổ dài hơn chỉ cho lần đọc đang chạy được xong, không cấp thêm lần chạm nào.

Các cửa sổ trên là mức nền đo khi máy rảnh. Mỗi bước quan sát trước Post (camera, thư viện,
picker, album, chọn ảnh, bước chỉnh sửa, điều hướng và nhập caption, đọc lại nhạc) tính cửa sổ
một lần khi bắt đầu bằng `read_latency::window`:
`clamp(max(nền, số_lần_đọc × q90 + 2 s), nền, min(4 × nền, 90 s))`, với q90 lấy từ tối đa 32
lần đọc cây hoàn tất (`GET /source` và lệnh tìm `POST /element`/`/elements`) của chính máy đó
(serial) trong 10 phút gần nhất, cần ít nhất 5 lần;
thiếu số đo thì giữ nền. Không gia hạn sau mỗi lần poll, và mọi cửa sổ vẫn bị chặn bởi ngân
sách chung của giai đoạn (`phase_deadline`), nên không vượt ngân sách nhạc/compose. Cửa sổ
không áp cho bằng chứng tươi trước Post (deadline đọc lại cuối 12–16 giây ở trên vẫn là trần
của mọi lần đọc bên trong), cho bước bấm/effect hay cho chờ về feed sau Post. Hết cửa sổ vẫn
trả cùng mã lỗi như trước; nới cửa sổ không bao giờ biến màn chưa đọc thành quyền bấm.

Ân hạn đọc (`READ_GRACE` = 15 giây, `ui_automation/runtime.rs`): trên đường Đăng thật, khi cửa
sổ của một bước quan sát trước Post hết mà đúng một lần đọc còn đang chạy, bộ bọc đọc
(`read_before_deadline`, `tiktok_composer::observation::read`) chờ tiếp chính lần đọc đó tối đa
15 giây rồi bước xét điều kiện một lần với kết quả ấy; chưa đạt thì kết thúc với cùng mã lỗi
như trước. Một lần locate (tìm rồi đọc khung/thuộc tính) tính là một lần đọc. Sau cửa sổ không
bắt đầu lần đọc mới và không poll lại. Ân hạn dừng ngay khi Stop và không bao giờ vượt ngân sách
nhạc đang bao bước (mọi `with_*_budget` là trần; chính ngân sách hết thì không có ân hạn). Không
áp cho thao tác bấm/gõ/vuốt/back (vẫn xả hết và đối soát như cũ), cho đọc popup (đọc xong vẫn
cần thêm lần đọc của bước nên vô ích), cho bằng chứng tươi trước Post và mọi thứ sau Post
(`close_read_grace` trước `final_reproof_window`). Kết quả nhận trong ân hạn là bằng chứng tươi
như mọi lần đọc; kiểm epoch/generation trước khi bấm vẫn áp dụng.

Dispatcher giữ completion receipt đến khi SQLite xác nhận cùng
assignment/campaign/run/attempt/device/phase/revision. Journal cạnh DB lưu receipt
trước settlement; khôi phục journal trước orphan sweep, chỉ chuyển trạng thái DB,
không gọi điện thoại. Lỗi ghi journal vẫn thử DB; khi cả hai thất bại giữ receipt
trong bộ nhớ và ghi lỗi. Shutdown chỉ thử lưu một lượt, không chờ DB vô hạn.
Startup chưa khôi phục xong chặn admission Publish bằng lỗi rõ ràng và dùng
dispatcher hiện có thử lại; không tạo scheduler mới. Cleanup work claim chỉ nhắm
đúng owner/thiết bị/stage của worker đã kết thúc, không xóa claim của phase kế tiếp.

Preflight Android dùng `DeviceControlPlane::tiktok_action_capabilities` và catalog
`app_automation::action_capabilities` chung cho UI, Nuôi thủ công/lịch/Điều phối,
Tương tác và Đăng bài. Readiness chỉ đọc khóa màn hình, transport và owner, không
đánh thức máy hoặc mở session. Mỗi action thiếu locator/tuple bị từ chối riêng;
`runtimeProofRequired` cho phép vào bước chứng minh target, chưa cấp quyền tap.
Allowlist bản TikTok cho effect nằm ở `riviu_core::tiktok_fleet_policy`
(`TIKTOK_FLEET_BUILDS`, mặc định đúng một bộ `com.zhiliaoapp.musically` / `45.7.3` /
`en`). Package và `versionName` so khớp tuyệt đối; locale chỉ so phần ngôn ngữ trước
`-`/`_`; phiên bản hoặc ngôn ngữ chưa đọc được bị từ chối. Locale Android
(`adb::parse_locale`) lấy theo cấu hình đang chạy (`am get-config`), rồi mục đầu của
`system_locales`, rồi `persist.sys.locale`. Khi không đọc được cấu hình mà hai giá trị
sau khác ngôn ngữ thì locale coi như chưa đọc được: máy Riviu 31 báo
`persist.sys.locale=en-US`, `system_locales=vi-VN` và TikTok 45.7.3 hiện tiếng Việt.
Khi package và phiên bản đúng allowlist mà chỉ ngôn ngữ sai hoặc chưa đọc được
(`BuildNotAllowed::only_language_differs`), lời từ chối nói đúng nguyên nhân và cách đổi
ngôn ngữ, không bảo hạ bản. Khi máy kết nối, sau khi Riviu Helper được xác minh và dưới
lease Repair, `AndroidDriver::switch_system_language_to_english` đổi ngôn ngữ hệ thống sang
`en-US` qua helper `systemLocale` (`CHANGE_CONFIGURATION` cấp bằng `pm grant`), một lần mỗi
boot id (marker ghi bền trước lệnh, ý định ghi nhật ký trước), rồi đọc lại bằng
`parse_locale` và chỉ báo xong khi ra `en`. Không có retry trong cùng boot. Khi helper chưa
gắn hoặc lệnh helper không ăn (Samsung Android 9, 12/10/2026: vẫn `vi-rVN`), Riviu ghi
`settings put system system_locales` với `en-US` đứng đầu (giữ các locale còn lại), đọc
lại, rồi tự khởi động lại máy một lần để áp dụng — chỉ khi sổ ghi không còn việc chưa rõ
hay đang chạy trên máy và chưa dùng lượt khởi động lại 6 giờ của bộ theo dõi tải; nếu
không, ngôn ngữ áp dụng ở lần khởi động sau.
Mọi action trong
`action_capabilities` là effect, nên bản ngoài allowlist trả `buildNotAllowed` cho tất
cả action và `require_actions` trả một thông báo duy nhất. Publish preflight thêm issue
`build_not_allowed`, `publish_readiness` trả `buildNotAllowed`, và refusal trước
transfer không kèm hướng dẫn đo composer. Khi phiên effect mở, cùng allowlist được kiểm
lại trước tap đầu tiên: `run_cohort` của Tương tác (Android), `HierarchyRun::prepare`
của Nuôi, `publish_runtime_labels` của Đăng bài (`build_not_allowed`, Terminal, chưa
đăng gì) và `execute_ready` của dọn công khai. `controls_for_runtime` và đường adaptive
giữ nguyên cho việc chỉ đọc: xác minh link bài đã gửi, đọc account, chẩn đoán và kiểm
bình luận không hỏi allowlist. Test adapter trên bản khác truyền
`TikTokFleetPolicy::allowing` qua `action_capabilities_with_policy`, không xoá coverage.
Không có override trong Cài đặt; iPhone (đường pixel) không đọc được bản TikTok nên
chưa đi qua allowlist này.
Migration 47 lưu `device_app_bindings` theo `(udid, appKey)` với CAS. Resolver nằm
trong `DeviceControlPlane`; Android luôn đọc lại danh sách package để chứng minh lựa
chọn còn cài và có adapter. `tiktok_build` và resolve package dùng cùng binding. Máy
chưa có binding giữ foreground fallback cũ; UI buộc chọn khi fallback còn mơ hồ.
Handoff đóng package từ completion journal trước fallback và trả kết quả từng máy;
frontend không dispatch lượt mới nếu còn một máy `closed=false`.
Follow trong Tương tác dùng profile đã đo ở Trill38.3.2/en và Global
45.4.3/45.7.3/46.0.41/46.1.3/46.4.3/en;
source proof Follow ngẫu nhiên của Nuôi dùng capability `feedFollow` riêng ở
Trill38.3.2/en. Capability `follow` của profile không cấp quyền Follow trong Nuôi.
Action Follow mặc định false khi đọc request cũ. Worker xác nhận account người
chạy, canonical bài đích, rồi đúng handle trên profile trước intent. Không tự Follow
chính mình; quan hệ đã đạt là no-op. Tap chỉ đi qua action ledger một lần, kết quả
chưa đọc lại được giữ uncertain, không biến thành quyền tap lại.
`follow_profile` chỉ trả `followed` sau khi mở lại URL profile chuẩn và đọc hai
snapshot mới có trạng thái Following. Trạng thái tức thời sau tap không đủ;
thất bại ở bước mở lại là uncertainty sau effect, không cấp quyền thử Follow lần nữa.
Đối chiếu Follow mở URL profile chuẩn của handle đã xác minh, đọc hai snapshot mới
liên tiếp; không gọi tap hoặc gate Follow. Kết quả hiện tại không ghi đè journal uncertain.
Điều hướng tác giả lấy định danh và tọa độ từ cùng snapshot XML, kiểm lại trên
generation mới trước tap; không ghép kết quả find/rect của node đã tái sử dụng.
Nếu máy đã có nick gán, transaction giữ account và transaction ghi intent Post
đều so với account đọc được. Đổi nick giữa hai transaction làm lượt Đăng bị từ chối;
không tự sửa metadata từ quan sát và không sửa bằng chứng bài đã đăng trước đó.
Chờ đọc Android dùng một deadline cho từng bước, kiểm Dừng cả trước và sau đọc.
Lỗi transport/timeout được phân loại riêng; không coi nó là nút vắng mặt. Chờ
semantic chỉ phục hồi session một lần với package/device/epoch được chứng minh,
sau đó dùng đường đọc không phục hồi lại. Selector và XPath cùng đọc cây trợ năng;
fallback điều hướng cục bộ không gọi provider. OCR/template chỉ dùng profile đã đo
và ảnh mới, không cấp quyền bấm từ kết quả mơ hồ. Timeline đăng lưu bước, chiến
lược và kết quả phục hồi đọc trong các event hiện có, không đổi checkpoint.
Flow Delay, khoảng xem video/nghỉ, polling và backoff vẫn giữ nguyên. Chỉ các điểm
chờ sẵn sàng đã có hợp đồng quan sát mới dùng điều kiện; không thay hàng loạt sleep.
Không bọc timeout quanh future đã gửi tap/Back/Send/Post: cho thao tác kết thúc,
rồi kiểm Dừng và đối soát trước bước kế tiếp. Không tự phát lại effect chưa rõ.

Sau khi bấm Next của picker, lỗi accessibility/HTTP timeout ở bước chờ editor
cho phép quan sát XML trong tổng tối đa 30 giây, kể cả lần đọc đầu. Chỉ hai generation mới trong cùng phiên,
cùng một nút Next đã đo và không có Loading mới chứng minh đã tới editor.
Phục hồi này không bấm lại picker, không ghi intent và không gọi Đăng; Dừng vẫn
được kiểm trước tap và sau mỗi lần đọc.
Global 45.7.3/en có nhánh phục hồi riêng khi đã chọn nhạc nhưng không đọc được
bảng nhạc: hai ảnh mới 1080×2220 qua OCR cục bộ phải nhận diện đủ bốn tab ổn định
trong cùng phiên trước khi Back một lần. Sau đó hai snapshot XML mới phải xác
nhận đúng tên nhạc duy nhất trên editor. Lỗi/mơ hồ/Dừng vẫn từ chối; không bấm
chọn nhạc lại và không nới deadline chung ba phút.
Khi nhánh XML gặp cây rỗng hoặc lỗi accessibility, hai ảnh mới cho phép chuyển
tab Hot một lần. Nhánh này vẫn bắt buộc XML xác nhận Hot và danh sách đầy đủ.
Riêng Global 45.7.3/en, nếu Android trả ảnh chụp không phải PNG (kể cả 0 byte),
không dùng ảnh đó để suy vị trí. Nhánh mở nhạc chỉ nhận một nút entry qua hai XML
mới cùng app/phiên, đọc lại đúng nút và app ngay trước một tap. Sau khi mở, chỉ
tiếp tục khi XML chứng minh Hot đã chọn và hàng nhạc ổn định; không có ảnh thì
không gọi đường chọn tab bằng ảnh/OCR. Hết hạn hoặc XML đổi thì dừng trước Post.
Trên 45.7.3/en, hai ảnh native mới có Next và Your Story chứng minh editor đã
mở; nhạc gợi ý đang Loading là trạng thái riêng. Mẫu chữ Loading phải khớp
hai ảnh mới trước khi mở nút nhạc đã đo. Bảng mở dở được quan sát tiếp; khi đủ
bốn tab mới chuyển Hot, không đợi nội dung For You. Giữ ngân sách chung ba phút.
Trên đúng 45.7.3/en ở 1080×2220, adapter ảnh dùng hai quan sát mới cùng phiên
để chứng minh gạch chân Hot, tên và nghệ sĩ của từng hàng đầy đủ. Hàng hồng đã
chọn, tên trùng và hàng bị cắt bị loại; trước chọn phải đọc lại cùng danh tính.
Nếu bảng Hot đã lộ đủ XML trên Global 45.7.3/en, hai snapshot XML mới tăng
generation và có cùng danh sách hàng hoàn chỉnh được đọc trước OCR. Nhánh này
không cần OCR khi dịch vụ OCR báo 429; nó vẫn loại tên trùng/hàng bị cắt, kiểm
app và phiên, rồi đọc lại cùng hàng trước khi tap. XML chưa đủ hoặc không ổn
định thì tiếp tục đường ảnh/OCR cũ; không suy hàng từ ảnh hay tap theo vị trí.
Nếu OCR không dựng được pool trong 60 giây, nhánh này xác nhận lại sheet bằng
hai quan sát mới rồi dùng snapshot XML chỉ khi Hot đã chọn và hàng nhạc đầy đủ,
ổn định. XML không rõ trả lỗi có thể retry trước Post, không tap hàng nào.
Nhánh này chỉ chọn một lần, chờ hai ảnh mới có đúng tên nhạc chuyển hồng để
xác nhận lựa chọn đã tải xong, rồi đóng bảng đã chứng minh bằng ảnh; hai XML mới
trên editor phải khớp nguyên tên nhạc. Không khớp thì giữ lỗi trước Đăng.
Trên Global 45.7.3/en có OCR, hai lần liên tiếp `/source` không có gốc accessibility
(`AccessibilityReadUnavailable` hoặc timeout gốc) trong cùng một pha đọc bảng nhạc
thì pha đó dừng hỏi XML và chuyển sang đường ảnh đã đo, không tap. Ở pha đọc danh
sách thì bỏ cả lượt XML trực tiếp; ở pha đọc lại trước khi chọn thì danh tính
tên/nghệ sĩ đã chốt phải khớp đúng một hàng trong hai ảnh ổn định. Hàng đã hiện chọn
không bao giờ bị bấm lại. Sau khi bấm hàng, cửa sổ đọc lại 8 giây hết mà chưa có lần
đọc nào trả về được coi là gốc tạm không đọc được và đi nhánh editor/ảnh bảng như lỗi
accessibility, không phải `sound_load_timeout`. Xác nhận sau chọn bằng ảnh dùng chip
`tv_top_text` qua XML; chỉ khi không lần đọc editor nào trả cây thì hai ảnh mới cùng
phiên, không thấy tab bảng nhạc, có OCR chip đúng tên đã chốt mới thay được. Bằng
chứng OCR này không cho phép Đăng: bước kiểm nhạc cuối trước Đăng vẫn đọc XML.
Dòng tiến trình `ReadRecovery` ghi đường đã chọn (`selectionRoute`: `xml`/`visual`) và
số lần không đọc được gốc; deadline chung ba phút không đổi.
Đếm "liên tiếp" tính cho cả bước nhạc, không phải từng pha: lần thử lại tại chỗ sau
lỗi tạm và pha mở bảng nhạc chạy lại không đặt lại số đếm, chỉ một lần đọc trả về gốc
mới đặt lại. Khi đã đủ hai, pha mở/đọc danh sách vào thẳng đường ảnh, và ảnh bảng nhạc
không còn hỏi XML kiểm tra hộp thoại trước mỗi khung hình: không nút hộp thoại nào được
bấm khi thiếu XML, còn hộp thoại che bảng làm mất tab/hàng đã đo nên không có tap.
`ReadRecovery` ghi `rootUnreadable`/`visualHandoff` cho cả pha mở bảng nhạc.
Một `GET /source` chỉ đọc chạm hạn yêu cầu của driver (30 giây, `agent_request_timeout`;
#44, box3150g 2026-10-10 01:01) trên bảng nhạc cũng là một gốc không đọc được như lỗi gốc
~10,5 giây, không phải lỗi tạm để đọc thêm: pha có thể thay phiên đi đường đọc qua mọi cửa
sổ rồi đường ảnh y như gốc null. Sau một lần chạm hạn như vậy (kể cả `/source` bị bỏ ở hạn
12 giây của chính bước nhạc), gốc không đọc được kế tiếp đã đủ để chuyển sang đường ảnh, nên
mỗi bước nhạc tốn tối đa một lần đọc 30 giây; chỉ `/source` chỉ đọc được phân loại như vậy,
không yêu cầu gây hiệu ứng nào bị gửi lại. Trước khi bấm hàng, bước nhạc đã đủ số gốc không
đọc được thì không hỏi thêm XML nào (không kiểm hộp thoại, không đọc lại danh sách, lần thử
lại tại chỗ không đọc lại editor): pool Hot giao danh tính đã chốt cho đường ảnh; tab khác
(For You) chưa đo đường ảnh nên lượt thử kết thúc trước mọi tap với
`sound_any_choice_requires_fresh_attempt`. Bước nhạc thấy bảng nhạc mù (đủ hai gốc không
đọc được, một `/source` chạm hạn 30 giây, hoặc vào thẳng đường ảnh vì lượt trước đã mù) mà hết
ba phút thì lượt kết thúc với `sound_sheet_blind_requires_fresh_attempt`: dispatcher tự cấp
đúng một lượt mới trước Post cho mỗi phân công (mở lại TikTok sạch, phiên helper mới; khóa
`soundSheetBlind` trong sổ khôi phục), bảng mù lần thứ hai thì hết lượt tự thử. Hết ba phút
mà bảng không mù là `publish_sound_budget_exhausted`, cũng retryable và dùng chung đúng một lượt
mới đó (khóa `soundSheetBlind`): mỗi phân công có tối đa một lượt tự thử lại sau khi hết giờ
chọn nhạc, mù hay không, luôn trước ý định Post. Đo 12/10/2026: ba trong năm máy hết giờ
nhạc ở r91 (`d8e672cc`) đăng được ở r93 (`4a25d804`) hai giờ sau. Cả hai, chưa bấm Đăng, nói rõ TikTok
không trả giao diện bảng nhạc, thêm "bảng nhạc không chuyển sang tab Hot" khi đường ảnh hết
giờ mà chưa khung nào thấy Hot, và bước tiếp: dừng phần mềm khác (như GenFarmer) điều khiển máy,
khởi động lại máy nếu máy không còn bài đăng chưa rõ, rồi Thử lại.

Nguồn của tên nhạc kỳ vọng là một hàng trong pool của plan (quy tắc chủ sản phẩm 2026-10-12,
"Bắt buộc Hot"). Trên plan có pool Hot (Global musically), policy `Default` và `TrendingAny`
đều chỉ nhận hàng Hot. Composer không bao giờ yêu cầu pool For You hiện tại (cờ
`allow_current_recommended` luôn tắt). Bảng mở ở For You thì chuyển Hot qua đường XML hoặc đường
ảnh đã đo. Không chứng minh được Hot thì lỗi trước Post như trên, không có đường lùi sang For
You, và cũng không giữ nhạc TikTok tự gắn trên editor. Lựa chọn `Recommended` do lượt cũ chốt
không được dùng lại: lượt kết thúc trước khi mở bảng với `sound_any_choice_requires_fresh_attempt`,
và dispatcher cấp một lượt mới trước Post (binding cũ bị xóa; không có lượt mới khi đã có Post
intent). Trill 38.3.2 không có tab Hot; pool đã đo của bản đó vẫn là Recommended.
Khi gốc bảng nhạc không đọc được ngay trong pha có thể thay phiên (#44/#47, 2026-10-09:
gốc cửa sổ đang hoạt động vẫn null kể cả trên server UiAutomator2 mới), Android thử đọc
qua mọi cửa sổ trước: bật `enableMultiWindows: true` trên đúng phiên đó qua
`/appium/settings`, đọc lại cài đặt để chứng minh, rồi đọc lại `/source`; cây trả về có
một gốc cho mỗi cửa sổ nên chỉ giữ các cửa sổ của gói TikTok đã chọn (bỏ system UI, bàn
phím), không có cửa sổ nào của gói đó thì coi là chưa đọc được, không phải vắng. Lỗi đọc
đó (`AccessibilityReadUnavailable`) có thể thử lại trước Post như lỗi gốc ~10,5 giây, dù
câu chữ không có từ timeout (`accessibility_read_unavailable`), trừ khi chuỗi lỗi có lý do
từ chối cụ thể; run 72401bfd (máy khác, 2026-10-12) từng kết thúc `legacy_terminal` ở
preRow vì câu chữ này. Các bộ
phân loại bảng nhạc chạy y nguyên trên cây đã lọc; epoch không đổi và mọi guard epoch,
foreground, generation mới trước tap giữ nguyên. Tối đa một lần chuyển mỗi bước nhạc;
`ReadRecovery` ghi `multiWindowRead` (`ready`/`unreadable`/`failed`/`unsupported`) kèm
thời gian bật và đọc lại, `selectionRoute` ghi `multiWindowRead: true/false`. Cuối bước
nhạc, trên mọi đường thoát, đặt lại `enableMultiWindows: false` và đọc lại xác nhận
(`multiWindowRestore`); không khôi phục được thì phiên bị niêm phong (`SessionSealed`),
không dùng cho màn chỉnh sửa hay Đăng, lượt thử mới mở phiên khác. Khi đọc qua mọi cửa
sổ cũng không có cây và đã đủ hai gốc không đọc được liên tiếp trên phiên có OCR đo sẵn
(#47, box3150e 2026-10-10: server khởi động lại lên `ok=true` nhưng vẫn mù y như cũ), pha
đọc chuyển thẳng sang đường ảnh, không khởi động lại và không hỏi thêm `/source` nào
(`ReadRecovery` ghi `rootUnreadable`/`visualHandoff` ở bước đọc qua mọi cửa sổ); chỉ phiên
không có đường ảnh mới khởi động lại instrumentation một lần như trước. Trên đường ảnh,
mọi tap vào bảng (tab Hot, hàng nhạc) là tap contact 45–130 ms tại điểm OCR trên khung
1080×2220, sau khi chứng minh màn hình cùng kích thước, cùng phiên và cùng app ngay trước
cử chỉ (machine29 và #47: tap native 16 ms được ACK nhưng không đổi tab, không chọn hàng).
Tap contact đó đi qua `input swipe` của shell adb (giữ tại chỗ 45–130 ms), không qua `/actions`
của helper: đường ảnh chỉ chạy khi accessibility của bảng đã mù, và run 72401bfd (máy khác,
2026-10-12) có tap Hot qua helper được ACK trong 0,5–0,7 giây mà 64–172 giây không khung nào
thấy Hot. Chỉ phiên không có shell (chưa gửi gì) mới lùi về helper; tap shell lỗi thì không tap
lại ngay (`sound_shell_tap_failed`, thử lại trước Post trong ngân sách). Hai từ chối trước tap
có mã riêng thay vì `legacy_terminal`: màn hình khác 1080×2220 (`sound_visual_screen_unmeasured`,
terminal) và phiên/app đã đổi (`sound_visual_target_changed`, thử lại);
tab Hot tối đa hai lần và chỉ khi hai ảnh mới chưa đổi, hàng nhạc đúng một lần, rồi chờ hai
ảnh mới có hàng chuyển hồng. OCR không bao giờ là bằng chứng chọn: `editorProof` ghi `xml`
khi hai snapshot XML mới của màn chỉnh sửa cùng tên nhạc, và chỉ bằng chứng đó mới xác
nhận lựa chọn (`visualSelection` ghi thời gian bấm hàng/đóng bảng và xác nhận editor);
màn chỉnh sửa không xác nhận thì lượt thử kết thúc trước Đăng. Sau khi đã
dừng server cũ mà phiên mới không lên thì phiên cũ cũng bị niêm phong, mọi yêu cầu sau đó
bị từ chối ngay trước khi ra mạng và vòng dọn composer (Back + đọc) dừng ở lỗi đầu tiên
thay vì gõ vào server đã chết. Sau khi khởi động lại cho chủ đọc, bằng chứng sẵn sàng là
`/status` cộng phiên mở và prime được, không phải `/element` (lệnh đó chờ chính gốc cửa
sổ đang hoạt động vừa làm chủ đọc phải khởi động lại); chính lần đọc bảng nhạc kế tiếp
của chủ đọc mới là bằng chứng đọc được.
Trên 45.7.3, APK khai báo lại các id `title` (408 layout), `tv_title` (303), `cover`
(88), `desc` (75), `bix` (61), `esv`, `z3k`, `cqf`, `tv_top` và `gpr` ở nhiều màn khác,
nên một lần khớp id duy nhất chưa đủ làm bằng chứng. Mỗi chỗ dùng phải có phạm vi:
`title`/`z3k` nằm trong hàng nhạc đã đo, pill album `tv_title` chỉ tính khi Next của
picker cùng cây, `bix` trong rehearsal chỉ là Back của trang caption khi ô caption cùng
cây, `esv` phải khớp nguyên văn câu hỏi, `tv_top` chỉ loại ô nằm trong ô `cover`, còn
`cover`/`desc` trên đường xác minh vẫn phải qua bằng chứng tài khoản và link chuẩn.
Bản 45.7.3 còn có neo `content-desc` riêng của bản build (chuỗi chưa resolve
`@<id chuỗi>`, giống nhau mọi ngôn ngữ, đổi số mỗi bản build; đọc tĩnh APK 09/10/2026):
`cqf` Gửi bình luận `@2131823090`, `sn3` nút quay `@2131823130`, `bix` Back
`@2131823215`. Nơi đọc control từ cây thì desc phải khớp cùng id (`cqf` cũng là nút Gửi
trong tin nhắn). Trên bảng nhạc 45.7.3, mỗi hàng nằm trong một khung bấm được `opr`;
`nms` chỉ hiện ở hàng đang phát nên đánh dấu hàng vừa chọn. Bảng có thể mở ở For You
nên vẫn phải bấm tab Hot. Bấm lại hàng đang phát sẽ bỏ chọn nhạc, nên không bao giờ
dùng thao tác đó để tạm dừng.
Tap từ ảnh dùng `tap_image` với đúng kích thước native, không thêm jitter.
Nguồn ảnh minicap giữ kích thước native và giới hạn 2 FPS để giảm tải encode
trên điện thoại; giới hạn này không thay chất lượng stream H.264 của ô thiết bị.
Thông báo instrumentation Android chỉ nêu công cụ khác đang giữ phiên khi
ActivityManager ghi nhận runner đang hoạt động. Danh sách package đã cài không
phải bằng chứng tranh quyền accessibility.
Like/Save/Follow/Comment trên Android cũng kiểm nick gán trước khi mở bài đích.
Harness nghiệm thu khóa nick trong preflight và so account trong intent/canonical
URL; receipt và ô Sheet khớp chưa đủ để công nhận đúng account.

Timeline Script lưu bước và event của đúng máy trong cùng transaction, gồm session
và thời gian thực thi; Dừng khi chờ admission không được ghi intent mới. Điều phối
đọc timeline của đúng tác vụ con và snapshot máy của từng bước. Export kiểm hash
ảnh/XML; replay chỉ dùng fake driver. Kết quả chung của bước không thay bằng chứng
của từng máy.

Tag nickname Global45.7.3/en chỉ được chấp nhận khi snapshot trước chọn chứa
đúng handle trong hàng picker o3b/o2g, cùng tên hiển thị o2n. Sau chọn phải còn
cùng session, picker đã rời và ô soạn khớp toàn bộ phép thay token đã dự kiến;
đọc cuối trước Gửi phải khớp nguyên văn kết quả đã chứng minh. Không suy nickname
thành username và không dùng mapping này cho build khác chưa đo.

Lịch tự động (`automation_schedules`) ghim bản hồ sơ và, từ migration 51, danh sách máy
đã duyệt (`approved_udids_json`). Claim một lần đến hạn so target vừa resolve với danh sách
đó trong cùng transaction (`approved_scope_difference`): máy được include mà không có trong
danh sách, hoặc máy đã duyệt mà nhóm/danh sách không còn chỉ tới, làm occurrence `failed`
với `approved_scope_changed`; lịch chưa có danh sách (lịch cũ theo nhóm hoặc Toàn bộ) cho
`scope_not_approved`. Cả hai không tạo child, không gửi lệnh, vẫn dời `next_due_at` và ghi
`last_error_code`. Máy đã duyệt nhưng mất kết nối là exclusion runtime trong snapshot, không
phải đổi phạm vi. Migration 51 coi danh sách explicit của bản hồ sơ đang ghim là đã duyệt.
Đổi tên, chu kỳ, bật/tắt hay revision giữ danh sách; chỉ tham số `approvedUdids` thay nó.
`scheduleScope.ts` ở UI chép cùng quy tắc chỉ để giải thích; backend quyết định.

Lịch Nuôi (`nurture.settings`, `nurture_schedule::decide`) chỉ chạy trên danh sách đã duyệt:
`schedule_udids` cho lịch cả ngày, `udids` của khung cho từng khung. `decide` không nhận roster
đang kết nối; danh sách rỗng trả `Tick::Refuse` (`scope_not_approved`), dời mark và ghi
`nurture.schedule.refused`, không bao giờ mở rộng ra cả fleet. `claim_nurture_schedule_mark`
đọc lại settings trong cùng transaction và từ chối máy ngoài danh sách của đúng mark đó hoặc
danh sách rỗng. Danh sách chỉ được ghi từ Hẹn giờ (Lưu / Lưu & bật); lưu thiết lập giữ danh
sách cũ, và bắt đầu thủ công lưu thiết lập với công tắc và mọi danh sách của lịch đúng như bản
đã lưu (khung mới trong bản nháp để rỗng), nên không thay hay tự gieo phạm vi lịch.

`RIVIU_DEV_MANUAL_ACCEPTANCE=1` trong bản debug giữ mọi lịch tự chạy ở trạng thái
chờ và không sửa lịch đã lưu. Chế độ này cũng không tự resume Flow/Điều phối,
không chạy idle sweep/comment verifier, không tự đóng app và không bind
Local API đã lưu. Dispatcher chỉ nhận đúng cặp campaign/máy trong file tuyệt đối
`RIVIU_DEV_ACCEPTANCE_SCOPE`; `RIVIU_DEV_ACCEPTANCE_ACTIVATION` phải khớp
`activationId` trong file. Thiếu file/activation, JSON lỗi hoặc ID không khớp đều
đóng kín. Backend chụp danh sách tối đa 100 máy lúc khởi động và pin toàn bộ danh
sách campaign được kích hoạt (tối đa 100); thay campaign hoặc danh sách máy giữa
phiên làm toàn bộ gate đóng lại. Giới hạn concurrent production vẫn được giữ.
Nghiệm thu hẹn giờ phải bật riêng `capabilities.publishSchedule`; chỉ lịch có
campaign và toàn bộ máy nằm trong scope mới được clock production nhận khi tới
giờ. Các lịch khác, Nuôi và Điều phối vẫn đóng băng, không sửa lịch đã lưu.

Verifier nền chỉ nhận bài đến hạn khi có `publishVerification` hợp lệ cho đúng
campaign và serial. Nó kiểm lại khoảng 2 phút sau lượt trước; dừng sau 8 lượt liên tiếp không có bằng chứng mới
khi tìm bài (lượt TikTok còn xử lý bài theo dõi bằng hạn 60 phút riêng), và có ngân sách riêng 12 lần metadata chưa sẵn sàng của candidate đã ràng buộc;
không tự mở lại bài cần review. Truy vấn và cập nhật lượt thiết bị giới hạn trong
scope. Cleanup nền cần riêng `publishCleanup` và chỉ dọn bản chuyển đã có proof
canonical qua kiểm revision/lease hiện có. Thiếu hoặc đổi scope thì không nhận
việc mới; dùng Dừng để hủy và nhả việc đang chạy.

Scope tối thiểu chỉ mở dispatch do harness đã xác nhận; verifier và Sheet vẫn tắt:

```json
{
  "activationId": "acceptance-20260922-random-id",
  "campaignIds": ["campaign-id"],
  "deviceIds": ["android-serial"],
  "capabilities": {
    "publishVerification": false,
    "sheetDelivery": false
  }
}
```

Khởi động debug với cùng giá trị, ví dụ
`RIVIU_DEV_ACCEPTANCE_ACTIVATION=acceptance-20260922-random-id`. File ban đầu phải
có `campaignIds: []`, đúng danh sách máy và cả hai capability tắt; harness chỉ thay
atomic sang một campaign sau khi đã lưu create receipt. Production/release không
đọc quyền này.

Chỉ bật từng capability trong lần kích hoạt atomic khi lượt nghiệm thu thật cần nó;
backend pin bộ cờ ở lần đọc active đầu tiên, muốn đổi phải dừng và tạo activation mới.
`publishVerification` cho phép worker đọc lại đúng publication trên đúng máy;
`sheetDelivery` cho phép gửi đúng assignment của campaign/máy trong scope. Hai cờ
mặc định `false` và không được suy ra từ việc campaign đã nằm trong scope. Scheduler
Đăng bài đứng yên nếu capability publishSchedule chưa được bật trong scope;
harness phải gọi lệnh Execute đã xác nhận. Thay scope chỉ ảnh hưởng claim kế tiếp,
không hủy effect đang in-flight vì hủy giữa intent và hậu kiểm sẽ làm mất trạng thái.
Harness nhận `--content-snapshot` JSON chứa `captionOverrides` và `soundPolicy`.
Đạt end-to-end đòi canonical post proof, receipt đúng revision/epoch và đọc lại
ô qua `publish_sheet_readback`; trạng thái sent một mình chưa đủ.

Migration 44 backfill nghĩa vụ Sheet một lần; trigger cập nhật nghĩa vụ ngay
trong transaction publication. Scan shared-v2 ghi checkpoint mỗi trang, nhường
sau 32 trang hoặc 45 giây và giữ lock/token/payload qua lát 90 giây. Pending
không tăng lỗi. Receipt giữ theo publication/revision/target/epoch. Storage
executor cấp chỗ trước spawn_blocking với 1 ghi, 2 đọc, tối đa 64 yêu cầu chờ.
Monitor list/query/detail/log, settings CAS/credential và các bước đọc cấu hình,
nhận claim, settle/defer/fail Sheet dùng executor chung. HTTP và thao tác thiết bị
chạy ngoài closure storage; không giữ transaction hoặc slot DB qua các bước đó.
Danh sách publication dùng một kết nối và projection hiện có, không tải manifest
media hoặc lịch sử event. DTO trả về vẫn giữ uncertainty, marker Dừng và quyền retry.
Khôi phục schema bằng backup SQLite tương ứng; không mở binary cũ trên DB đã migrate.

Phiên Android có `GuiScope` ghi quan sát vào `artifacts/traces` qua artifact store
hiện có: run, device, assignment/step, session epoch, thứ tự, thời gian và lỗi.
Các lần đọc hierarchy giữ XML/hash; ảnh là frame đang có trong stream, chỉ phục vụ
chẩn đoán vì độ mới chưa được xác minh. ACK của tap không thay hậu điều kiện nghiệp vụ.
Phiên đối soát publication không mở stream bằng chứng sẽ chụp một ảnh có deadline
5 giây trước khi nhả owner; ảnh này cũng chỉ phục vụ chẩn đoán, không thay proof bài.
Một worker ghi và hàng đợi 64 bản ghi tách I/O đĩa khỏi deadline điều khiển; lỗi
ghi hoặc đầy hàng đợi làm export báo trace chưa đủ. Export và shutdown có flush.
`operation_trace_export` kiểm scope thiết bị và hash, lưu bundle ở
`artifacts/trace-exports` để không bị reconcile artifact Flow cách ly khi restart.
Retention `artifacts/traces` (`ui_automation/trace_retention.rs`): worker nền chạy 3
phút sau khởi động rồi mỗi 6 giờ, xoá thật thư mục run (không qua Recycle Bin) có
tệp mới nhất quá 7 ngày, rồi xoá run cũ nhất cho tới khi tổng dưới 20 GiB; hai trần
này là đề xuất 14/10/2026 trước tồn 53 GB, chưa đo theo nhu cầu chẩn đoán. Run mới
hơn 24 giờ không bao giờ bị xoá vì dung lượng. Run của campaign còn bài
`posting`/`verifying`/`uncertain` hoặc `needsReview` được bảo vệ theo
`run_folder_id(campaign_id)`; `.staging`, `.quarantine` và tên không phải run id
không bị chạm.
`cargo run --locked -p riviu-core --example trace_replay -- TRACE_JSON` chỉ đọc
timeline/XML bằng fake driver; không kết nối điện thoại hay phát lại public effect.

Watchdog view giữ admission trong cả lượt kiểm và các task start/restart đã join.
Shutdown ngừng cấp lượt mới trước khi drain và dừng view. Nếu DELETE session
Android mất ACK, cleanup kiểm PID của đúng hai package agent trên từng serial
đang sở hữu sau teardown; không coi lỗi transport là bằng chứng tiến trình đã vắng.

Windows sidecar pin cryptography 50.0.1, tornado 6.5.10. macOS giữ cryptography
48.0.0 vì đường wheel Intel; kết quả audit Windows không chứng nhận macOS.
`scripts/check_dependency_drift.py --toolchains --python-runtime` kiểm pin thực tế.
`scripts/dev_compile_cache.ps1 configure` bật sccache nếu đã cài, giới hạn 8 GB
trong shell hiện tại và giữ nguyên tối ưu release.

Google Sheets trực tiếp dùng OAuth Desktop PKCE S256 và callback loopback có
state, timeout và hủy; đăng nhập mở trình duyệt ngoài. Scope là `openid`, `email`
và `https://www.googleapis.com/auth/spreadsheets`; kết nối bằng link, không mở
Picker. Scope này rộng hơn `drive.file`: tài khoản vẫn phải có quyền sửa Sheet.
Phiên cũ chỉ có `drive.file` cần đăng nhập lại và cấp quyền Sheets. Refresh token
và client config dùng SecretStore hiện có; IPC chỉ trả identity/trạng thái,
không trả token.

Hợp đồng shared-v2 dùng writer UUID riêng từng PC, reportingEpoch và khóa bền
trên developer metadata để tuần tự hóa các client cùng giao thức. Journal local
ghi intent trước request; updateCells, receipt và dấu commit được ghi cùng batch
rồi đọc lại. Không append không khóa, replay POST mơ hồ hoặc lấy khóa của PC khác
chỉ vì quá TTL. Khi chưa rõ request đã áp dụng chưa, giữ pending để đối chiếu,
không tự xóa khóa. Đây không phải CAS chống sửa tay và không chặn được request
của client cũ đã bay trước nâng cấp.

Tab schema 1 trả mã lỗi typed `SharedSheetUpgradeRequired`; UI chỉ gửi
`legacyWritersStopped=true` sau xác nhận nâng cấp một lần. Người vận hành phải
dừng mọi writer cũ và đợi request đang bay kết thúc trước xác nhận. Giữ dữ liệu,
publicationId và epoch; chỉ các client shared-v2 được tiếp tục ghi. Reset/backup
qua luồng reset cũ bị từ chối với schema 2, trước khi sao lưu hoặc xóa. Hợp đồng
và test mô phỏng không thay nghiệm thu ghi đồng thời trên nhiều PC thật.

Cấu hình ứng dụng Google bắt buộc đi kèm binary release. `build.rs` đọc biến
`RIVIU_GOOGLE_OAUTH_CONFIG_JSON` (CI đọc GitHub Actions secret cùng tên tại cả bước
build deployment checker và đóng gói app). Build tại máy phát triển có thể dùng
`apps/desktop/src-tauri/google-oauth.local.json` đã gitignore, hoặc đường dẫn qua
`RIVIU_GOOGLE_OAUTH_CONFIG_FILE`. Biến JSON có ưu tiên cao nhất, kể cả khi rỗng.
JSON gồm `clientId`, `clientSecret` (tùy chọn); `pickerApiKey` và `projectNumber`
cũ vẫn được chấp nhận nhưng không bắt buộc cho kết nối bằng link. Client ID
phải thuộc OAuth Desktop. Không đưa access token, refresh token hay tài khoản vào
JSON này; các trường lạ bị từ chối. Đây là cấu hình phân phối trong binary, không
phải nơi giữ bí mật server. Cấu hình đã lưu trên PC luôn được ưu tiên; trước đăng
nhập app ghim cấu hình vào SecretStore để nâng cấp binary không đổi client của
phiên cũ. Build release dừng nếu cấu hình OAuth thiếu hoặc sai; debug
không cấu hình vẫn dùng được để phát triển phần khác. Đặt
`RIVIU_REQUIRE_GOOGLE_CONFIG=1` để kiểm tra cùng điều kiện trong bản dev.
Cấu hình được ghi vào OUT_DIR rồi nhúng vào binary, không in giá trị ra log.
Không đưa file cấu hình local vào source, log hay gói chứng cứ.
Chép `.exe` đã build đủ cấu hình không cần chép SecretStore; máy đích tự đăng
nhập tài khoản Google của mình. Build không tự đọc credential của người dùng.
Kiểm tra OAuth thật cần người vận hành tự đăng nhập; không dùng credential của
máy phát triển để chứng minh kết nối trên PC khác.

Chuyển Apps Script sang direct trên tab chưa có direct owner ghi intent local
trước, dừng nhận claim, drain, gọi retirement dưới ScriptLock, rồi nhận writer
trên tab và commit provider. Metadata direct schema 1/2 đã xác thực đi theo luồng
nâng cấp/join, không gọi retirement lại bằng request/writer của PC mới dù cấu hình
webhook cũ còn lưu; schema 1 vẫn bắt buộc xác nhận dừng và drain writer cũ.
Checkpoint đang chờ giữ nguyên requestId, đích, epoch và bố cục. Lượt cũ giữ
publicationId và đích/epoch; logout giữ provider để không fallback sang webhook. Login mới thay authorization generation để mở lại nghĩa vụ ghi bị dừng
do hết quyền. API request chung trần hai slot và nhịp tối đa một request/giây;
delivery có deadline 90 giây, đọc theo trang giới hạn 16.000 ô. Luồng backup/reset
schema 1 giữ gid0, sao lưu toàn workbook, dừng epoch cũ và xác minh lại trước khi
mở epoch mới; không áp dụng luồng này cho tab shared-v2.

Bình luận Android đọc tài khoản trong phiên điều khiển trước khi mở bài đích;
chỉ cập nhật trường handle, giữ các metadata khác. Reply giải username người được
tag tại thời điểm gửi và giữ identity của câu gốc để mở nhánh khi đọc lại.
Kịch bản dùng `device_meta.handle` và snapshot `roleBindings` trong request. Preview
trả `conversationAccounts`; tạo campaign kiểm cùng bộ validator trong transaction
SQLite, áp dụng cả lịch và Điều phối. Trước câu và tại gate Send kiểm lại metadata
của người gửi và các vai được tag; không sửa snapshot lịch sử.
Worker xác minh dùng chung `open_exact_target_by_hierarchy`, không mở URL lần nữa
trước resolver. Trên Global 45.7.3/en, resolver mở bài bằng ID trong link của
campaign qua `snssdk1233://aweme/detail/<id>` và không mở bảng Chia sẻ, vì TikTok
rất có thể tính Copy link là một lượt chia sẻ công khai (probe 09/10/2026: bộ đếm
chia sẻ từ 0 lên 1 khoảng một phút sau một lần Copy link). Trước khi gửi link,
TikTok phải ở foreground trên màn không phải trang chi tiết bài. Nếu TikTok không ở
foreground hoặc còn trang chi tiết cũ, Riviu khởi động lại TikTok một lần; vẫn còn
trang chi tiết thì từ chối mà không gửi link. Sau khi gửi, hai snapshot có generation
tăng dần phải cùng là trang chi tiết (`bj1` Back, không có tab Home, có Comments)
với cùng tác giả và đúng một caption. Feed, ứng dụng khác, "This video is
unavailable" hoặc thẻ đổi giữa hai lần đọc đều từ chối trước thao tác công khai.
Chỉ gửi lại cùng link một lần khi chưa thấy trang chi tiết. Trên một số tài khoản
(máy #32/#34, 09/10/2026) link này mở bài trong trình xem kiểu feed (có tab Home,
thanh Not interested/Interested) chứ không phải trang chi tiết. ID một mình không
chứng minh thẻ feed, vì thẻ có thể đổi. Khi hết ngân sách mà chưa chứng minh được
trang chi tiết, và chỉ trong trường hợp đó, resolver chuyển **một lần** sang proof
bằng link sao chép bên dưới cho bài đó. Bài không còn, popup giao người vận hành,
lệnh hủy hoặc lỗi gửi link vẫn dừng ngay. Evidence của Like, Lưu, Follow và bình
luận ghi `targetRoute` (`detailLink`, `copiedLink` hoặc `screenHandle` ở đường ảnh);
`copiedLink` kèm `copyLinkCountsAsShare: true`. Bước popup chung chạy trước mọi lần
đọc trên cả hai nhánh, kể cả lần mở lại trước mỗi thao tác. Hộp hệ thống Android 9
"Allow TikTok to access your contacts?" chỉ được bấm **Deny**, không bao giờ Allow.
Các bản khác chỉ còn
đường đọc, vì thao tác công khai đã bị danh sách bản được phép chặn, và giữ proof
bằng link sao chép: trong cửa sổ 30 giây, chỉ mở lại cùng link một lần nếu đọc được
Home/Profile và thẻ LIVE sau ít nhất 10 giây mà chưa có nút bình luận. Không mở
lại với sai package, bài báo không tồn tại hoặc lỗi clipboard. Android dựng lại
navigation task bằng NEW_TASK|CLEAR_TASK trong nhánh này để thoát SplashActivity
giữ intent cũ; không xóa dữ liệu ứng dụng. Một lượt quan sát tối đa 75 giây trong
ngân sách 120 giây đã lưu; lease 150 giây gồm mở phiên và cleanup. Ở nhánh này
link sao chép phải khớp trước thao tác công khai. Lượt gửi chưa xác minh không được
gửi lại.

Inspector dùng `ui_automation::inspector::ElementSelector` và `inspector_commands`.
UI và `scripts/riviu_agent_mcp.mjs` gọi cùng observe/tap/record qua Tauri hoặc
`POST /v1/inspector/{observe,tap,record,recording}`. MCP dùng URL loopback và token
API hiện có, không mở ADB server hoặc phiên driver thứ hai. Mỗi thao tác giải lại
selector duy nhất theo package; schema selector v2 hỗ trợ prefix, scope ancestor và
clickable ancestor có định danh/độ sâu/geometry hữu hạn, và `ordinal` (thứ tự 1-based
trong khung `scope` đặt tên). Ordinal chỉ được đề xuất khi không có selector thuộc tính
duy nhất, người dùng phải chọn; lúc chạy nó đọc cây mới và từ chối khi các match nằm ở
nhiều khung hoặc ít hơn N. Schema v1 thiếu các trường mới vẫn khớp exact như cũ.
Recorder ghi intent trước tap/vuốt, lưu ảnh/cây trước và sau. Nó không tự chọn node mới
sau delay: UI gọi `inspector_confirm_postcondition` với selector người dùng chọn;
backend mặc định kiểm selector đó chỉ có ở snapshot sau. `sameScreen` (bước làm mới tại
chỗ) chỉ được bật bằng lựa chọn riêng và chỉ đòi selector còn ở snapshot sau. Bước chưa
xác minh giữ unverified và không xuất Flow tự chạy. Bước clipboard của recorder đọc/ghi
qua cùng helper với `device_get_clipboard`; nội dung đọc chỉ trả về UI, không lưu vào
phiên ghi.
Flow tap dạng `selector` và Swipe dạng `fromFraction`/`toFraction` có hậu điều kiện
`elementVisible`, được kiểm cả compiler, runtime và ledger. Ledger nhận baseline `none`
chỉ khi cấu hình là tap phần tử (cùng package) hoặc swipe tỷ lệ và hậu điều kiện là
`elementVisible`; tap/vuốt theo ảnh vẫn giữ baseline ảnh/generation. `ReadClipboard`
chế độ `arm` ghi sentinel `riviu-clipboard-sentinel-<attempt>`; chế độ `read` từ chối giá
trị còn tiền tố sentinel (`ClipboardUnchanged`) thay vì ghi biến. Cả kết quả đạt và không đạt đều phải khớp selector đã biên dịch.
ElementVisible chỉ chứng minh UI; không thay proof nghiệp vụ
Đăng/Gửi, canonical link hoặc tài khoản. Dữ liệu quan sát nằm trong artifacts/inspector.

Xác minh bài ảnh đọc caption trong viewer, lấy link bằng clipboard sentinel,
đối chiếu toàn bộ caption, tài khoản và content ID với metadata oEmbed công khai.
Đây là chỗ duy nhất cần Copy link vì link của bài vừa đăng chưa có ở nguồn nào
khác. Mỗi lần Copy link có thể bị TikTok tính là một lượt chia sẻ của chính bài đó:
`verificationDiagnostic.copyAttempts` đếm số lần và `copyLinkCountsAsShare` đánh dấu
chúng là thao tác tính lượt chia sẻ trong evidence.
Nếu viewer đã ẩn caption, chỉ mở Share trên màn bài có nút Back và Comments đã
nhận diện; metadata công khai vẫn phải khớp toàn bộ nội dung. ID bài hỗ trợ đối chiếu
phiên: TikTok có thể cấp ID trước nút Đăng; mốc dưới lấy từ sự kiện `opening_app`
đã lưu của đúng campaign/máy, tối đa 30 phút trước `submittedAt`. Thiếu mốc ấy thì
chỉ dùng `submittedAt`, không tự nới khoảng thời gian. Caption, tác giả và content ID
phải cùng khớp; lỗi mạng giữ trạng thái chờ xác minh. Timestamp suy ra từ ID chỉ là
thời điểm cấp ID; riêng nó không đủ, và kể cả khi đủ điều kiện (xem định nghĩa id-allocation-time
ở phần Xác minh Publish) nó không phải giờ xuất bản.
Cửa sổ đọc clipboard và thông báo xử lý bắt đầu sau ACK của thao tác Copy;
độ trễ tìm nút/dispatch không được ăn mất thời gian quan sát. Global 45.7.3/en
ở 1080×2220 có vùng OCR thông báo riêng để tránh ghép chữ thanh tìm kiếm.
Thông báo phải mới so với ảnh trước, đúng phiên và hash; chỉ cho kết quả chờ
xử lý, không thay canonical URL, receipt hay quyền đăng lại.

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
`ServerWatch` chỉ diễn giải chính các lần đọc roster đó. Server từng có máy mà ngừng
trả lời sẽ phát `AdbServerNotice` `lost`, rồi `returned` khi trả lời lại. Trong
`OUTAGE_START_HOLD` (15 giây) keeper không mở stream mới, để tránh client adb tự khởi động
daemon tranh với công cụ đang restart. Sau đó lần mở đầu tiên của mỗi máy giãn 2→30 giây
khi lỗi. Mở view kiểm tra `get-state` trước mọi lệnh adb khác. Ghi kênh điều khiển scrcpy
(chạm/phím/keyframe) có hạn 2 giây cho cả chờ khoá lẫn ghi. Hết hạn khi chờ khoá nghĩa là
chưa gửi; hết hạn hoặc lỗi khi ghi nghĩa là không chắc đã tới máy, nên đóng socket và
không gửi lại.

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
Điều khiển trực tiếp Android gửi `DOWN` qua scrcpy ngay khi pointer-down, giữ cùng
contact tới `UP` hoặc cancel; thời gian nhấn giữ tính sau ACK của `DOWN`. Touch dùng
overlay session/ManualControl owner hiện có, không mở lease mới trên mỗi sự kiện.
Ô nhập chữ thủ công gửi một lần qua `device_type_text` hoặc `group_input(type)` sau
xác nhận Enter/nút gửi; IME composition và Shift+Enter không kích hoạt gửi.

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
`android_helper_setup` trong `state`: tối đa mười máy cùng lúc (`MAX_CONCURRENT_PREPARATIONS`),
admission và lease Repair giữ stream; riêng lệnh cài APK helper vẫn tối đa hai máy
(`MAX_CONCURRENT_HELPER_INSTALLS` trong `install_apk`). `prepare_helper_runtime` xác minh gói,
quyền sở hữu và clipboard có khôi phục. Bằng chứng clipboard được ghi bền vững
(`clipboard-proof.json` trong thư mục `helper-runtime` của máy) gắn với `boot_id`, SHA-256 APK
bundle, `ownerId`, `serviceInstance` và `ownerGeneration`; khi controller mở lại và resume đúng
owner đó trên cùng lần khởi động máy, bằng chứng được dùng lại, không đổi bàn phím lần nữa.
Lệch bất kỳ trường nào thì chạy lại vòng clipboard đầy đủ. Nhập caption vẫn kiểm chọn IME
và settlement clipboard ở từng lần dùng. Khi gặp retained owner lỗi, worker dùng chung kế hoạch maintenance với lệnh thủ công.
Mỗi kết nối hoặc episode lỗi mới có một lần chuẩn bị; contention trước dispatch
được chờ lại, kết quả chưa rõ giữ nguyên maintenance ID để đối soát. Inventory chỉ
tiêu thụ bản ghi lỗi đã quan sát, không gọi HTTP helper trong lượt quét. Preflight
gặp clipboard đang làm việc không được chốt thành helper hỏng. Chỉ runtime được
xác minh mới xóa lỗi chuẩn bị; package presence hoặc reconnect không đủ.

Tự đối soát helper (retained owner bị fence `HelperRecoveryRequired`: hồ sơ cũ từ bản
cài trước, helper do controller khác giữ, bản helper cũ) chạy tự động, không có bước xác
nhận (quyết định của chủ sản phẩm 12/10/2026). Điều kiện duy nhất là giữ được lease
Repair: không tác vụ nào của controller này đang giữ máy, nên không chen vào việc đang
chạy. Ledger không còn là cổng: `helper_auto_reconcile_note` chỉ đọc bản ghi chưa rõ hoặc
đang chạy đầu tiên của máy (`publish_assignments` ở `posting`/`verifying`/`uncertain`,
`interaction_assignments` ở `sending`/`uncertain`, `tiktok_action_runs` hoặc
`public_cleanup_runs` ở `armed`/`uncertain`, `flow_node_attempts` có hiệu ứng ở
`intentCommitted`/`effectDispatched`/`verifying`/`uncertain`, claim ghi Sheet) để nêu
trong nhật ký rằng bản ghi đó vẫn được giữ để đối soát. Supersession không lặp thao tác
nào: Post/Send/Sheet chưa rõ vẫn chỉ được quan sát, và bài đang xác minh cần chính helper
mới để lấy liên kết. Tiến trình helper vắng hoặc dừng được bằng chính kế hoạch
maintenance. App chạy đúng luồng của nút (`pending` → `prepare` → `execute`, không
`reconcileOnly`, rồi qualify phiên mới), giữ `recordsRetained`/`archivedUnresolved`, ghi
một dòng `agent.helper.auto_reconcile` ("tự đối soát helper: không có thao tác chưa rõ —
nhận phiên mới", hoặc nêu bản ghi vẫn được giữ). Điểm chạy: worker kết nối (khởi động
controller, cắm lại, reboot) và admission của Nuôi, Tương tác, Điều phối Nuôi và
preflight Đăng bài, mỗi máy tối đa một lần mỗi admission, không lặp. Preflight Đăng bài
vì vậy có thể dừng helper cũ của máy rảnh. Kế hoạch maintenance vẫn tự từ chối khi không
chứng minh được (IME ban đầu mất, APK đổi nguồn, tiến trình còn khi đối soát); khi đó nút
**Khôi phục helper** là đường dự phòng. Máy reboot hoặc mất tiến trình helper được coi là
phiên cũ đã mất, không phải chưa rõ.

Chính driver áp dụng quy tắc đó, kể cả trong lease của tác vụ (lượt tự khởi động lại khi
quá tải, `device_load::reboot_for_job`), không chờ lease Repair. Helper chỉ giữ chủ phiên,
token và epoch trong bộ nhớ (`OwnerSession.java`), nên chủ phiên được giữ là đã mất khi có
một trong các bằng chứng sau (`runtime.rs::owner_restart_proof`):
- boot id hiện tại khác boot của đúng epoch đó (`bootId` ghi trong `runtime-owner.json` lúc
  nhận phiên; hồ sơ cũ dùng `clipboard-proof.json` cùng owner/instance/generation);
- `pidof com.riviu.agent` trống;
- pid khác `helperPid` đã ghi khi phiên được chứng minh.

Khi đó `settle_restarted_owner` chuyển `runtime-owner.json` và `release-<owner>.json` (kể cả
`releasePending`) vào `restarted/<ownerId>/` kèm `receipt.json` (bằng chứng, hash, không
RELEASE, không lặp thao tác), rồi nhận phiên mới. Nó chạy ở `ensure_runtime` trước khi
settle release, và ở client đang cache trước reconnect hoặc RELEASE lặp lại
(`retire_if_restarted`). Đọc không được, `ime-checkpoint.json` còn, có
`maintenance-active.json` hoặc hồ sơ khác epoch thì không có bằng chứng và fence cũ giữ
nguyên.

adbd chạy root (`id -u` = 0) được tự sửa, không có bước xác nhận (quyết định của chủ sản
phẩm 12/10/2026): `agent_commands::auto_unroot_adb` chạy ở worker kết nối (trước
`prepare_helper_runtime`) và ở admission trên (trước kiểm ledger), dưới lease Repair nên
không chen vào việc đang chạy của controller này. `AndroidDriver::repair_root_adb_shell`
gửi `adb -s <serial> unroot` tối đa một lần cho mỗi boot id của máy (mốc đã dùng lưu bền ở
`driver-state/adb-unroot/` trước effect nên khởi động lại app không cho thêm lần nào, chỉ
khởi động lại máy mới cho; chưa đọc được boot id thì không gửi và báo tắt GenFarmer, khởi
động lại máy), ghi `agent.helper.adb_unroot.started` trước effect,
chờ hữu hạn (45 s) đến khi `id -u` = 2000, không bao giờ restart adb-server của PC hay chạy
helper bằng root. Không phụ thuộc ledger: lệnh chỉ restart adbd của máy, không chạm trạng
thái TikTok và không lặp thao tác. Vẫn root thì giữ `shell_root` với lý do tiếng Việt (tắt
GenFarmer, khởi động lại máy). Lúc nhận phiên, fence `HelperRecoveryRequired` chưa phân
loại được thăm dò danh tính adbd (chỉ đọc) để chốt đúng `shell_root`/`shell_uid`/
`android_user` thay vì `recovery_required`; readiness mang theo dòng chốt đó để preflight
nêu cùng nguyên nhân với ô máy.

Đóng app không bị giữ vô hạn: drain có deadline từng bước; drain không settle (ví dụ máy
đã rời adb nên không chứng minh được helper vắng) vẫn thoát, giữ nguyên journal helper để
lần mở sau tự đối soát theo luật trên.

Inventory dùng dữ liệu máy đã đọc khi cùng máy đang giữ khóa chuẩn bị helper; lỗi
scan Android không được coi là disconnect để cấp lại lượt cài. Gói `com.riviu.agent`
có versionCode tối thiểu 5 cho launcher; APK, manifest và NOTICE phải khớp hash.

## Start Publish và handoff máy đã chọn

### Vào và phục hồi

- Preflight chỉ quan sát readiness và owner; không dừng thao tác hay đóng TikTok.
  `needsRelease` liệt kê máy đã chọn và phạm vi owner mà xác nhận sẽ nhả.
- Preflight và Start dùng chung một request ID dự định. Bị chặn do acceptance mode hoặc
  owner không tách được đều làm `canExecute` false. Một owner không nhả được thắng mọi
  hàng nhả được khác trên cùng máy.
- Report hết hạn (30 giây) không chặn nút xác nhận: Start chạy lại preflight một lần cùng
  request ID, kể cả khi có pending marker chưa có receipt, rồi áp đúng luật so sánh review
  (`reviewChanges`); khác hoặc `canExecute` false thì dừng trước effect. Receipt failed không có campaign là từ chối
  bền; retire xóa pending marker local và phát cho cả trang lẫn progress monitor. Lỗi
  storage không phát retire. Receipt null không bao giờ có nghĩa là bị từ chối.
- ID pending lịch sử không có receipt có nút **Hủy yêu cầu chưa được tiếp nhận** trên
  progress monitor. Nút hủy vĩnh viễn ID đó, không hủy campaign. Một transaction
  IMMEDIATE trả nguyên start receipt đã có, hoặc nhận create receipt đã có thành uncertain
  cùng campaign của nó; chỉ khi không có cả hai mới ghi tombstone failed
  `cancelledBeforeAcceptance`. Fingerprint sentinel cố ý không bind body chưa biết.
- Start và đường tạo legacy kiểm cùng receipt bền trong transaction ghi. Start/create
  trễ cùng ID hoặc thắng trước khi hủy, hoặc thua tombstone. Sau khi hủy đã xác nhận,
  operator phải preflight ID mới.

### Thay thế đã xác nhận

- Đăng ngay kiểm cả assignment đang xếp hàng lẫn owner sống và guard Post/link. Tạo lịch
  không ngắt việc đang chạy.
- Loại theo từng assignment đã chọn. Đường loại chung thu hồi composer và verifier link
  có guard của nó. Nuôi dừng theo từng máy đã chọn. Thao tác không chia được có máy ngoài
  vùng chọn bị từ chối trước khi thu hồi.
- Marker handoff-pending được ghi atomic cùng lần loại, nên settle không nhả account
  reservation của assignment trước khi đóng app thật.
- Handoff chờ dispatch job đang chạy, claim của assignment, device work owner và trạng
  thái Nuôi của máy đã chọn drain hết. Đóng app thật dùng giới hạn chung hai máy và
  deadline hiện có; không tăng giới hạn.
- Start có hai ngân sách handoff riêng, mỗi cái 125 giây (`HANDOFF_BUDGET`): chờ tới lượt
  handoff tính từ lúc tiếp nhận; nhả máy tính từ lúc gọi `prepare_publish_devices`, sau
  preflight duyệt lại. Khi hai bước còn dùng chung một hạn (13/10/2026), preflight duyệt lại
  30 máy tiêu ~100 giây, nên 6 máy cuối hết hạn ngay khi tới lượt đóng.
- Máy không nhả được trước dispatch không làm hỏng cả lượt. `unreleased_publish_devices` trả
  các máy `closed=false` kèm lý do. Preflight sau nhả chỉ bỏ qua issue gắn đúng các máy đó;
  issue khác vẫn từ chối cả lượt và digest vẫn so trên toàn bộ tập đã duyệt.
  `create_publish_holding_back` tạo assignment của các máy đó ở `failed_before_dispatch`,
  `error_code = publish_device_not_released`, trong cùng transaction tạo campaign. Claim
  campaign (`claim_publish_pipeline`, `enqueue_publish_jobs`) không xếp hàng các hàng này, kể
  cả khi thử lại toàn campaign; chỉ `publish_retry_assignment` (**Thử lại máy này**) xếp lại và
  xóa mã. Không máy nào nhả được, hoặc kết quả nhả thiếu/trùng hàng, thì Start vẫn từ chối
  trước khi tạo campaign. Không force-drop lease, không thu hồi Repair, không gửi lại Post.
- Closer độc quyền kiểm lại quyền của assignment, đóng app, bỏ context độc quyền rồi lưu
  release proof riêng của assignment. Proof bind lần loại mới nhất, máy, state, revision,
  Post intent và evidence; assignment đổi hoặc resume xác minh tường minh mới hơn làm
  proof mất hiệu lực. Chỉ account reservation của assignment đó bị xóa; link chưa giải
  quyết trở thành nợ review, không phải owner máy sống.
- Thay thế không dừng publish toàn campaign. Assignment cùng campaign không được chọn
  không bị loại hay đóng. Caller handoff thủ công cũ giữ hành vi cũ.
- `require_current_preflight_digest` không đổi. Readiness mới được kiểm sau khi nhả;
  invalidate cache không nới duyệt nội dung nguồn hay mapping đích.

### Digest đã duyệt

- `publish_preflight_digest` (SHA-256) bind: request (máy, bài, caption override,
  `soundPolicy`, `sheetEnabled`, `deleteAfterPublish`, `runAt`, `targetRef`), toàn bộ
  `PublishBundle` sau override (đường dẫn, cỡ và SHA-256 từng file, caption), snapshot
  đích (số máy, alias, `rosterSha256` của cả fleet), và mỗi máy: số, alias, **tài khoản**
  (`DeviceMeta.handle` đã chuẩn hóa, từ 10/2026), package/version/locale TikTok, ngưỡng và
  kết luận dung lượng, cùng `sheetDelivery` (spreadsheet, gid, phiên bản). Không bind: số
  byte trống, mtime, bài nhạc cụ thể (chọn lúc chạy theo `soundPolicy`), chủ giữ máy. Chủ giữ
  máy được duyệt riêng bằng phong bì nhả máy bên dưới, vì nhả máy đã duyệt cố ý đổi chủ.
- Mỗi dòng `needsRelease` có `envelope`: máy, loại chủ và đúng việc đang giữ: id thao tác (Nuôi
  theo mã lượt), lượt đăng kèm từng assignment và revision (`publish:{máy}:{lượt}:{assignment@revision,…}`),
  hoặc thế hệ lease khi chủ không có thao tác nào được liệt kê. Điều khiển trực tiếp và dọn nền
  là tạm thời, không thuộc phong bì. Start gửi kèm phong bì đã xem (`approvedRelease`). Chủ đã
  đi thì không cần duyệt; chủ mới, việc khác trên cùng máy hoặc revision đã đổi làm Start bị từ
  chối mà không dừng chủ nào chưa được duyệt.
- Thứ tự kiểm của Start (`publish_create_campaign`): (1) replay đúng request ID và
  fingerprint, trả receipt cũ, không dừng/tạo lại; (2) kiểm request, khóa tài khoản các máy nhận
  (Start đã khóa ngay lúc tiếp nhận, xem "Khóa tài khoản theo bản duyệt"), rồi vào khóa handoff
  (chỉ chờ); (3) `require_approval_before_effects`: preflight chỉ đọc bằng builder gốc (không
  reconcile helper) trong context mới: đo lại build, dung lượng và đọc lại tài khoản, không dùng
  lại báo cáo đã chuẩn bị (`reuse_prepared` chỉ phục vụ màn xem lại, context không `fresh`); so
  digest đã duyệt rồi so các chủ đang giữ với phong bì; lệch thì receipt thành từ chối, không đổi
  chủ nào. Đây chỉ là từ chối sớm; (4) `defer_idle_work`, rồi `prepare_publish_devices`:
  `fence_new_work` chặn mọi việc mới hoặc lấy lại lease trên các máy đó qua bộ điều phối chủ
  sở hữu (chỉ ManualControl của chính lượt nhả được lấy máy), một lần quan sát dưới hàng rào
  thành **kế hoạch nhả bất biến**, so với phong bì, `publish_account_lock_drift` xác nhận khóa
  tài khoản của chính thế hệ chuẩn bị này còn hiệu lực và tài khoản vẫn đúng như lúc khóa, rồi chỉ kế hoạch đó
  được thực hiện: loại
  assignment theo đúng revision đã duyệt (CAS), dừng Nuôi bằng `stop_run` so mã lượt dưới khóa
  của runtime (lượt Nuôi khác thì từ chối, không dừng), dừng thao tác theo id; không có lần liệt
  kê sau nào thêm việc để dừng. Hàng rào giữ tới khi đóng máy xong; các kiểm tra lúc đóng vẫn
  từ chối nếu có chủ mới; (5) preflight mới sau khi nhả, `canExecute` và digest trước staging,
  chuyển media, Post. Kiểm lại ở UI trước khi gửi chỉ giúp báo sớm, không thay các bước này.
- Lượt tạo mới mang `approval` bất biến trong `request_json`: digest đã duyệt và tài khoản đã
  chuẩn hóa của từng máy nhận. Một bộ kiểm (`publish_approval_refusal`, cả tập máy nhận chưa qua
  biên hiệu lực) chạy trước claim của bộ hẹn giờ và trước claim của `execute_pipeline` (chạy tay,
  lịch qua điều phối, thử lại); `admit_publish_reconnect` kiểm lại khi nhận việc từ hàng đợi.
  Biên hiệu lực thật là `admit_publish_effect`: sau khi đã giữ lease thiết bị và qua mọi lượt chờ
  (staging, lease, UI capacity), ngay trước chuyển media, dừng/mở TikTok hay composer, một
  transaction ngắn so bản duyệt với tài khoản hiện tại của mọi máy còn lại (không giữ transaction
  qua I/O thiết bị). Kết quả kiểm còn đúng sau khi trả về là nhờ khóa tài khoản của cả tập (mục
  dưới), không phải nhờ phép so này; phép so là lớp chặn sau cho thay đổi đi vòng qua các đường
  ghi. Lệch ở bất kỳ máy nào
  thì toàn bộ việc còn lại chưa qua biên hiệu lực chuyển cần xem lại (`failed_before_dispatch`,
  `publish_approval_changed: lý do`, job đang chờ kết thúc), và lỗi là `Terminal` nên không thử
  lại. Lượt không có `approval` (tạo trước 10/2026) cũng vậy; thử lại vẫn so với cùng bản duyệt
  nên không "hợp thức hóa" thay đổi. Recovery lấy `expectedAccount` từ bản duyệt; bằng chứng tài
  khoản trên điện thoại trước Post vẫn là lớp chặn sau. Hàng đã gửi hoặc chưa rõ chỉ đối chiếu,
  không đăng lại; giữ nguyên không chiếm quyền, ngân sách thử lại và đối soát link/Sheet. Nội dung
  của lịch là bản sao managed đã kiểm hash lúc tạo và kiểm lại trước chuyển; sửa file nguồn sau
  đó không đổi thứ được đăng (nội dung được chốt khi lưu lịch).

### Khóa tài khoản theo bản duyệt

- Từ lúc bản duyệt được chấp nhận (Start, hoặc lưu lịch/tạo lượt con cho lượt hẹn giờ) đến khi
  mọi assignment của tập đó xong, mọi máy trong tập giữ đúng tài khoản đã duyệt. Mọi đường ghi
  tài khoản (`set_device_handle`, đối soát tài khoản, nhập/lưu metadata, tài khoản quan sát từ
  tương tác) gọi `ensure_account_edit_allowed` trong transaction ghi của chính nó và từ chối, nêu
  tên lượt: "tài khoản đang được giữ cho lượt đăng … đã duyệt và chưa xong; hủy lượt đó hoặc chờ
  nó xong rồi mới đổi tài khoản". Ghi lại đúng tài khoản cũ (khác cách viết) không phải thay đổi.
  Máy đang chạy một bước đăng (job `running` hoặc work claim) vẫn giữ tài khoản tới khi bước xong.
- Khóa là dòng bền trong `settings`, khóa `publish.account-lock.{loại}.{khóa}.{máy}`, giá trị JSON
  gồm máy, chủ, loại, tài khoản lúc khóa và tên hiển thị:
  - `preparing` (khóa theo **thế hệ** của một lượt chuẩn bị, chủ là request ID): tạo trong cùng
    transaction tiếp nhận Start (`accept_publish_start`), hoặc khi tạo lượt không qua Start bắt
    đầu (`lock_publish_preparation_accounts`), luôn trước khi đọc tài khoản để so bản duyệt. Mỗi
    thế hệ có một chủ trong tiến trình (`PublishPreparationOwner`), được đăng ký **trước** lần ghi
    khóa; ghi khóa từ chối nếu chủ đã mất. Khóa chỉ có hiệu lực khi chủ đó còn sống và Start (nếu
    có) chưa kết thúc và chưa nối lượt. Chủ mất ở mọi lối ra (lỗi, future bị hủy kể cả khi lệnh ghi
    đã commit, xóa dòng thất bại, Start không ghi được kết quả), nên khóa không bao giờ còn hiệu lực
    mà không có chủ đang chạy; dòng còn sót được thu lại ở lần ghi tài khoản kế tiếp hoặc lúc mở
    app. Mỗi lượt chỉ xóa đúng thế hệ của mình, nên hai lần gọi cùng request ID không xóa khóa của
    nhau. Lượt tạo không thay khóa này; khóa `campaign` giữ máy từ lúc insert.
  - Start ghi thế hệ chủ của mình (`publish.start-owner.{request}`) cùng transaction tiếp nhận;
    chủ sống tới khi trạng thái kết thúc được ghi (thử lại qua storage tối đa 8 lần, khoảng 45
    giây). Start còn `accepted`/`preparing` mà chủ đã mất được chuyển `uncertain`
    (`PublishStartInterrupted`), như khi khởi động lại, ở lần đọc trạng thái Start, lần ghi tài
    khoản kế tiếp hoặc lúc mở app; trạng thái đó cần người kiểm, không bao giờ được claim hay đăng
    lại.
  - `campaign` (chủ là lượt): tạo trong transaction insert lượt có `approval`; transaction đó so
    tài khoản hiện tại của từng máy với bản duyệt và từ chối tạo lượt nếu lệch. Khóa lại khi việc
    mới quay về tập (claim, thử lại assignment, recovery xếp lại). Còn hiệu lực khi lượt chưa hủy
    hoặc lỡ lịch và còn hàng chưa có effect intent đang ở trạng thái chạy hoặc có job đang chờ/chạy.
    Hàng đã gửi/chưa rõ chỉ đối chiếu theo bản duyệt nên không giữ khóa; hàng
    `failed_before_dispatch` không còn job thì đã xong cho tới khi được thử lại.
  - Dòng hết hiệu lực bị bỏ qua, và bị xóa ở lần ghi tài khoản kế tiếp trên máy đó, khi pipeline
    kết thúc, khi hủy lượt, khi bản duyệt bị từ chối, khi Start kết thúc và ở lượt quét lúc mở app
    (lượt quét này cũng khóa các lượt có `approval` còn việc, kể cả lượt tạo trước khi có khóa).
- Các phép so vẫn chạy làm lớp chặn sau cho thay đổi đi vòng qua đường ghi:
  `publish_account_lock_drift` trước lần loại assignment hoặc dừng đầu tiên của Start,
  `publish_approval_refusal` trước claim và `admit_publish_effect` trước hiệu lực.

## Xác minh Publish và kết quả qua restart

VerificationQueue giữ một observer mỗi máy, không dùng trần observer toàn máy chủ.
Cleanup media chạy worker riêng; một máy đọc chậm không chặn dispatcher hoặc Sheet.
Candidates được phân trang hữu hạn, ưu tiên bài tới hạn và luân phiên giữa các máy.
Nhãn thời gian own-post chấp nhận EN và VI (`N phút trước` / `vừa xong`).
`evidence.verificationStatus` giữ reasonCode, attempts, readFailures, checkedAt và
nextCheckAt. Bài còn thiếu link được kiểm lại khoảng 2 phút (120 giây) sau cuối lượt trước khi còn
ngân sách; đây không phải hạn tổng và không mở lại TikTok mỗi 2 phút. `verificationBudget` version1 lưu fingerprint gắn immutable intent,
publicationStage, observations và noProgressObservations trong cùng CAS với kết quả.
Candidate metadata đúng schema/assignment/intent/bundle/account/caption/time được
kiểm qua mạng với `metadataObservations`, tối đa `metadataLimit=12` mỗi đợt xác minh.
Các lượt này giữ nguyên bộ đếm tìm bài; lỗi đọc thông thường giữ bộ đếm metadata.
Hết giới hạn metadata lưu cause `metadataRetryBudgetExhausted` và giữ candidate.
Mọi lần gọi oEmbed trong tiến trình đi qua một cổng chung
(`tiktok_share/photo_proof/oembed_gate.rs`): một yêu cầu một lúc, các lần bắt đầu cách nhau
ít nhất 5 giây; sau 429/5xx/timeout/lỗi kết nối thì lùi từ 30 giây, gấp đôi mỗi lần bị chặn
liên tiếp, có jitter, trần 10 phút, và tôn trọng `Retry-After` trong trần đó. Lượt phải chờ
cổng quá 20 giây thì không gửi và ghi như yêu cầu không được trả lời (stage `request`); Stop
huỷ được cả lúc đang chờ. Đo 14/10/2026 13:45–14:10 trên một máy chủ 20 máy: trang chủ TikTok
200 trong 0,3 s, link ngắn 301 ngay, riêng `/oembed` treo 15–20 s hoặc 429/503, thỉnh thoảng
200 với cùng link. Lượt TikTok không trả lời (stage `request`, `bodyRead`, `bodyLimit`,
`responseParse`, hoặc `httpStatus` 429/5xx) không tiêu `metadataLimit`: budget đếm ở
`metadataUnreachableObservations` và giới hạn theo thời gian, 120 phút
(`METADATA_UNREACHABLE_MINUTES`) tính từ `metadataUnreachableSince`, lần đầu không được trả
lời trong budget này. Hết hạn lưu needsReview cause `publicMetadataUnreachable`, giữ
candidate và không đăng lại. Dữ liệu lệch (caption/tác giả/ID) và các 4xx khác vẫn kết thúc
như trước.
Viewer mở từ feed (bước video và ảnh mở rộng) không chứng minh được bài là của mình nên chỉ
xác minh qua metadata công khai. Khi metadata ấy không trả lời (hết giờ, 429/503, lỗi
request) hoặc bảng Chia sẻ của viewer không có Copy link, lượt quan sát không dừng mà sang
lưới Hồ sơ của chính tài khoản: tài khoản trên header, caption đầy đủ, nhãn thời gian sau
`submittedAt`, Share → Copy link có sentinel và redirect chuẩn
(`ownProfileCaptionAndCanonicalLink`). Từ lần metadata không trả lời đầu tiên, lượt đó không
Copy qua đường công khai nữa. Proof hồ sơ ra content ID khác candidate đang giữ là hai bài
cùng khớp (`multipleMatchingPosts`), không xác minh. Hồ sơ không chứng minh được thì kết quả
vẫn là `metadataUnavailable` và candidate đi tiếp đường metadata. Với candidate đã giữ mà lần
tra gần nhất là lỗi khả dụng, verifier vẫn tra mạng trước; còn lỗi khả dụng và máy rảnh thì
lấy claim `verify` của máy (loại trừ observer metadata của cùng bài), mở phiên bằng
`open_manual_session` với cùng guard restart (không restart khi upload chưa chứng minh xong)
và chỉ nhận link đúng content ID của candidate. Không chứng minh được thì ghi lượt metadata
như cũ, kèm `phoneRoute` trong chẩn đoán; máy bận/offline chỉ tra mạng; candidate bị metadata
từ chối không đi đường máy.
Caption dài trên trình xem mở từ ô Hồ sơ Global45.7.3/en bị gập (đo r117 14/10/2026, máy 47:
`desc` hiện 150/207 ký tự) và bấm vào nó mở trang ảnh mở rộng (`rey` caption đầy đủ, `red`
Share, không có nhãn thời gian), nên không ảnh chụp nào có đủ caption và thời gian. Khi đó
proof nối hai ảnh chụp bằng chính lần bấm đó: trình xem ô có phần đầu caption khớp và nhãn
thời gian trong khung Đăng, rồi trang mở rộng có caption khớp chính xác. Copy link trên trang
đó vẫn phải ra đúng tài khoản, và ID bài phải được cấp trong khung chuẩn bị đã ghi
(`validate_photo_identity`). Nhãn theo giờ/ngày ("· 1h ago", đo r119 máy 46 khi lộ trình máy
tới bài 1–2 giờ sau Đăng) không đặt được cả khoảng làm tròn sau lần Đăng; khi nhãn không loại
trừ lần Đăng (thời điểm tạo muộn nhất có thể không trước `submittedAt`) thì chỉ cho phép Copy,
và cùng phép kiểm ID đó mới chứng minh thời gian. Nhãn phút giữ quy tắc cũ. Trước khi duyệt lưới,
mỗi lượt bấm lại tab Profile một lần để TikTok tải lại lưới (r118 máy 47: 90 phút sau Đăng ô
mới nhất vẫn là bài 12 giờ trước), chờ lưới đổi tối đa 5 giây rồi chứng minh lại tài khoản.
Bảng Chia sẻ Global45.7.3/en (đo 14/10/2026, máy 36 và 44) có `Bottom sheet` với header
`tv_title` "Send to" và `l86` "Close"; biến thể "Send to" không có dòng Copy link. Bảng do
lượt này mở mà không có Copy link được đóng bằng Close rồi đọc lại; bảng còn mở từ lượt
trước là màn đã biết, không phải hộp thoại lạ, và được đóng bằng Close ngay khi phiên xác
minh mở, trước guard restart và phép đọc tài khoản (Close không chạm upload). Không bấm ô
liên hệ hay ứng dụng nào trong bảng.
Sau 8 lượt liên tiếp không có bằng chứng mới của đúng bài, lưu needsReview với cause
`verificationNoProgress`, nextCheckAt=null và giữ intent/media/khoản thiếu Sheet.
Lỗi đọc/transport và tìm kiếm đều tiêu ngân sách hữu hạn này; offline/busy
chưa được quan sát không tính là một lượt. Lượt cũ chưa có budget bắt đầu từ lần quan
sát đầu của contract mới, không lấy attempts lịch sử làm số lượt không tiến triển.
Lượt Copy không ra link vì TikTok còn giữ upload không tính vào 8 lượt: reasonCode
`tiktokProcessing`, hoặc `clipboardUnchanged` khi chẩn đoán cùng lượt ghi
`appRestart.reason = uploadNotProvenSettled`. Các lượt này chờ trong hạn
`PROCESSING_WAIT_MINUTES = 60` tính từ `submittedAt` (hoặc từ resume, lưu ở
`verificationBudget.processingWaitFrom`). Đây là trần, không phải mục tiêu: máy 23
(ce031713cd92d90701, SM-G955F, Global 45.7.3 en) ngày 13/10/2026 còn báo “Post is
being processed” sau hơn 36 phút. Hết hạn, lượt như vậy lưu needsReview ngay với cause
`processingWaitExhausted`. Không đăng lại, không restart TikTok khi upload chưa settled,
và toast không phải proof. `clipboardUnchanged` không kèm chẩn đoán đó vẫn tính vào 8 lượt.
Stop chờ primitive đã được cấp quyền hoàn tất, không huỷ giữa khôi phục IME.
AccessibilityReadUnavailable chỉ dành read-only sau Android recovery; sound observer
cho một lần đọc bổ sung/phase trong ngân sách và xóa snapshot cũ trước readback.


`Submitted` chỉ chứng minh thao tác Đăng đã qua biên hiệu lực và TikTok trở lại feed.
`Verifying` giữ media và nhịp xác minh (Android chỉ restart TikTok khi upload đã được chứng minh xong); `Succeeded` đòi canonical link, đúng tài khoản
đã đọc trước Post, caption và bằng chứng thời gian theo định nghĩa id-allocation-time dưới đây. `effect_intent`
giữ `expectedAccount`/`submittedAt` để phép xác minh sau restart không dựa đồng hồ lúc
retry. Thiếu bằng chứng không được cold-start hoặc bấm Post lại.

**Bằng chứng thời gian của Đã xác minh (id-allocation-time evidence).** Thời điểm mà sản phẩm
đòi cho một bài là: content ID chuẩn của bài được TikTok cấp **trong cửa sổ của lượt này** (sau
`prepared_at` của lượt: mốc `opening_app` đã lưu của đúng campaign/máy, tối đa 30 phút trước
`submittedAt`; thiếu mốc ấy thì từ `submittedAt`), **trên đúng tài khoản dự kiến** đã ghi trước Post
và **với caption khớp** toàn bộ. Đây là bằng chứng thời điểm cấp mã bài, không phải dấu thời gian
xuất bản: UI, Sheet và tài liệu không được trình bày nó như giờ TikTok công bố bài. Route hồ sơ
(`ownProfileCaptionAndCanonicalLink`) còn đọc nhãn thời gian tương đối sau `submittedAt`; các route
metadata công khai dựa vào thời điểm cấp ID cùng tác giả, caption và content ID. Bài thỏa định nghĩa
này là Đã xác minh; UI không hạ các bài ấy. Ngược lại, bản ghi tự mâu thuẫn (cờ đã xác minh nhưng
`captionMatched=false`, `submissionTimeMatched=false`, thiếu tài khoản đã ghi trước Post, tài khoản
trong evidence hoặc account diagnostic khác tài khoản frozen trong effect intent, hoặc tác giả trong
link khác tài khoản đó) không được hiện là Đã xác minh. Effect intent là nguồn tài khoản chính; so
handle đã chuẩn hóa (bỏ `@`, khoảng trắng, không phân biệt hoa thường).
Trong backend, "đã xác minh" là `succeeded` + `post.publicationVerified` + `postUrl` chuẩn. `checkLink.reason = "alreadyVerified"` và `evidence_has_post_link` chỉ kiểm hình dạng link, nên UI không bao giờ coi chúng là bằng chứng.

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
Copy dùng hitbox cùng cây; đường direct thử tối đa hai sentinel độc lập trước khi
trả lỗi. Đây không phải định danh/chống Copy lại cùng bài qua mọi fallback. Các
đường photo/video/direct cùng dùng tối đa24Copy trong một lượt (ngân sách bằng
12ứng viên x2, vẫn giữ ngân sách thời gian và ambiguity scan).
Hết Copy budget là chưa xác minh, không biến thành mismatch hoặc bỏ qua ambiguity.
Diagnostic lưu tuple, generation, reason, chi phí và `publicationEvidence`: caption
đầy đủ khớp, rồi caption+thời gian của cùng snapshot/ứng viên khớp. Hai mức proof mới
chỉ reset bộ đếm một lần mỗi mức, không chốt bài; fingerprint không dùng generation,
clock, Copy count hoặc caption của bài bị loại. Thay nhãn thời gian/lý do không reset.
Snapshot thành công thuộc đúng bài được nhận; timeout không trả ứng viên còn mơ hồ.

Nhãn thời gian own-post Global45.4.3/en dùng `:id/zj1`, đo trên hai máy và bài mới
trong [hồ sơ lịch sử](../archive/README.md). Global45.7.3/en dùng `:id/zwj`, đã đối chiếu bốn bài thật;
các đường còn lại giữ `:id/tv_post_time`. Caption đúng và nhãn phút mới có thể
chờ ngay trên bài tối đa65giây tới cửa sổ phân biệt được thời điểm; đọc lại cả
caption/thời gian sau chờ, không nới điều kiện interval-after-submission. Phép đọc phải có đúng một nhãn thời gian
và caption hợp lệ; đổi ID không nới khoảng thời gian hoặc nhận caption của bài cũ.
Global45.7.3/en có caption rút gọn kết thúc bằng ellipsis rồi U+2060 (WORD JOINER).
Verifier chỉ bỏ đúng một U+2060 cuối sau `…` khi xét prefix để mở rộng caption;
không bỏ ký tự trong caption đầy đủ, không dùng prefix làm proof và không áp sang
build khác. Sau mở rộng vẫn đọc caption đầy đủ và giữ proof tài khoản/thời gian/link.
Hồ sơ Global46.2.1/en dùng chung `:id/cover` cho bài đăng và bản nháp. Khi lấy link,
đọc badge nháp `:id/zq_`, loại cover chứa badge trước khi chọn ứng viên. Lỗi đọc
badge phải dừng trước tap cover; mở nháp rồi Back không bảo đảm quay về lưới hồ sơ.
Ô cover được cắt theo mép trên của thanh Home/Profile đang quan sát, kể cả khi
accessibility vẫn đánh dấu ô phía dưới là visible. Cả điểm chạm và điểm vuốt phân
trang dùng phần còn hiển thị đó. Bài có banner TikTok báo đã bị gỡ được bỏ qua trước
khi mở Share; không dùng nút chia sẻ của bài đã bị gỡ để tìm link bài khác.

`publish_commands/verification.rs` chạy phiên xác minh trong control plane; DB CAS ghi
proof, outbox và snapshot cùng transaction. `verified_cleanup.rs` xử lý riêng media
đã nhập vào điện thoại theo `importId`, chỉ sau proof bài và canonical URL. Mọi
chiến dịch Đăng ngay/Hẹn giờ mới yêu cầu đích Google Sheet được preflight xác minh
và bật dọn bản chuyển; UI không nhận lựa chọn tắt hai nghĩa vụ này. Chính sách
Sheet/dọn media của chiến dịch cũ bất biến theo snapshot đã lưu, không áp mặc định
mới hồi tố. Không xóa tệp nguồn trên desktop; lỗi dọn được thử lại độc lập qua
restart, không phát lại Post. Driver Android `cleanup` xoá album, dòng MediaStore theo
`_id` và thư mục staging `.riviu-publish/<scope>` suy từ chính `importId`
(`riviu-<scope>-<12 hex>`); id không đúng hình dạng này bị từ chối.
Worker dọn chỉ nhận bài `succeeded`, nên bài `failed_before_dispatch`/`cancelled`/
`missed` để lại media (đo 14/10/2026: 12–30 thư mục mỗi máy). `publish_commands/
media_sweep.rs` quét sót khi máy rảnh: cổng `app_completion_block_reason` + claim
`cleanup` + lease `IdleSweep` không chờ, giữ stream; liệt kê qua driver
(`list_publish_media_leftovers`, chỉ trả tên hợp lệ shell-id đúng hình dạng Riviu);
scope còn sống = assignment chưa terminal hoặc `succeeded` chưa có proof dọn
(`live_publish_media_scopes`); mỗi lượt xoá tối đa 8 thư mục, mỗi máy 10 phút một
lượt, 2 máy mỗi tick; mỗi thư mục có intent `claimed` rồi receipt `cleaned`/`failed`
trong `publish_media_sweeps`, tên lỗi hoặc claim mất receipt được bỏ qua 24 giờ.
Không chạy dưới scope dev-acceptance; không mở phiên UI, không đóng app.
Worker lưu `nextCheckAt` bằng thời điểm kết thúc quan sát cộng 120 giây cho bài
còn ngân sách, kể cả lỗi đọc. Mỗi lần quan sát có deadline và lease riêng. Restart
giữ mốc gửi, lần kiểm tiếp, fingerprint và số lượt không tiến triển; needsReview
không được background tự mở lại. Nhật ký verifying chỉ nói kiểm thao tác Đăng đã có;
`checking_existing_post_link` ghi lần kiểmN, `link_pending` hiện lý do thiếu link,
`link_needs_review` nói tự kiểm đã dừng. Không suy submitted/công khai từ thiếu link.
Android ở trạng thái Connected sau restart vẫn được kiểm khi agent đã sẵn sàng;
worker mở session qua control plane, không đòi mở điều khiển bằng tay để đổi
trạng thái thành Ready. Máy Busy/Preparing/Error/offline vẫn chờ; iOS giữ điều kiện Ready.
Receipt `submitted` chỉ chứng minh biên Post, không chứng minh upload đã xong: lượt
xác minh đầu vẫn chạy ngay nhưng chỉ quan sát, giữ TikTok chạy. Với receipt Android
`submitted`/`posted` chưa có link, `verification_restart` chỉ tắt đúng package đã ghi
trong intent khi `verificationBudget` của đúng intent đó đã có `publicationStage` 2 (một
lượt trước đọc được caption đầy đủ và thời gian gửi trên cùng snapshot bài; với bài ảnh
Global 45.7.3/en, snapshot viewer ảnh mở từ ô hồ sơ trước Copy cũng tính khi `desc` khớp
đủ và `zwj` nằm trong cửa sổ gửi, không tap thêm; viewer ảnh mở rộng và Trill 38.3.2
chưa đo được cả hai trên một snapshot nên không ghi) và hai
snapshot mới cùng phiên cho thấy màn đã đo, không có composer/upload/ProgressBar.
Thiếu một điều kiện thì giữ TikTok chạy, ghi `appRestart.state=keptRunning` và lượt
vẫn chờ quan sát; không chuyển lỗi, không Post lại. Khi được phép, kiểm proof hết tiến
trình, mở lại và kiểm tiến trình đang
chạy trước Copy. Cùng lease giữ suốt chu kỳ; kiểm revision/stop trước và sau các await,
không đóng máy còn bài khác đang giữ. Proof restart ghi vào `verificationDiagnostic.appRestart`.
Android hẹn giờ bắt đầu kiểm khi phiên đăng nhả máy; lỗi restart cũng giữ nhịp 120 giây.
Unknown receipt và iOS giữ đường đọc không restart. LinkAndSheet không quay lại Post.
Riêng Global 45.7.3/en, receipt `post_uncertain` chỉ được restart một lần cho đúng intent
khi lượt Copy trước đó đã ghi `tiktokProcessing` trên bài ứng viên, tài khoản trước Post đã
được xác minh, hai snapshot mới cùng phiên cho thấy viewer ảnh đã ổn định và caption
đầy đủ khớp bundle. Giao dịch CAS giữ dấu restart theo assignment và hash intent trước
khi tắt app; lỗi hoặc crash không tự lặp lại thao tác tắt. Màn Home, composer, upload,
khác tài khoản hoặc máy còn bài khác chưa rõ kết quả đều giữ đường quan sát/review.
Restart chỉ mở lại TikTok để lấy link; không bấm Đăng lần nữa và không coi Copy
`tiktokProcessing` là bằng chứng bài đã xuất bản.
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
Xem [hồ sơ lịch sử](../archive/README.md).
Scope LinkAndSheet/SheetOnly không được dispatch fresh sibling trong campaign.
Clean-start guard đọc hàng chờ dưới lease để Nuôi/Publish không đóng upload khác;
manual viewing vẫn được, IdleSweep đứng ngoài máy đang chờ.

Sheet hỗ trợ mẫu compact qua [Apps Script](../apps-script/README.md): `postedAt` lấy từ
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

### Phục hồi chỉ xác minh trong Theo dõi

`publish_recovery_capabilities(campaignId)` trả quyền và lý do riêng cho `checkLink`,
`resumeVerification`, `retryBeforePost` theo assignment/revision. UI chỉ hiển thị
**Tiếp tục xác minh bài đã gửi** khi backend cho phép; nút xác nhận nói rõ chỉ bài
cũ trên máy đó, không tiếp tục sibling chưa gửi. Mutation gọi
`publish_resume_verification(assignmentId, confirmed, expectedRevision)`; revision
cũ hoặc bài không đủ identity trả stale/ineligible, không mở lại đường Post. Backend
vẫn kiểm quyền tại transaction; capability chỉ hướng dẫn UI, không là giấy phép
vượt điều kiện khi trạng thái đã đổi.

Resume giữ campaign cancelled, publication/effect intent, jobs và đích Sheet/epoch
ban đầu. Cause `verificationNoProgress`, `processingWaitExhausted`, `metadataRetryBudgetExhausted` và `publicMetadataUnreachable` cho phép resume rõ ràng; các review
khác không được nới. Transaction lưu status/budget/review-before-stop cũ trong
`verificationBeforeResume` rồi mở budget mới; gọi lặp/ACK replay không reset.
Worker hiện có dùng `nextCheckAt = checkedAt + 120 giây` khi còn ngân sách; 8 lượt
không tiến triển liên tiếp ở bước tìm bài, 12 lượt metadata được trả lời mà chưa xác minh, hoặc 120 phút TikTok không trả lời kiểm tra công khai, lại needsReview.
Confirmed resume đặt lại các ngân sách đó và mở hạn chờ xử lý 60 phút mới, nhưng không đổi candidate, thời điểm capture hay metadata attempt.
**Kiểm tra liên kết** một lần có thể
chốt canonical nhưng thất bại không reset budget hoặc mở lại review đã dừng.
**Kiểm tra liên kết** gọi `publish_check_links`, không
`publish_execute`; phản hồi pending/busy/stopped/stale/noCandidate không được coi là
verified. **Dừng kiểm tra lại** gọi `operation_stop` theo `publish:<campaignId>` để
dừng quyền quan sát đã tiếp tục trong cả campaign, không chỉ dòng đang xem; các
receipt và trạng thái cancelled vẫn giữ. Intent đóng phiên được ghi cùng transaction
thu hồi quyền; resume từ chối `stopInProgress` trong khi kết quả đóng còn stopping,
needsAttention hoặc failed, kể cả sau restart. Chỉ kết quả closed sau khi các worker
đóng đã kết thúc mới cho phép resume. Refresh detail/capabilities/guards không được
tự tạo, Execute hoặc resume. Observation đã lưu needsReview không trả pending và
không hứa retry định kỳ khi không có nextCheckAt.

Resume không giải guard thiết bị. Khoản chờ Submitted cũ chỉ có thể nhả giữ theo
policy riêng: đủ ít nhất 4 giờ từ submittedAt, không active pipeline, quan sát mới
own-profile đúng tài khoản/package và binding intent; idle proof có hiệu lực hữu hạn
24 giờ. Còn guard khác của cùng máy thì máy vẫn bị giữ. Không lấy tuổi campaign,
ảnh cũ hoặc việc bấm nút resume thay proof; không xoá stop marker để né admission.

**Thử lại trước khi Đăng** là mutation riêng `publish_retry_assignment`, chỉ cho bài
failedBeforeDispatch chưa có effect intent và có `retryBeforePost.allowed`. Pipeline
còn chạy hoặc capability chưa đọc được thì nút bị khoá và hiển thị lý do, không
nới atomic claim của backend. UI đọc lại trạng thái khi API trả stale/busy thay vì
gán ready hoặc chuyển sang retry toàn campaign.

Bàn đăng nhanh hiển thị `manifest.notices` cùng đường dẫn bundle/file trong **Cảnh
báo nguồn**, kể cả cảnh báo thiếu, rỗng hoặc không đọc được file đối tác theo kết quả
scanner. Cảnh báo caption trùng chỉ xét tập bài đã chọn và caption override hiện
tại, chuẩn hoá khoảng trắng để so sánh; không sửa caption gốc. Các cảnh báo này tự
chúng không chặn đăng, không xoá dữ liệu hoặc đổi mapping; lỗi input/preflight thật
vẫn có quyền chặn. Số cảnh báo là dữ liệu của lần quét, không cố định theo một nguồn.
Các mô tả này là hợp đồng chức năng, không chứng nhận đã nghiệm thu live trên fleet.

## Nghiệm thu Publish qua ứng dụng đang chạy

[Runbook duy nhất](publish-acceptance.md#nghiệm-thu-publish-qua-ứng-dụng-đang-chạy).

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

Composition có thêm opt-in `library: { flowId, channel: "published" }`. Con trỏ
publication nằm trong DB và cập nhật bằng CAS; trước khi chuyển con trỏ, backend resolve
một snapshot tất cả publication và compile mọi Flow hiện hành. `flow_run` là cửa production
duy nhất resolve dependency mới: khi hash thay đổi, nó ghi một revision cha bất biến rồi
enqueue. Retry/recovery chỉ đọc compiled plan đã ghim, tuyệt đối không lấy publication mới.
Chế độ snapshot xóa sâu metadata `library` trên bản sao để toàn subtree thật sự đóng băng;
Flow lịch sử không có metadata giữ nguyên bytes và semantics.
`flow_library_unpublish` cũng dùng CAS và bị chặn khi còn publication hoặc bản hiện hành
tham chiếu nguồn. Publish/Unpublish/Save/Archive/resolve-new-run giữ cùng gate SQLite liên
process; hai desktop dùng chung DB không thể xen kẽ tạo reference treo. Chỉ sau khi bỏ
công bố và gỡ consumer mới được Archive.

Local API task routes gọi cùng Flow command handler của UI; không tạo scheduler
riêng. POST chạy Flow không tự retry khi mất response. API status/cancel luôn dùng
run ID trả về; contract/schema ở trang API và module `local_api/tasks.rs`.

GUI Service có endpoint template-match cục bộ dùng OpenCV/NumPy; thao tác xem thử
trong FlowVisionCapture không chạm điện thoại. Snapshot, crop, hash, epoch và
generation được ràng buộc hai phía. Tối đa N worker native (mặc định 4, dùng chung
với OCR; `RIVIU_OCR_WORKERS`); client timeout vẫn giữ slot đến khi công việc native
kết thúc. Các tùy chọn thử đa tỷ lệ chỉ áp dụng phép
thử này; Flow TapVision/IfVision vẫn dùng matcher Rust và config đã lưu.

`OcrReadText` lấy frame từ generation đang sở hữu rồi gọi OCR cục bộ qua reasoner
được tiêm vào FlowRuntime. Screenshot/hash/epoch được kiểm hai phía. Tesseract và
model vie/eng được ghim SHA-256 và đóng gói; máy chạy không tải model ngầm.
Output OCR giữ văn bản, engine, confidence, bounds và binding trong ledger; lỗi
đọc/mất generation không thành văn bản rỗng giả. Worker OCR là process thường trú
giữ model đã nạp, dùng chung trần N slot cục bộ với template matching (mặc định 4);
timeout/hủy giết và thu hồi đúng worker đó trước khi nhả slot. AI resolve/recover giữ
trần riêng hai slot.

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

`CommentLocatorIdentity.commentLink` là trường tùy chọn tương thích dữ liệu cũ,
chứa `postId`, `commentId`, URL đã bỏ tracking. Adapter `tiktok_comment_link`
đo trên musically45.7.3/en: giữ đúng hàng → Share with → Copy link, clipboard có
sentinel mới; redirect chỉ HTTPS trên các host TikTok cho phép. Không nhận link
bài thiếu ID, ID trùng tham số, hoặc post khác. Worker bổ sung link sau khi chứng
minh nội dung/tác giả, trong deadline còn lại; thiếu link không đảo trạng thái gửi.
`interaction_comment_link` lấy ID cho lượt đã verified; `captureUdid` cho phép lấy
trên máy khác cùng campaign sau khi chứng minh tác giả. `udid` kiểm mở ID trên máy
thuộc campaign; không dùng cùng `captureUdid`. Không có thao tác gửi/xóa. Reply mở link HTTPS
chứa `share_item_id`/`share_comment_id` (musically45.7.3 không nhận scheme `aweme`),
sao chép lại link của hàng vừa tới và yêu cầu ID khớp trước dùng nút Reply.
Cả hai lần đều đi qua Copy link của bảng chia sẻ bình luận, có thể bị tính là lượt
chia sẻ; ID bình luận không đọc được từ màn hình nên vẫn giữ. `commentLink` có trong
evidence nghĩa là đã có một lần Copy link như vậy.

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
claim độc quyền theo thiết bị và giới hạn riêng cho chuyển media. Mọi pha điều khiển máy
qua adb (`transfer`, `compose` gồm quan sát, chọn nhạc, soạn, Post; `verify` trên máy;
`cleanup`; `appCompletion`) chia chung một ngân sách `PUBLISH_PHONE_WORK_CAP` = 10 máy trong
toàn ứng dụng (`crates/core/src/db/publish_dispatch.rs`); chỉ lượt đọc metadata mạng
(`metadata`) đứng ngoài. Đây là trần, không phải mục tiêu, và là số đo 13–14/10/2026 trên
30 máy SM-G955F/N Android 9 (`target/device-acceptance/controllers/controller-r105.stdout.txt`):
30 máy cùng soạn làm UiAutomator2 `/source` p50 6,7 s, p90 12,7 s, max 18,3 s (bình thường
dưới 1 s), ~1/3 dòng hỏng lượt đầu ở picker/bảng nhạc; vòng retry 10 máy đăng ngay 9/10.
Trần cũ 6 (đo 2026-10-12: 20 máy soạn với 20 tile live làm adb daemon timeout) chỉ đếm
compose, nên 6 máy soạn cộng quan sát xác minh không giới hạn vẫn thành 20–30 máy bận.
Job compose/transfer vượt trần ở lại `queued` với lý do `stage_capacity`, trước mọi effect và
intent; quan sát xác minh trên máy vượt trần chờ với lý do `fleetCapacity`
(`VerificationWait::FleetCapacity`, worker đọc `publish_phone_work_in_use` mỗi tick nhưng
claim trong SQLite mới là nơi quyết định). Worker chỉ tạo task sau admission;
media được tải theo bài đã nhận, hàng chờ chỉ giữ ID. `publish_attempts` giữ lịch
sử lần thử; `publication_id` được backfill bằng assignment ID và bất biến, không
nhóm lại dữ liệu cũ theo caption, thư mục hoặc tên máy. Intent và revision của
pipeline vẫn kiểm tại nút Đăng. Worker đã lỗi sau effect chỉ chuyển sang xác minh.

Trong trần đó, claim compose mới được giãn:
- tối đa một lần mỗi 8 s;
- số máy soạn cùng lúc bắt đầu ở 4, chỉ tăng dần tới trần khi q90 đọc cây của fleet dưới 8 s;
- giảm một nửa (không dưới 2) mỗi lần `publish_observation_deadline`.

Retry cả lượt chờ q90 đó hạ, tối đa 10 phút sau giờ hẹn. Máy chưa có số đo mượn q90 của fleet.
Khi adb server mất hoặc bị nghi treo thì không claim việc mới, tối đa 10 phút mỗi sự cố; job
đang chạy vẫn đi tiếp theo luật intent. Hằng số và lý do nằm trong
`crates/core/src/read_latency.rs`; các số này chưa đo, phải đo lại trên box 20 máy.
Một hàng album đã thấy một lần mà chưa xác nhận được trả `album_row_unconfirmed` (retryable),
không phải `album_not_visible`. Bằng chứng ổn định qua hai lần đọc phải từ chối snapshot dùng
lại: generation phải tăng.

Dispatcher không tự quét danh sách máy (`publish_commands/pipeline.rs`, `RosterRefresh`). Nó
đọc kết quả thô của lượt quét nền (`state.rs`, ~3 giây một lượt, `DeviceRegistry::record_scan`
lưu danh sách đúng như driver trả, trước khi gộp vào tile hay đổi trạng thái), nên đội máy
không bị quét hai lần và vòng chính không bao giờ chờ khóa inventory, hàng đợi adb hay probe
chậm; shutdown chỉ chờ worker đang giữ máy, rồi ghi journal completion như cũ. r110
(14/10/2026, 28 máy): lượt quét riêng của dispatcher tới tay sau 145–180 giây trong khi quét
nền cùng đội máy chỉ ~3,8 giây (nguyên nhân chưa xác định), nên cả 28 job nằm
`roster_pending` 20 phút. Lượt quét nền liệt kê backend iOS và Android song song; backend liệt
kê từ 5 giây trở lên ghi "liệt kê máy của một backend chậm". Sidecar iOS `list` sạch mà không
có iPhone thì 30 giây sau mới chạy lại (iPhone mới cắm hiện trong tối đa 30 giây). Dòng
"Android inventory metadata enrichment" có `scan_elapsed_ms`, `lock_wait_ms`, `largest`.
Inventory Android dùng lại model/OS/pin 60 giây mỗi máy (r111: mức 15 giây chiếm ~110/160
lệnh `adb shell` mỗi phút của đội 28 máy rảnh) và hỏi `/status` của agent tối đa 8 máy cùng
lúc. Kết quả quét chỉ dùng được trong `ROSTER_MAX_AGE` = 300 giây tính từ lúc lượt quét bắt
đầu (biên an toàn, không phải thời gian đo được); quá hạn ghi một cảnh báo. Chưa có kết quả hoặc
kết quả quá hạn nghĩa là *chưa biết*: job ở lại `queued` với lý do `roster_pending`, không
mở cửa sổ reconnect 2 phút, không đánh lỗi `unauthorized`, không coi cả đội là offline.
Danh sách máy chỉ cho phép nhận claim; lease của worker và các lần đọc đúng serial đó
(build TikTok, tải máy dưới lease) vẫn quyết định có effect hay không.

`publish_get_limits`/`publish_set_limits` nhận `{transfer,compose,verify,deviceTotal}`,
lưu ở `publish.dispatch.limits` theo database máy chủ, mỗi giá trị từ 1 đến 64.
Giữ cả bốn trường để tương thích dữ liệu cũ. Admission dùng `transfer` cho lượt chuyển media (vẫn nằm trong ngân sách máy chung), `verify` làm trần số lượt đọc metadata mạng đồng thời, và trần cứng `PUBLISH_PHONE_WORK_CAP` = 10 cho mọi pha trên máy (không đọc từ trường `compose`); `deviceTotal` không tác động admission.

**Hình trực tiếp trong lúc đăng.** View scrcpy của máy đang đăng vẫn chạy suốt lượt; chỉ
bước đọc bảng nhạc tạm dừng luồng (HX-PAUSE, `pause_capture`/`resume_capture`), khi đó tile
hiện "Đang đăng bài, tạm dừng hình". Không giữ view tắt cả lượt: HX-QUIET (`547afdfd`) làm
thế và trung vị `/source` theo pha (chẩn đoán của các bài thành công, SM-G955F/N Android 9)
tăng gấp đôi ngay hôm đó: thư viện 0,55 → 1,2–1,3 s, chọn ảnh 1,2 → 2,5–2,7 s, đọc tài khoản
0,65 → 1,1 s (r103b/r105/r106 so với r107–r113, 13–14/10/2026); bảng nhạc, nơi cả hai đều tạm
dừng, giữ ~1,0 s. Đã gỡ 14/10/2026.
Giảm `transfer` chặn cấp lượt chuyển mới đến khi số đang chạy xuống mức mới,
không cắt request đang thực hiện. Phân trang candidate chỉ phân chia lượt quét,
không giới hạn tổng worker. Foreground stream tăng theo số máy đã phát hiện;
preview vẫn dùng ngân sách cấu hình chung và nhường cho foreground.
Đợt hẹn giờ chỉ được claim trong cửa sổ 30 giây theo giờ máy chủ, `[run_at, run_at + 30 s]`
(`claim_publish_pipeline`): giờ hẹn áp cho lần nhận cả đợt, không áp cho từng máy. Không job
nào mang deadline (`enqueue_publish_jobs`): trong đợt đã nhận, hàng chờ giãn compose 8 s,
`PUBLISH_PHONE_WORK_CAP`, reconnect hay backoff thử lại không bao giờ thành `missed` (20 máy
× 8 s = 160 s là chờ bình thường); started_at_ms chỉ được ghi khi thực sự dispatch. Claim sau
cửa sổ chỉ nhận lượt đã chạy, tức người vận hành chạy lại, nên cũng không có deadline (trước
đây hàng chạy lại nhận deadline cũ đã qua và thành `missed` ở tick sau). `deadline_ms` chỉ còn
ở job của bản cũ; dispatcher và lúc mở app chỉ chuyển `missed` khi deadline đó đã qua. Đợt
không được nhận trong cửa sổ thì cả lượt CAS sang `missed`: `app_opened_after_deadline` khi
app mở sau `run_at + 30 s`, `schedule_capacity_deadline` khi app đang chạy mà không nhận được
đợt trong 30 giây; event của lượt ghi `runAt`, nhật ký thao tác ghi `publish.schedule.missed`
kèm lý do; không tự đăng bù. App mở trong cửa sổ vẫn nhận đợt.

Restart không cần người vận hành cho việc chưa bấm Đăng. Lịch chưa tới giờ giữ `scheduled`
(Start của nó giữ `queued`). Job chưa có effect intent tự chạy tiếp
(`publish_recovery::resume_after_restart`): `queued`/`paused` giữ attempt, pha và sổ thử lại,
chỉ bỏ cửa sổ reconnect đã chạy lúc app đóng; `running` mất attempt cùng tiến trình, nên chạy
lại từ chuyển media với attempt mới, nhạc đã chọn được lưu làm bằng chứng rồi chọn lại, và lần
bị ngắt tính một lượt thử của bước đang chạy (`app_restarted_before_post`) để một attempt làm
chết tiến trình không lặp vô hạn qua các lần mở; bước đã hết lượt thì dừng trước Đăng
(`restart_retry_exhausted`) chờ **Thử lại máy này**. Job có intent chỉ đối soát. Start chỉ
thành `uncertain` (`PublishStartInterrupted`) khi còn chuẩn bị: `accepted`/`preparing`, hoặc
`queued`/`running` mà lượt chưa hẹn giờ và chưa có job; chỉ job của Start bị ngắt ở chính lần
mở đó dừng (`start_interrupted`). Trước migration40→41 tạo và đọc lại bản sao SQLite
`pre-publication-v40.db`.

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
Cây đọc xong mà không có node nào của package TikTok là app khác chiếm foreground,
không phải cây picker thiếu: chẩn đoán ghi `foregroundNotTarget` cùng package đó,
và bước dọn composer không bấm Back vào app khác; lượt sau khởi động sạch TikTok.


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
## Tích hợp TypeSafe

`core::typesafe::Client` gọi HTTP API System One với một Choice về bằng chứng chữ.
Endpoint cố định, không redirect, tối đa bốn request đồng thời và deadline tổng 25
giây gồm chờ admission. Kiểm schema, tập lựa chọn, xác suất và tổng phân phối trước
khi sử dụng. Không ghi nội dung yêu cầu hoặc credential vào tracing; ghi verdict,
confidence, token, model và thời gian. Không tự retry request trong đường bình luận.

`typesafe_get_settings`, `typesafe_update_settings` dùng revision/CAS riêng;
`typesafe_update_credential` chỉ ghi SecretStore, không có fallback SQLite.
`typesafe_check_comment` là thao tác API rõ ràng, không auto-refetch. DTO sinh bằng
ts-rs trong `generated-ipc.ts`. Client đính vào NurtureSettings ở backend có
`serde(skip)` và Debug đã che khóa. Session snapshot không đổi vì settings toàn cục
thay giữa lượt. Gate TypeSafe bổ sung cho `grounded_verify`; không thay vision,
account/target proof, lease, intent hoặc cancellation. Phiên chỉ có chữ cần lựa chọn
supported với confidence ≥ 0,80 và xác suất supported ≥ 0,85; cần đo lại trên dữ liệu
tiếng Việt khi đổi model. Kết quả insufficient của phần chữ không phủ nhận chi tiết
có thể được ảnh chứng minh, nên nhánh vision vẫn dùng verifier ảnh hiện có.

Bản dev tối ưu riêng `zune-jpeg` ở mức 3 để kiểm ảnh trong trace theo kịp các máy
chạy đồng thời; code ứng dụng vẫn giữ debug. `trace_bench INPUT_IMAGE OUTPUT_DIR`
đo giải mã và ghi mười quan sát qua artifact store thật. Không bỏ giải mã, hash,
atomic publish, fsync hoặc tăng hàng đợi để che trace bị thiếu.

Tìm kiếm Android giữ mapping card/grid/author theo build Global 45.4.3, 45.7.3,
46.0.41, 46.1.3 và 46.4.3; chỉ mở card sau khi đọc lại đúng từ khóa và tab Videos.
Picker album Trill 38.3.2/en cuộn trong RecyclerView h27 khi album import chưa ở
viewport; tên album vẫn phải khớp duy nhất và ổn định. Sau khi chạm dòng album, pill
chỉ xác nhận khi một lần đọc xong ghi đúng tên. Đọc xong mà pill ghi album khác thì
từ chối `AlbumNotConfirmed`; đọc không xong hoặc pill chưa có chữ là chưa biết và trả
deadline quan sát có kiểu. Không chạm lại dòng album. Nút Gửi trên đường hierarchy
được đọc từ một snapshot mới, gồm vị trí, định danh và bit enabled; thiếu bit hoặc
nhiều node cùng khớp thì từ chối trước effect.

Global 45.7.3/46.0.41 có thể hỏi mở Settings để cấp vị trí trong lúc chuyển tab
tìm kiếm. Chỉ nhận đúng tiêu đề, lời giải thích và nút Cancel trong cùng panel;
không mở Settings hoặc cấp quyền. Snapshot tạm thiếu ô tìm kiếm phải chờ trong
deadline, chưa được chọn card; ô hiện lại phải vẫn khớp từ khóa nguyên vẹn.

Save Global 45.4.3/45.7.3/46.0.41/46.1.3/46.4.3 dùng icon selected trong đúng
chuỗi parent được đo. Follow đọc header/handle và nút quan hệ riêng theo build;
counter Following không chứng minh quan hệ. Sau intent chưa đọc được kết quả thì
giữ uncertain, chỉ mở profile chuẩn để đối soát, không bấm Follow lại. Picker tag
45.4.3/45.7.3/46.0.41/46.1.3/46.4.3 nối username chính xác với nickname trong cùng
hàng trước chọn; toàn bộ câu sau chọn vẫn phải khớp phép thay token đã chứng minh.
