# Claude Code handoff — ổn định Nuôi TikTok trên iPhone

Đọc toàn bộ file này trước khi sửa. Không chỉ phân tích hoặc đề xuất: hãy kiểm tra code hiện tại, sửa theo thứ tự ưu tiên, chạy test thật, đọc log, tiếp tục sửa và cuối cùng cập nhật báo cáo.

## Mục tiêu bắt buộc

1. Bấm **Bắt đầu Nuôi TikTok** khi TikTok đã mở sẵn:
   - Không lock/unlock màn hình.
   - Không nháy Home/SpringBoard.
   - Không launch/restart TikTok nếu app vẫn foreground và dùng được.
2. Popup TikTok xuất hiện bất ngờ giữa phiên phải được nhận diện chính xác và tự đóng:
   - Trang **Chọn chủ đề** → bấm **Bỏ qua**.
   - Sheet **Add phone** và popup có nút X → bấm đúng tâm nút X.
   - Không dùng lịch “mỗi N video”.
   - Không spam `GET /screenshot` qua WDA.
   - Không tap mù nếu detector không chắc chắn.
3. Like, follow và swipe phải chạy ổn định, không làm WDA chết rồi recovery 2–3 phút.
4. Log UI phải nói rõ: app đã mở sẵn hay vừa được launch, popup nào được phát hiện/đóng, action nào thành công/thất bại, recovery nào đang diễn ra.
5. Thiết kế phải độc lập theo UDID để sau này nhiều thiết bị không tranh relay/port/session.

## Thiết bị live test

- Model: iPhone 8
- iOS: 16.7.15
- UDID: `a99f4bd9f877b2a0e3682ee24fd1c68f75ba6982`
- TikTok bundle: `com.ss.iphone.ugc.Ame`
- WDA runner: `com.riviu.managersphone.agent.xctrunner`
- `tidevice`: `$HOME/Library/Python/3.9/bin/tidevice`

## Stack và file cần đọc trước

- `crates/core/src/nurture.rs` — flow Nuôi TikTok, popup, launch/recovery.
- `crates/core/src/screen_match.rs` — NCC template matching.
- `crates/core/assets/tiktok_close_x.png` — template nút X.
- `crates/core/src/driver.rs` — `DeviceDriver`/`UiSession`.
- `crates/ios-driver/src/wda.rs` — tap/swipe/screenshot/health.
- `crates/ios-driver/src/pmd.rs` — WDA relay/session/process ownership.
- `crates/ios-driver/src/stream.rs` — `StreamHub`, có `latest()` và `subscribe()`.
- `sidecars/pymobiledevice3/riviu_pmd.py` — launch, stream, WDA proxy.
- `apps/desktop/src-tauri/src/state.rs` — có cả `NurtureEngine` và `StreamHub`.
- `apps/desktop/src/components/NurturePopup.tsx` — status/log UI.
- `apps/desktop/src-tauri/src/bin/live_nurture_test.rs` — harness test thật.
- `TOOL TIKTOK/scripts/tiktok/shared/wda_watchdog.py` — cách tool tham khảo match nút X.
- `TOOL TIKTOK/scripts/tiktok/shared/wda_screen.py` — OpenCV multi-scale NCC.
- `docs/LIVE_NURTURE_REPORT_2026-07-26.md` — báo cáo vòng test cũ.
- Log thô: `/tmp/riviu-live/test1.log` đến `/tmp/riviu-live/test11.log`.

## Các lỗi đã xác nhận bằng live test

### P0 — WDA tap thường chết

Triệu chứng:

```text
like fail: WDA request failed (.../wda/tap): error sending request for url
```

Hiện tại `WdaClient::tap()` trong `wda.rs` gọi `/wda/tap` trước. Nếu lỗi được phân loại là transport error, code trả lỗi ngay và không thử `/actions`. Trong khi swipe đã dùng `/actions` làm primary vì `dragfromtoforduration` chờ app quiescence và hay treo trên TikTok.

Việc cần làm:

- Benchmark trực tiếp `/wda/tap` và W3C `/actions` tap trên máy thật.
- Ưu tiên `/actions` cho tap nếu nó ổn định hơn, giống swipe.
- Retry tối đa một lần với session hiện tại hoặc soft session recreate; không hard-recycle ngay.
- Tách lỗi HTTP 4xx/5xx, session invalid và lỗi relay/transport. Không gom tất cả thành “WDA unhealthy”.
- Action chỉ được tính thành công sau khi request thực sự hoàn tất; nếu có thể, xác nhận bằng thay đổi icon từ frame stream.

