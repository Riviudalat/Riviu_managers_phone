## 2. Đọc mục này TRƯỚC KHI sửa bất cứ thứ gì liên quan tới WDA

Đây là những ràng buộc đã trả giá bằng nhiều giờ live test. Vi phạm là hỏng phiên.

### 2.1 KHÔNG bật `autoDismissAlerts` trong session capabilities

Cờ này bắt WDA chạy alert-monitor nền, liên tục quét accessibility hierarchy.
Với TikTok, truy vấn đó **không bao giờ trả về** và khoá luôn luồng XCTest phục vụ
mọi gesture. Triệu chứng đánh lừa: `/status` và `GET /screenshot` (sessionless) vẫn
OK, `POST /session` vẫn OK trong ~5 ms, nhưng **mọi lệnh session-scoped timeout**.
Đây là nguyên nhân gốc của toàn bộ chuỗi "tap chết / swipe blocked / recovery 2–3 phút".

Xem `crates/ios-driver/src/wda.rs::session_capabilities()`.

### 2.2 Stock WDA PHẢI prime session trước mọi lệnh khác

Ngay sau `POST /session`, gửi `POST /session/{id}/appium/settings` với
`snapshotMaxDepth: 1`. Không prime → lệnh hierarchy đầu tiên treo → runner kẹt.

Đo được (runner mới, TikTok foreground): không prime = timeout 4/4; có prime =
`window/size` 107–690 ms, tap 393–601 ms, pass 4/4.

Xem `wda.rs::prime_session()`.

### 2.3 `snapshotMaxDepth` của stock WDA PHẢI là 1

Đặt 20 hoặc 50 → lệnh kế tiếp treo ngay (đã thử cả hai). Đây là ràng buộc cứng.
Hệ quả: **không dùng được element finding** (TikTok không lộ TextField/TextView ở
depth 1), và ô nhập bình luận không focus được (xem §5).

### 2.4 Thứ tự khởi động: session TRƯỚC, stream SAU

Stock: `run_session` tạo + prime WDA session rồi mới `ensure_stream`. RT-MMO
không prime; phiên có comment phải đi đúng chuỗi đã live-confirm ngày 28/07:
**bootstrap agent mới -> foreground TikTok -> `POST /session` mới -> stream**.
MJPEG vẫn luôn đứng sau session. RT-MMO dùng mode `mjpeg` bắt buộc và chỉ báo
stream sẵn sàng sau frame đầu tiên; không fallback âm thầm sang DVT screenshot.
Bật stream trước làm lệnh session đầu tiên treo.

### 2.5 Không bọc request WDA bằng `tokio::time::timeout`

Huỷ request giữa chừng làm relay tidevice wedge. Deadline phải nằm trên chính
request (`req.timeout(...)`). Xem `wda.rs::send()`.

### 2.6 Không tạo session với `bundleId=SpringBoard` hoặc `forceAppLaunch=true`

Gây nháy lock screen / Home.

### 2.7 Chỉ recycle transport khi gesture thật lỗi với lớp transport

Health probe (`/status`) false-negative dưới tải USB. Recycle vì probe = giết một
agent đang sống, mất 2–3 phút. Phân loại lỗi ở `crates/core/src/driver.rs::UiErrorKind`.

### 2.8 Không `pkill` rộng

Kill theo PID **và** kiểm tra command-line khớp fingerprint. Xem
`supervisor.rs::kill_if_matches()`.

### 2.9 Tắt 3uTools khi test

3uTools tự chạy XCTest Runner (`notes.3u`) trên máy. iOS chỉ cho một XCTest session
→ runner của ta lên được HTTP nhưng luồng test bị chặn. Đây là nguồn nhiễu xuyên
suốt các vòng test cũ.

```bash
tidevice -u <UDID> kill notes.3u
```

### 2.9.1 Windows phải sở hữu cả cây tiến trình bằng Job Object

`TerminateProcess` / `Child::start_kill()` chỉ dừng đúng process cha; Python và
`tidevice relay` có thể tiếp tục giữ usbmux/port sau khi desktop bị force-stop.
Desktop và `live_nurture_test` phải gọi
`riviu_ios_driver::install_process_tree_guard()` trước khi spawn bất kỳ child nào.
Guard gắn process root vào Windows Job Object có
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; `riviu_pmd.py wda-proxy` đồng thời giữ một
Job Object lồng cho riêng relay. Không dựa vào signal handler Python để dọn child
trên Windows. Registry thu hồi lần chạy sau vẫn phải kiểm tra PID + command-line
fingerprint; nhánh Windows dùng CIM và chỉ terminate process object đã khớp.

