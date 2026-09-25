# Hướng dẫn phát triển

Stack giữ nguyên: Rust workspace, Tauri 2, React/TypeScript/Vite. `src/api.ts` là biên
IPC frontend. Không thêm một control plane riêng để đi vòng ownership/admission hiện có.

Seeding dùng `ThreadCampaignRequest.seeding`, planner và ledger Tương tác hiện có;
migration 46 thêm Share và khoảng ordinal cho lượt hành động độc lập bên cạnh tối
đa 64 bình luận. Lịch cũ không có `seeding` giữ hành vi cũ. Bù Tim/Lưu phải claim
ngân sách trong transaction; armed/uncertain vẫn chiếm lượt. Reply chờ bằng chứng
câu cha và không replay Send khi chưa rõ kết quả.

Deployment checker được build từ crate `riviu-deployment-checker`, dùng lại mã
kiểm package của desktop mà không link Tauri. Stage đúng checker sau lần compile
cuối và đối chiếu hash trước bundle; EXE/MSI dùng chung app đã compile một lần.

DTO thiết bị, khả năng, Dừng, timeline và Sheet proof được sinh bằng ts-rs 12.0.1:
`cargo run --locked -p riviu-core --example export_ipc -- apps/desktop/src/generated-ipc.ts`.
`python scripts/check_generated_ipc.py` kiểm không có drift. TanStack Query chỉ
giữ read model persisted theo run/device/phạm vi; metadata thiết bị cache 2 giây,
revision bất biến có thể cache lâu, còn danh sách thay đổi nhanh luôn stale. Mutation
làm stale đúng family liên quan. Retry, focus và reconnect refetch bị tắt. Credential,
preflight, probe thiết bị và effect không dùng query cache.
SQLite async đi qua `StorageExecutor`: tối đa một writer, hai reader và 64 yêu cầu
chờ; admission xảy ra trước `spawn_blocking`. Các monitor/guard đọc theo lô trong
executor; transaction không được giữ qua await HTTP hoặc thiết bị.

Publish recovery dùng migration 45 và dispatcher hiện có. `publish_retry_assignment`
nhận `assignmentId`, `confirmed`, `expectedRevision`, `requestId`; replay cùng request
chỉ trả ACK, không tạo attempt khác. Retry tự động tối đa ba lần mỗi bước, thủ công
chỉ tạo một lượt bài nhưng vẫn cho tối đa ba retry tạm thời trong từng bước của
lượt đó. Lượt thủ công không tự requeue cả bài khi worker kết thúc. Counters,
checkpoint, account và ứng viên nhạc được giữ trong SQLite.
Reconnect đúng serial tối đa 120 giây nhường work permit; startup không tự chạy lại
lượt recovery gián đoạn. Có Post intent thì chỉ xác minh; `publish_retry_sheet_assignment`
chỉ mở lại outbox của bài đã có canonical proof, không đi vào composer.
Recovery state lưu thêm `lastErrorCode` và `lastErrorKind`. Các đường mới truyền
`RecoveryFailure` có kiểu; parser chuỗi chỉ là compatibility boundary cho journal/lỗi
cũ. Thay chữ hiển thị không được đổi retry policy. Retry vẫn chỉ trước Post và tối đa
ba lần mỗi bước.

Preflight Android dùng `DeviceControlPlane::tiktok_action_capabilities` và catalog
`app_automation::action_capabilities` chung cho UI, Nuôi thủ công/lịch/Điều phối,
Tương tác và Đăng bài. Readiness chỉ đọc khóa màn hình, transport và owner, không
đánh thức máy hoặc mở session. Mỗi action thiếu locator/tuple bị từ chối riêng;
`runtimeProofRequired` cho phép vào bước chứng minh target, chưa cấp quyền tap.
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
Sau khi bấm Next của picker, lỗi accessibility/HTTP timeout ở bước chờ editor
cho phép quan sát XML thêm tối đa 30 giây. Chỉ hai generation mới trong cùng phiên,
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