### P0 — Swipe blocked và recovery quá lâu

Triệu chứng:

```text
swipe blocked — clear + retry
WDA unhealthy after reopen
WDA reopen failed
```

Một lần lỗi có thể làm phiên đứng 130–210 giây. `reopen_session()` soft 60 giây rồi hard recycle 120 giây. Health probe từng false-negative và tự giết một WDA vẫn sống.

Việc cần làm:

- Không hard-recycle chỉ vì `/status` hoặc `/window/size` chậm.
- Chỉ recycle sau một lệnh HID thật (`tap`/`swipe`) thất bại với lỗi transport đã được xác nhận.
- Đặt recovery budget rõ ràng, ví dụ soft recreate ≤10–15 giây, hard recycle chỉ một lần.
- Khi hết budget phải log lỗi cụ thể và chuyển thiết bị sang trạng thái lỗi, không treo im lặng.
- Xác nhận swipe bằng frame stream thay đổi, không dùng thêm WDA screenshot.

### P0 — Popup “event-driven” chưa thật sự được giải quyết

Yêu cầu của user là popup vừa xuất hiện thì tự đóng, không đợi mỗi 3/4 video. Những cách đã thử:

- WDA screenshot mỗi 1.5–2.5 giây → relay USB dễ wedge.
- WDA screenshot sau mỗi video → vẫn chậm và có lần wedge.
- Poll mỗi N video → user không chấp nhận.
- Tool TikTok tham khảo bản v2.8 chỉ chạy watchdog lúc mở app; popup giữa phiên không tự đóng. Không copy hạn chế này.

Hướng nên triển khai:

- Dùng **frame stream đang có sẵn**, không gọi WDA `GET /screenshot` để quan sát.
- `StreamHub` đã publish JPEG theo UDID và có `subscribe()`. Hãy tạo detector nhận frame từ broadcast callback; đây là xử lý theo frame event của stream, không phải timer “mỗi N video”.
- Không để `riviu-core` phụ thuộc ngược vào `riviu-ios-driver`. Định nghĩa abstraction/frame source trong core hoặc inject receiver/provider từ Tauri state.
- Mỗi UDID có task detector riêng, cancel token riêng, debounce/cooldown riêng.
- Chỉ decode/match khi frame thay đổi đủ lớn; có thể giới hạn xử lý 2–4 FPS bằng frame coalescing, nhưng không dùng logic “mỗi N video”.
- Nút X:
  - NCC grayscale multi-scale.
  - Template hiện tại: `tiktok_close_x.png`.
  - Ngưỡng đã đo: Add phone `0.988`; chọn chủ đề `0.459`; feed `0.428`.
  - Ngưỡng hiện tại `0.85`.
  - Giới hạn vùng bên phải màn hình.
  - Yêu cầu 2 frame liên tiếp cùng vị trí trước khi tap.
- Trang chọn chủ đề:
  - Không chỉ dựa vào brightness vì video trắng có thể false-positive.
  - Tạo template/OCR cho “Bỏ qua” hoặc tổ hợp nhiều đặc trưng độc lập.
- Khi detector chắc chắn, gửi đúng một WDA `tap_image()` theo tọa độ frame; cooldown rồi xác nhận popup biến mất bằng frame tiếp theo.
- Log score, loại popup và tọa độ trong debug; UI chỉ hiện thông điệp ngắn.

### P0 — Launch TikTok và WDA đang xung đột

Các kết quả trái chiều cần giải quyết đúng, không hard-code theo một test:

- Launch TikTok sau khi WDA relay đã chạy từng làm usbmux/relay chết.
- Start WDA/xctest có lúc làm TikTok mất foreground.
- Code hiện tại vì vậy lại launch TikTok vô điều kiện trước mỗi UI session.
- Điều này chưa đạt yêu cầu “TikTok đang mở thì không khởi động lại”.

Hướng xử lý:

1. Nếu có WDA healthy đang được app sở hữu, reuse relay/session; không start xctest mới và không launch app.
2. Nếu chưa có WDA:
   - Dùng frame stream hoặc API ngoài WDA để xác định TikTok đang foreground.
   - Start WDA đúng một lần, không gắn `bundleId` trong capabilities và không `forceAppLaunch`.
   - Kiểm tra frame sau khi WDA sẵn sàng.
   - Chỉ bring TikTok foreground nếu frame xác nhận đã lệch app.
3. Nếu bắt buộc launch trong khi relay sống, serialize/pause đường USB hoặc tìm API activate ổn định; không chạy `tidevice launch` song song với relay/xctest.
4. Không launch TikTok mù lúc kết thúc phiên. Chỉ park app nếu detector thấy đã rời TikTok.

### P1 — Nhiều relay/zombie cùng một UDID

Đã quan sát đồng thời các relay `18100`, `18101`, `18102`, nhiều `riviu_pmd.py wda-proxy` và `tidevice xctest` cho cùng thiết bị. Đây là nguồn USB contention.

Việc cần làm:

- Một per-UDID supervisor sở hữu duy nhất:
  - một WDA runner,
  - một control relay,
  - một stream process,
  - một cached session.
- Dùng lock theo UDID cho start/stop/recycle/launch.
- Reuse port/process khỏe; kill stale child process do chính app tạo.
- Không dùng `pkill` rộng trong code production.
- Khi app restart, thu hồi child process cũ có PID/ownership record.
- Test đồng thời ít nhất 2 logical jobs trên cùng UDID: job thứ hai phải bị queue/reject, không tạo relay thứ hai.

### P1 — Code hiện tại có workaround nguy hiểm hoặc sai nghĩa

Trong `nurture.rs` hiện tại:

- `ensure_ready()` luôn `Ok(())`; WDA chết có thể bị che giấu tới action kế tiếp.
- `watch_and_clear_popups()` chỉ sleep; tên hàm gây hiểu nhầm.
- UI/status vẫn ghi `round ... clear popups`, nhưng `ensure_tiktok_on_session()` hiện không thực sự quét popup.
- `clear_onboarding_screens()` chạy `for attempt in 0..2`, nhưng có nhánh `if attempt == 2`; nhánh đó không bao giờ chạy.
- `clear_overlays_quick()` chỉ dismiss system alert, không phải TikTok popup.
- Cuối phiên từng gọi `launch_app` mù để park TikTok trong khi WDA/relay còn sống.
- `tiktok_on_screen()` dựa vào brightness vài vùng và WDA screenshot; dễ nhầm và gây tải relay.

Hãy loại bỏ workaround sau khi có thiết kế stream detector/recovery đúng. Không giữ tên/status giả.

### P1 — Debug build làm NCC rất chậm

Template matcher Rust chạy chậm rõ rệt ở debug. Live test release nhanh hơn. Tuy nhiên không được chỉ “chạy release để che thuật toán chậm”.

Việc cần làm:

- Profile `screen_match.rs`.
- Chỉ crop ROI trước, downscale hợp lý, cache decoded template.
- Tránh clone/decode template mỗi lần gọi.
- Có benchmark detector trên frame iPhone 8.
- Dev build vẫn phải phản hồi hợp lý.

### P1 — Log chưa phản ánh trạng thái thật

Log phải phân biệt:

- `TikTok đã mở sẵn — reuse`
- `WDA đã có — reuse`
- `Khởi động WDA mới`
- `Đã bring TikTok foreground` (không gọi là restart nếu không terminate)
- `Phát hiện Add phone score=...`
- `Đã đóng popup` / `Xác nhận popup biến mất`
- `Tap like thất bại: transport/session/http`
- `Soft recovery x/y giây`
- `Hard recovery x/y giây`
- Kết thúc phải có tổng videos/likes/comments/follows, thời gian, số popup, số recovery và lỗi cuối.

Không ghi `done` nếu phiên hết giờ nhưng chỉ xử lý được 0–1 video vì WDA treo. Khi đó phải là `failed` hoặc `partial`.

### P2 — Harness test còn hạn chế

`live_nurture_test.rs` hiện hard-code xác suất và chưa đo đủ:

- Thêm flags: `--like-prob`, `--follow-prob`, `--comment-prob`, `--watch-min`, `--watch-max`.
- Ghi JSONL timestamps và latency cho mỗi WDA request/recovery.
- Exit code khác 0 nếu 0 video, vượt recovery budget hoặc kết thúc `partial/failed`.
- Không start desktop app cùng lúc với harness vì hai process sẽ tranh USB.

