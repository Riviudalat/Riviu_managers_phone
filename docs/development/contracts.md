# Hợp đồng thực thi và dữ liệu

[Hướng dẫn phát triển](../developer-guide.md). Tài liệu này mô tả invariant, không chứng nhận live. Mọi thay đổi phải đối chiếu owner source và regression.


Stack giữ nguyên: Rust workspace, Tauri 2, React/TypeScript/Vite. `src/api.ts` là biên
IPC frontend. Không thêm một control plane riêng để đi vòng ownership/admission hiện có.

Đăng bài dùng `publish_start` để lưu receipt trước chuẩn bị chậm và trả ACK;
`publish_start_status` đối soát cùng request ID sau mất phản hồi. Migration 48
giữ receipt khởi chạy và yêu cầu loại assignment. Restart không tự phát lại
Post cho receipt chưa rõ kết quả. Preflight phát `publishPreflightProgress`
theo từng máy, tối đa bốn probe đồng thời; bản chuẩn bị tối đa 30 giây chỉ được
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

Publish observation dùng lỗi deadline có kiểu và một lượt đọc lại transient trong
cùng phase/deadline; tạo cursor mới không làm mới ngân sách. `SessionEpochChanged`
giữ nguyên kiểu để chủ phase/dispatcher bỏ target cũ và xác minh lại account, không
chấp nhận observation của phiên mới dưới binding cũ. Caption Unknown được đọc lại;
nội dung khác thật hoặc mơ hồ vẫn từ chối. Bước nhạc có một deadline 180 giây chung
cho các attempt/backoff; hết tổng budget không tự requeue để cấp lại cửa sổ mới.

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
campaign và serial. Nó dùng cùng ngân sách 3 lần không tiến triển và nhịp 5 phút;
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

## Xác minh Publish và kết quả qua restart

VerificationQueue dùng giới hạn máy chủ, mặc định4 observer, một observer mỗi máy.
Cleanup media chạy worker riêng; một máy đọc chậm không chặn dispatcher hoặc Sheet.
Candidates được phân trang hữu hạn, ưu tiên bài tới hạn và luân phiên giữa các máy.
Nhãn thời gian own-post chấp nhận EN và VI (`N phút trước` / `vừa xong`).
`evidence.verificationStatus` giữ reasonCode, attempts, readFailures, checkedAt và
nextCheckAt. Bài còn thiếu link được kiểm sau300giây tính từ cuối lượt trước khi còn
ngân sách. `verificationBudget` version1 lưu fingerprint gắn immutable intent,
publicationStage, observations và noProgressObservations trong cùng CAS với kết quả.
Sau3lượt liên tiếp không có bằng chứng mới của đúng bài, lưu needsReview với cause
`verificationNoProgress`, nextCheckAt=null và giữ intent/media/khoản thiếu Sheet.
Lỗi đọc/transport, processing và tìm kiếm đều tiêu ngân sách hữu hạn này; offline/busy
chưa được quan sát không tính là một lượt. Lượt cũ chưa có budget bắt đầu từ lần quan
sát đầu của contract mới, không lấy attempts lịch sử làm số lượt không tiến triển.
Stop chờ primitive đã được cấp quyền hoàn tất, không huỷ giữa khôi phục IME.
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
restart, không phát lại Post.
Worker lưu `nextCheckAt` bằng thời điểm kết thúc quan sát cộng 300 giây cho bài
còn ngân sách, kể cả lỗi đọc. Mỗi lần quan sát có deadline và lease riêng. Restart
giữ mốc gửi, lần kiểm tiếp, fingerprint và số lượt không tiến triển; needsReview
không được background tự mở lại. Nhật ký verifying chỉ nói kiểm thao tác Đăng đã có;
`checking_existing_post_link` ghi lần kiểmN, `link_pending` hiện lý do thiếu link,
`link_needs_review` nói tự kiểm đã dừng. Không suy submitted/công khai từ thiếu link.
Android ở trạng thái Connected sau restart vẫn được kiểm khi agent đã sẵn sàng;
worker mở session qua control plane, không đòi mở điều khiển bằng tay để đổi
trạng thái thành Ready. Máy Busy/Preparing/Error/offline vẫn chờ; iOS giữ điều kiện Ready.
Với receipt Android `submitted`/`posted` chưa có link, `verification_restart` tắt đúng
package đã ghi trong intent, kiểm proof hết tiến trình, mở lại và kiểm tiến trình đang
chạy trước Copy. Cùng lease giữ suốt chu kỳ; kiểm revision/stop trước và sau các await,
không đóng máy còn bài khác đang giữ. Proof restart ghi vào `verificationDiagnostic.appRestart`.
Android hẹn giờ bắt đầu kiểm khi phiên đăng nhả máy; lỗi restart cũng giữ nhịp 300 giây.
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
ban đầu. Cause `verificationNoProgress` cũng cho phép resume rõ ràng; các review
khác không được nới. Transaction lưu status/budget/review-before-stop cũ trong
`verificationBeforeResume` rồi mở budget mới; gọi lặp/ACK replay không reset.
Worker hiện có dùng `nextCheckAt = checkedAt + 300 giây` khi còn ngân sách; ba lượt
không tiến triển liên tiếp lại needsReview. **Kiểm tra liên kết** một lần có thể
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
