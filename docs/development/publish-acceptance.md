# Nghiệm thu Publish

[Hướng dẫn phát triển](../developer-guide.md). Chỉ chạy mode/phạm vi được cho phép.

## Nghiệm thu Publish qua ứng dụng đang chạy

`scripts/publish_acceptance.mjs` chỉ nối CDP loopback vào **một WebView Tauri đang
chạy**, rồi gọi IPC production. Không mở app/browser, không focus/click màn hình,
không mở driver/ADB, không đọc SQLite hay credential. Cần Node và Playwright đã cài
trong `apps/desktop/node_modules`. Không bật/restart debug app thứ hai để nghiệm thu
khi production đang giữ máy. CDP không có sẵn thì dừng; việc chạy script không tự
cấp quyền tạo thêm lượt public.
Khi nghiệm thu Android cắm USB, thêm `--real-android true`: harness đối chiếu từng
serial với roster Android USB `connected/ready` của chính AppState trước mọi
preflight/Start. Thiếu máy, app đang chạy mock hoặc roster cũ đều bị từ
chối; `inspect` không mang cờ này chỉ là phép đọc trạng thái, exit 0 không chứng
nhận máy thật. Cờ được ghim trong fingerprint của preflight và submit.

| Mode | Hành vi | Điều không thực hiện |
|---|---|---|
| `inspect` (mặc định) | Đọc roster + device metadata; nếu có campaign ID/receipt thì đọc `publish_get` | Không preflight, quét nguồn, Sheet check, mở thiết bị hoặc mutate |
| `preflight` | Giữ UUID trong `preflight-request.json`, kích hoạt scope đúng UUID/máy trước `publish_preflight(requestId)`; kiểm OAuth writer/target/epoch và username | Không Start/Post |
| `submit` | Đọc lại account, writer và scope; fsync `start-intent.json` trước một `publish_start`, rồi đọc `publish_start_status` | Không phát lại Start khi intent đã tồn tại |
| `observe` | Đọc `publish_start_status` rồi poll `publish_get` đến hạn; report cũ vẫn đọc theo đường cũ | Không gọi kiểm link chủ động, retry, resume, terminate hoặc Post |

`publish_sheet_check` có thể lưu cấu hình kết nối đã xác minh và làm network I/O;
chỉ gọi trong preflight/submit, **không** thuộc inspect read-only. Script không chuẩn
bị/reset Sheet, đổi writer hay lấy token. OAuth direct phải active và đúng file,
check phải xác minh đúng gid/epoch, preflight phải có Sheet enabled + target v2.
Username gán sẵn là ràng buộc bổ sung, không bắt buộc để Đăng. Nếu metadata trống,
preflight đọc username trên máy; không đọc rõ thì dừng, không tự điền metadata.
Submit chỉ Start nếu username đọc lại khớp snapshot đã duyệt. Lượt harness
mới yêu cầu dọn media nhập tạm sau khi xác minh bài và Sheet; nguồn gốc vẫn giữ.
Nhạc/caption lấy từ `--content-snapshot` và bundle, không tự sửa nội dung hoặc cấu
hình fleet.

Ví dụ dưới dùng biến do người vận hành điền từ roster và nguồn đã quét trong UI;
`$udids` và `$bundleIds` là chuỗi ID phân cách dấu phẩy **cùng thứ tự một-một**, không
phải số máy hoặc index. `$reportDir` dành riêng cho một lượt. Đọc hết `report.json`,
đặc biệt `roster.excluded`/issues, trước khi duyệt. Script không thu nhỏ tập khi một
máy bị chặn. Số máy lấy từ metadata, thiếu thì `null`, không đoán theo vị trí.

```powershell
# Chỉ đọc ứng dụng hiện có; port phải là CDP loopback đã được phép.
node scripts/publish_acceptance.mjs --report-dir $reportDir --udids $udids --cdp $cdp

# Có đọc thiết bị/network qua AppState production; không tạo bài.
node scripts/publish_acceptance.mjs --mode preflight --report-dir $reportDir --udids $udids --source $source --bundle-ids $bundleIds --sheet-id $sheetId --sheet-gid $sheetGid --dev-scope $scopeFile --real-android true --cdp $cdp

# CHỈ sau khi phạm vi/nội dung/số bài được người vận hành cho phép đăng public.
# $confirmation là đúng hash report.confirmation từ preflight trên.
node scripts/publish_acceptance.mjs --mode submit --report-dir $reportDir --udids $udids --source $source --bundle-ids $bundleIds --sheet-id $sheetId --sheet-gid $sheetGid --dev-scope $scopeFile --real-android true --cdp $cdp --confirm $confirmation

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

Lượt mới mặc định dùng giao thức Start; `--protocol legacy` không cho preflight/submit
qua CLI. Scope dev ban đầu phải có đúng danh sách máy và `campaignIds: []`. Harness
ghi độc quyền `preflight-request.json` với UUID; kích hoạt scope chứa đúng UUID ấy
trước khi gọi preflight. Scope đã kích hoạt là một chiều trong process này; nếu
preflight lỗi, không đổi campaign ID/capability để thử lại, cần lượt nghiệm thu mới
và đối chiếu owner/intent cũ. Preflight lưu `preflight.json` với requestId, digest,
preparationId, writer/epoch, account và hash duyệt. Submit ghi `start-intent.json`
exclusive + fsync trước `publish_start`; nếu mất ACK, chỉ đọc
`publish_start_status(requestId)`, không phát lại Start hoặc đổi requestId/thư mục.
Khi status có campaign ID, phải khớp ID đã duyệt rồi mới đọc campaign. Report cũ
có `create-intent.json`/`execute-intent.json` vẫn được observe, nhưng không
được submit theo đường cũ. Report directory có lock chống chạy
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