Mọi process console do desktop tạo (Python, PowerShell, tidevice) phải đi qua
`process_tree::background_command()` hoặc wrapper `_background_popen()` /
`_background_run()` của sidecar để gắn `CREATE_NO_WINDOW`. Không đổi sang
`pythonw`: protocol vẫn cần stdout/stderr pipe. Thiếu cờ ở bất kỳ nhánh scan,
stream hay recovery nào sẽ làm cửa sổ CMD nhấp nháy lặp lại trên Windows.

### 2.10 RT-MMO là backend riêng, không trộn với stock WDA

`crates/ios-driver/src/wda.rs::WdaProfile` chọn đúng một profile cho toàn bộ vòng
đời. RT-MMO dùng bundle `com.mrph.svc`, control `8906`, MJPEG `9093`; nó không
prime `snapshotMaxDepth` và không gọi `/wda/window/size` (endpoint này trả 404).
Liveness/reuse thường có thể attach session từ `GET /status`, nhưng **job bật
comment không được reuse đường đó**: status session vẫn gesture được nhưng có thể
ACK `/wda/keys` mà bỏ chữ. `start_fresh_text_session()` giữ relay USB, dừng stream
cũ, bootstrap riêng agent, foreground TikTok, rồi `POST /session` mới.

`RIVIU_WDA_BACKEND=rt-mmo` bắt buộc có `RIVIU_RTMMO_TOKEN`; thiếu token phải báo
lỗi cấu hình, không được rơi ngầm về stock. Token truyền sang sidecar bằng
environment, không nằm trong argv, registry, source hay trace. Mọi request RT-MMO
phải có `X-RT-Token`. Agent được launch qua `ProcessControl.launch(environment=...)`,
không qua `tidevice -e FARM_KEY:...` vì cách đó lộ token trong OS process argv.
Alias `rtmmo`, khoảng trắng bao quanh và mọi giá trị backend không hợp lệ đều
phải được parser xử lý/đẩy lỗi nhất quán; chỉ unset, `auto`, hoặc `stock` mới được
phép degraded khi probe sidecar lỗi. Stream và control luôn dùng cùng profile.

Gesture RT-MMO phải dùng sessionless `POST /wda/swipe`; tap là swipe lệch 1 px
với `delay=0.05`. Không dùng W3C `/session/{sid}/actions`: live test 28/07/2026
đo được touch đầu timeout 10 s hai lần rồi làm session biến mất. Profile stock
vẫn giữ `/actions` vì ràng buộc quiescence của stock WDA khác RT-MMO.

Sidecar launch RT-MMO bằng app launch với đủ `USE_PORT=8906`,
`MJPEG_SERVER_PORT=9093`, `FARM_KEY=<token>`. `ensure_stream` luôn attach session
trước khi mở MJPEG, kể cả khi được gọi từ initial device scan của desktop. Sau
**mọi** cold launch/restart và trước khi reuse/emit ready, phải gọi route bảo vệ
`/wda/locked` để xác thực token; `/status` được miễn auth nên không phải bằng
chứng. Đồng thời port MJPEG `9093` phải mở; control-only hoặc sai auth phải
relaunch có giới hạn với đủ env.

Mọi app launch từ sidecar cũng đi trực tiếp qua pymobiledevice3 DVT; không spawn
`tidevice launch` rồi để child tiếp tục chạy sau khi Rust đã hết deadline.
Sidecar pin `pymobiledevice3==10.1.0`: code dùng async `DvtProvider` /
`ProcessControl` của API này, không tương thích dòng 4.x/5.x sync. Nếu lần launch
RT đầu không bind/auth/MJPEG, phải kill đúng `com.mrph.svc`, chờ port 8906 đóng,
rồi mới launch lần hai để env mới thực sự được áp dụng.

Khi stream hiện tại còn sống, `ensure_stream` trả reuse ngay và **không** probe
session stock bằng `/window/size`; false-negative ở probe đó có thể tạo session
mới trong lúc MJPEG đang chạy và vi phạm session-trước-stream. Mỗi stream reader
giữ một generation; `clear()` tăng generation nên reader cũ dù còn byte buffered
cũng không được publish frame sang cache/broadcast của stream mới.

---