`RIVIU_DEV_MANUAL_ACCEPTANCE=1` trong bản debug giữ mọi lịch tự chạy ở trạng thái
chờ và không sửa lịch đã lưu. Chế độ này cũng không tự resume Flow/Điều phối,
không chạy cleanup/idle sweep/comment verifier, không tự đóng app và không bind
Local API đã lưu. Dispatcher chỉ nhận đúng cặp campaign/máy trong file tuyệt đối
`RIVIU_DEV_ACCEPTANCE_SCOPE`; `RIVIU_DEV_ACCEPTANCE_ACTIVATION` phải khớp
`activationId` trong file. Thiếu file/activation, JSON lỗi hoặc ID không khớp đều
đóng kín. Backend chụp danh sách tối đa 100 máy lúc khởi động và pin toàn bộ danh
sách campaign được kích hoạt (tối đa 100); thay campaign hoặc danh sách máy giữa
phiên làm toàn bộ gate đóng lại. Giới hạn concurrent production vẫn được giữ.
Nghiệm thu hẹn giờ phải bật riêng `capabilities.publishSchedule`; chỉ lịch có
campaign và toàn bộ máy nằm trong scope mới được clock production nhận khi tới
giờ. Các lịch khác, Nuôi và Điều phối vẫn đóng băng, không sửa lịch đã lưu.

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
Đăng bài luôn đứng yên;
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
selector duy nhất theo package; schema selector v2 hỗ trợ prefix, scope ancestor và
clickable ancestor có định danh/độ sâu/geometry hữu hạn. Schema v1 thiếu các trường mới
vẫn khớp exact như cũ. Recorder ghi intent trước tap, lưu ảnh/cây trước và sau. Nó
không tự chọn node mới sau delay: UI gọi `inspector_confirm_postcondition` với selector
người dùng chọn; backend kiểm selector đó chỉ có ở snapshot sau. Bước chưa xác minh giữ
unverified và không xuất Flow tự chạy.
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
dispatch package/link; Instagram/Threads từ chối rõ, chưa implement. Các điểm lưu
settings/profile/campaign, claim job và worker phục hồi phải từ chối network
chưa hỗ trợ trước effect; chỉ thêm enum/adapter GUI không cấp quyền chạy Nuôi,
Tương tác hoặc Đăng trên Threads. Orchestration fleet
(`OrchestrationDocumentV1`) là đồ thị gọi vào engine Nuôi / Tương tác / Đăng hiện có; nút
“Tạo mẫu 3 chức năng” seed 3 hồ sơ + một điều phối Nuôi→Tương tác→Đăng. Engine vẫn là
source of truth — không thay bằng node Flow V2 tap/swipe.

Lịch Script cũ claim mốc và tạo job trong cùng transaction; crash sau claim giữ
job chưa xác định/hủy theo bằng chứng, không tạo job thứ hai cho cùng mốc. Lịch Nuôi
cũ ghi intent và tiến mốc trước `start_many`; intent chưa settle cần đối soát thủ
công, không tự phát lại. Dev manual acceptance vẫn quét roster và tự mở ảnh xem
trước Android thụ động, nhưng không tự chạy lịch/worker có hiệu ứng hoặc nền iPhone;
lệnh thủ công vẫn có thể tác động máy nên không coi chế độ này là read-only toàn cục.

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
Publish preflight đọc điều kiện của tối đa bốn Android cùng lúc, dưới trần admission
ADB của host; trong mỗi máy vẫn giữ thứ tự transport, dung lượng, trạng thái khóa
và package/build. Kết quả được ráp lại theo thứ tự ghép bài, giữ đúng issue của
từng máy. Sheet vẫn có một lượt kiểm tra writer/target/epoch riêng mỗi preflight;
cache hiển thị trên frontend chỉ cho phép bắt đầu kiểm tra, không thay kết quả này.

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
Các khung chọn máy dùng chung `MachineChoice` và `machine-choice.css`: Nuôi/Tương tác
mở drawer hai cột, Đăng bài dùng danh sách hàng trong khung Thiết bị của bàn ba khung.
Danh sách cuộn riêng, không phân trang máy; drawer Tương tác cho cuộn tiếp tới phần
tài khoản khi hết danh sách trên cửa sổ thấp. State vẫn thuộc workspace; riêng Đăng bài
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
`Verifying` giữ media và nhịp xác minh (Android restart TikTok trước Copy); `Succeeded` đòi canonical link, đúng tài khoản
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