## Các cách đã thử và không được lặp lại

1. Không bọc request WDA bằng `tokio::time::timeout` rồi cancel giữa request; relay từng wedge do cancellation. Timeout phải nằm trong HTTP client/request.
2. Không dùng `/wda/dragfromtoforduration` làm primary cho TikTok; app không quiescent.
3. Không tạo session với `bundleId=SpringBoard` hoặc `forceAppLaunch=true`; gây nháy Home/lock.
4. Không tap tọa độ “safe” trên SpringBoard; từng mở nhầm Calendar.
5. Không gọi accessibility tree/find element dày; từng làm WDA chậm/wedge.
6. Không WDA-screenshot liên tục để làm popup watcher.
7. Không hard-recycle runner vì một health probe false-negative.
8. Không chạy nhiều `wda-proxy`/relay cho cùng UDID.

## Thứ tự triển khai đề nghị

1. Viết instrumentation request latency + process/port ownership trước.
2. Sửa tap: thử `/actions` primary và retry/recovery có budget.
3. Làm per-UDID WDA supervisor, đảm bảo một runner/relay/session.
4. Inject `StreamHub` frame source vào nurture.
5. Chuyển foreground classifier và popup detector sang frame stream.
6. Dọn workaround/status sai trong `nurture.rs`.
7. Test popup bằng ảnh fixture và frame thật.
8. Chạy smoke release 5–10 video.
9. Sửa lỗi phát hiện được, chạy lại.
10. Khi smoke đạt, chạy live test 30 phút và viết báo cáo cuối.

## Test bắt buộc

### Unit/integration

- Feed bình thường không match nút X.
- Add phone match score ≥0.85, tap đúng tâm X.
- Chọn chủ đề không bị nhầm với X; nhận đúng nút Bỏ qua.
- Video nền trắng không bị nhận là onboarding.
- Hai frame liên tiếp mới phát event; cooldown ngăn double tap.
- Tọa độ stream @2x/@3x map đúng sang WDA points.
- Per-UDID lock không tạo process thứ hai.
- Transport error của tap không kéo theo hard recycle ngay.

### Live smoke

Trước khi chạy, bảo đảm desktop app/harness khác không chiếm thiết bị. Kiểm tra process có mục tiêu, không `pkill` mù.

```bash
export PATH="$HOME/Library/Python/3.9/bin:$PATH"
cd /Users/levanlanh/Desktop/Din/Riviu_managers_phone
cargo build -p riviu-managers-phone --bin live_nurture_test --release
./target/release/live_nurture_test \
  --udid a99f4bd9f877b2a0e3682ee24fd1c68f75ba6982 \
  --minutes 5 \
  --videos 10
```

Sau smoke:

- Chạy lại khi TikTok đã mở sẵn.
- Chạy lại khi đang ở Home.
- Cố tình để xuất hiện Chọn chủ đề/Add phone nếu có thể.
- Kiểm tra không có Calendar, lock screen hoặc Home flash.
- Kiểm tra số relay/xctest trước, trong và sau test.

### Tiêu chí đạt trước live 30 phút

- 10 video liên tục.
- Ít nhất 3 tap like thành công.
- Ít nhất 8/9 swipe chuyển video thành công ngay lần đầu.
- Không hard recycle.
- Không request WDA nào treo quá 12 giây.
- Không có relay/xctest trùng UDID.
- Popup fixture và popup thật (nếu xuất hiện) được đóng đúng, không false tap.

### Live 30 phút

- Chạy đủ 30 phút.
- Không lock/unlock, không về Home, không mở Calendar.
- Không restart TikTok nếu app vẫn foreground.
- Popup được xử lý trong vài giây từ frame event.
- Kết thúc vẫn ở TikTok.

## Báo cáo Claude Code phải cập nhật

Tạo/cập nhật `docs/LIVE_NURTURE_REPORT_2026-07-26.md` với:

1. Root cause từng lỗi.
2. File/hàm đã sửa.
3. Các test và thời lượng thực tế.
4. Latency p50/p95 cho tap, swipe, popup detection và recovery.
5. Số video/like/follow/comment/popup/recovery.
6. Process/port theo UDID trước và sau.
7. Lỗi còn lại, không được ghi “đã xong” nếu chưa đạt tiêu chí.