`publish_commands/verification.rs` chạy phiên xác minh trong control plane; DB CAS ghi
proof, outbox và snapshot cùng transaction. `verified_cleanup.rs` xử lý riêng media
đã xác minh theo delete policy và importId, giữ khả năng thử lại qua restart.
Worker lưu `nextCheckAt` bằng thời điểm kết thúc quan sát cộng 300 giây cho mọi bài
đã gửi còn thiếu link, kể cả lỗi đọc. Không đặt tổng hạn chờ liên kết; mỗi lần quan
sát vẫn có deadline và lease riêng. Restart giữ nguyên mốc gửi và lần kiểm tiếp.
Android ở trạng thái Connected sau restart vẫn được kiểm khi agent đã sẵn sàng;
worker mở session qua control plane, không đòi mở điều khiển bằng tay để đổi
trạng thái thành Ready. Máy Busy/Preparing/Error/offline vẫn chờ; iOS giữ điều kiện Ready.
Với receipt Android `submitted`/`posted` chưa có link, `verification_restart` tắt đúng
package đã ghi trong intent, kiểm proof hết tiến trình, mở lại và kiểm tiến trình đang
chạy trước Copy. Cùng lease giữ suốt chu kỳ; kiểm revision/stop trước và sau các await,
không đóng máy còn bài khác đang giữ. Proof restart ghi vào `verificationDiagnostic.appRestart`.
Android hẹn giờ bắt đầu kiểm khi phiên đăng nhả máy; lỗi restart cũng giữ nhịp 300 giây.
Unknown receipt và iOS giữ đường đọc không restart. LinkAndSheet không quay lại Post.
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
ban đầu. Bài còn thiếu link dùng worker hiện có, `nextCheckAt = checkedAt + 300 giây`,
không có tổng hạn buộc bỏ bài pending; từng observation vẫn hữu hạn. Gọi lặp không
reset lịch hoặc gửi lại bài. **Kiểm tra liên kết** gọi `publish_check_links`, không
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

`scripts/publish_acceptance.mjs` chỉ nối CDP loopback vào **một WebView Tauri đang
chạy**, rồi gọi IPC production. Không mở app/browser, không focus/click màn hình,
không mở driver/ADB, không đọc SQLite hay credential. Cần Node và Playwright đã cài
trong `apps/desktop/node_modules`. Không bật/restart debug app thứ hai để nghiệm thu
khi production đang giữ máy. CDP không có sẵn thì dừng; việc chạy script không tự
cấp quyền tạo thêm lượt public.
Khi nghiệm thu Android cắm USB, thêm `--real-android true`: harness đối chiếu từng
serial với roster Android USB `connected/ready` của chính AppState trước mọi
preflight/Create/Execute. Thiếu máy, app đang chạy mock hoặc roster cũ đều bị từ
chối; `inspect` không mang cờ này chỉ là phép đọc trạng thái, exit 0 không chứng
nhận máy thật. Cờ được ghim trong fingerprint của preflight và submit.

| Mode | Hành vi | Điều không thực hiện |
|---|---|---|
| `inspect` (mặc định) | Đọc roster + device metadata; nếu có campaign ID/receipt thì đọc `publish_get` | Không preflight, quét nguồn, Sheet check, mở thiết bị hoặc mutate |
| `preflight` | Kiểm OAuth writer/target/epoch và gọi `publish_preflight`; lưu hash xác nhận | Không create/execute/Post |
| `submit` | Dùng hash đã duyệt; persist request trước create và intent trước Execute | Không tự replay Execute khi intent đã tồn tại |
| `observe` | Poll `publish_get` đến hạn báo cáo | Không gọi kiểm link chủ động, retry, resume, terminate hoặc Post |

`publish_sheet_check` có thể lưu cấu hình kết nối đã xác minh và làm network I/O;
chỉ gọi trong preflight/submit, **không** thuộc inspect read-only. Script không chuẩn
bị/reset Sheet, đổi writer hay lấy token. OAuth direct phải active và đúng file,
check phải xác minh đúng gid/epoch, preflight phải có Sheet enabled + target v2.
Lượt harness mới luôn giữ media (`deleteAfterPublish=false`), nhạc trending pool5;
đây không phải công cụ sửa caption, lịch hoặc cấu hình fleet.

Ví dụ dưới dùng biến do người vận hành điền từ roster và nguồn đã quét trong UI;
`$udids` và `$bundleIds` là chuỗi ID phân cách dấu phẩy **cùng thứ tự một-một**, không
phải số máy hoặc index. `$reportDir` dành riêng cho một lượt. Đọc hết `report.json`,
đặc biệt `roster.excluded`/issues, trước khi duyệt. Script không thu nhỏ tập khi một
máy bị chặn. Số máy lấy từ metadata, thiếu thì `null`, không đoán theo vị trí.

```powershell
# Chỉ đọc ứng dụng hiện có; port phải là CDP loopback đã được phép.
node scripts/publish_acceptance.mjs --report-dir $reportDir --udids $udids --cdp $cdp

# Có đọc thiết bị/network qua AppState production; không tạo bài.
node scripts/publish_acceptance.mjs --mode preflight --report-dir $reportDir --udids $udids --source $source --bundle-ids $bundleIds --sheet-id $sheetId --sheet-gid $sheetGid --cdp $cdp

# CHỈ sau khi phạm vi/nội dung/số bài được người vận hành cho phép đăng public.
# $confirmation là đúng hash report.confirmation từ preflight trên.
node scripts/publish_acceptance.mjs --mode submit --report-dir $reportDir --udids $udids --source $source --bundle-ids $bundleIds --sheet-id $sheetId --sheet-gid $sheetGid --cdp $cdp --confirm $confirmation

# Quan sát bài đã gửi; hết hạn báo cáo không dừng worker của app.
node scripts/publish_acceptance.mjs --mode observe --report-dir $reportDir --udids $udids --campaign-id $campaignId --cdp $cdp --wait-seconds 600 --poll-seconds 10

# Test local, mock duy nhất biên IPC; không nối app hoặc thiết bị.
node --test scripts/publish_acceptance.test.mjs
```

`--cdp` mặc định `http://127.0.0.1:9277`, chỉ nhận IP loopback/cổng. Nếu có nhiều
WebView, thêm `--page-url` đúng URL loopback và giữ giá trị ấy ở cả preflight/submit.
Không truyền lệnh IPC tùy ý. `--wait-seconds` chỉ dành observe (0–86400); poll 5–3600
giây chỉ đọc metadata, **không đổi nhịp verifier 300 giây**. Observe cần đúng danh sách
UDID của toàn campaign; thiếu/dư assignment là scope mismatch, không báo pass.

Intent ghi exclusive + fsync trước IPC. `create-intent.json` giữ UUID requestId,
fingerprint toàn request và confirmation; mất ACK create thì chạy lại **cùng submit,
cùng thư mục, cùng hash** để backend trả receipt cũ. Không đổi ID hoặc sửa/xóa intent.
Có `execute-intent.json` thì submit sau chỉ đọc campaign, kể cả process chết trước
khi biết Execute đã tới backend hay chưa. Khi chưa có intent local nhưng backend
đã có effect/dispatch hoặc campaign/assignment không còn queued/ready, harness cũng
chỉ observe, không Execute dựa vào việc mất file local. Trường hợp này có thể chưa được enqueued:
đối chiếu trong app, không tự phát lại lệnh. Report directory có lock chống chạy
đồng thời; lock còn sau crash cần kiểm tiến trình đã kết thúc trước khi người vận
hành gỡ lock, không gỡ intent. Không dùng report directory khác để né bảo vệ.

Báo cáo phân biệt `enqueued`, receipt `submitted`, `publicationVerified` + canonical
URL, `sheetSent` từ settlement backend và `urlReadback`. URL trần hoặc state succeeded
không thay proof. Khi đã có bài verified và delivery sent, harness gọi
`publish_sheet_readback` qua OAuth backend để đọc receipt và ô Sheet đúng
assignment/revision/epoch; chỉ đánh dấu `matched` khi URL, identity và revision
khớp. Thiếu OAuth, readback lỗi hoặc chưa tới lượt thì giữ `pending`, không tự
gửi Post hay Sheet lại. Không trích token để đọc bảng private. CSV chỉ là đối
chứng URL, không thay bằng chứng delivery writer. Harness chỉ chứng nhận
end-to-end khi có đủ ba lớp proof trên đúng máy và tài khoản.

Outbox canonical `failed` chưa có receipt dùng `publish_sheet_diagnose_failed`
với `assignmentId`, `expectedRevision`, `startRow` (bắt đầu từ 2). Lệnh GET chỉ
đọc target/epoch/account đã ghim, tối đa 32 trang hoặc 45 giây một lát; trả
`nextRow` để đọc tiếp cho tới `complete`. Kết quả chỉ có số hàng và các cờ
identity/revision/fingerprint/ô D khớp, không trả URL, note gốc hoặc dữ liệu
đối tác. `complete` chỉ có nghĩa đã quét hết grid hiện tại; note hỏng, hàng bị
chỉnh hoặc writer khác đang ghi vẫn cần đối soát thủ công. Lệnh không nhận
receipt, không đánh dấu `sent` và không mở retry.

| Exit | Ý nghĩa |
|---|---|
| `0` | Inspect không có campaign hoặc preflight đạt; **không** là bài đã đăng/Sheet đã ghi |
| `1` | Bị chặn/lỗi, sai scope, review, cancelled, failed hoặc superseded |
| `2` | Còn pending/hết hạn quan sát hoặc ACK create chưa rõ; không retry Post |
| `3` | Backend báo tất cả verified + Sheet sent nhưng thiếu readback bổ sung/receipt identity |

Canary Rust `publish_commands/live_canary.rs` là **scout cô lập**, dùng DB scratch,
Sheet disabled, không có verifier/Sheet worker đầy đủ. Không dùng làm nghiệm thu
OAuth. Mode `publish-one`/`link-only` cũ bị từ chối trước tạo driver; inspect/rehearse/
scout cần `RIVIU_PUBLISH_CANARY_ISOLATED=confirmed-no-production-owner`, cùng
`RIVIU_PUBLISH_UDID`, `RIVIU_PUBLISH_HANDLE`, `RIVIU_PUBLISH_BUNDLE`, nguồn và thư mục
report riêng. Đây là xác nhận do người vận hành chịu trách nhiệm, không phải phép
dò chứng minh máy rảnh; không chạy song song production trên cùng máy.
Canary không còn dispatcher công khai hoặc unconditional terminate TikTok; link-scout
mở warm session, không dùng cold-start cho bài đã gửi. Scout trước Post vẫn dùng
chuẩn bị phiên publish nên chỉ dành máy cô lập đã xác nhận không có upload; rehearsal
chỉ dọn media khi kết quả chắc chắn chưa đăng, còn uncertain thì giữ. Khi kết thúc
chỉ nhả session/process do nó sở hữu, không tự kết luận upload đã xong. Không chuyển
DB canary vào production hay lấy credential production cho canary.

## Bộ cài Windows

Binaries chẩn đoán cần feature `diagnostics`; checker cần `deployment-check` và được
build/stage riêng trước bundle. Bundle mặc định không mang các exe cũ dưới src/bin.
Windows CI gọi `tauri build --no-bundle` một lần, rồi `bundle_windows_installers.py`
đóng gói NSIS/MSI bằng cùng executable và chỉ thay overlay resource WiX. Script kiểm
dấu bundle duy nhất, phục hồi đúng ba byte trước mỗi loại và từ chối nếu code bị đổi;
điều này giữ đúng loại updater sau lỗi/gián đoạn bundler. Không build Rust lần hai. Overlay
`tauri.fast-bundle.conf.json` là lựa chọn nén zlib cho bộ cài local, không bật mặc định
trong release CI. Không dùng kết quả warm-cache để tuyên bố thời gian clean build.
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

### Smoke giao diện Tauri cô lập

Debug có chế độ `RIVIU_UI_SMOKE=1`, bắt buộc đi cùng `RIVIU_MOCK_DEVICES=1` và
`RIVIU_UI_SMOKE_DIR` là đường dẫn tuyệt đối tới thư mục **chưa tồn tại**, trong một
thư mục cha có sẵn trên đĩa local. Thiếu điều kiện, đường dẫn symlink/junction hoặc
WebView environment override sẽ bị từ chối trước bootstrap. Mỗi lần khởi động cần
thư mục mới; retry startup tạo DB mới bên trong cùng scratch đã nhận.

Chế độ này dùng driver mock trực tiếp, DB/log/WebView profile riêng và credential
chỉ trong bộ nhớ; không đọc kho credential vận hành, khởi USB/sidecar/API listener
hay worker nghiệp vụ. IPC dùng allowlist cụ thể; plugin ngoài event listen/unlisten
bị khóa. Đường production và release không dùng mode này. Chỉ đặt mock/data-dir ở
đường khởi động thường **không thay thế** ranh giới cô lập trên.

Có thể dùng `RIVIU_DEV_BACKGROUND=1` và cổng loopback `RIVIU_DEV_CDP_PORT` của bản
debug để kiểm renderer/điều hướng qua WebView2 mà không chiếm chuột. Chỉ nối đúng
process/scratch đã xác nhận. Các lệnh bị từ chối phải hiện unavailable, không giả
thành sẵn sàng; lỗi xác nhận frontend được ghi vào Hoạt động, không tự retry hoặc
đánh dấu deployment đạt. Smoke này không chứng nhận loaded-state dữ liệu thật,
OAuth/Google, automation điện thoại hoặc bộ cài.

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

Không chạy harness **có driver/control plane riêng** song song desktop trên cùng USB.
Harness Publish qua IPC ở trên dùng chính AppState của desktop, không phải ngoại lệ
cho phép mở driver thứ hai. Thao tác công khai/cài app/chuyển nội dung không thuộc
smoke điều hướng read-only. Số lượng test và kết quả live là số
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
# Tích hợp TypeSafe

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
viewport; tên album vẫn phải khớp duy nhất và ổn định. Nút Gửi trên đường hierarchy
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
