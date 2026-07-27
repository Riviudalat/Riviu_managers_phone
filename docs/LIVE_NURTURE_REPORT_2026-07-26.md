# Báo cáo live test Nuôi TikTok — 26/07/2026

Thiết bị: iPhone 8 · iOS 16.7.15 · UDID `a99f4bd9…ba6982`
App: `com.ss.iphone.ugc.Ame` · Agent: `com.riviu.managersphone.agent.xctrunner`
Harness: `live_nurture_test` (release). Log thô `/tmp/riviu-live/`, frame dump `/tmp/riviu-frames/`.

---

## 1. Nguyên nhân gốc từng lỗi

### 1.1 P0 — WDA tap/swipe chết, recovery 2–3 phút

**Nguyên nhân thật: capability `autoDismissAlerts` / `defaultAlertAction`.**

Cờ này khiến WDA cài một *alert monitor* chạy nền, liên tục truy vấn accessibility
hierarchy của app đang foreground. Với TikTok (cây accessibility rất sâu) truy vấn đó
**không bao giờ trả về**, và nó khoá luôn luồng XCTest — cũng chính là luồng phục vụ
mọi gesture. Hệ quả:

- `/status` và `GET /screenshot` (sessionless) vẫn trả lời bình thường → mọi health
  probe đều báo "agent khoẻ".
- `POST /session` vẫn thành công trong ~5 ms.
- **Mọi lệnh session-scoped** (`window/size`, `/actions`, `/wda/tap`) timeout.

Đây là lý do các vòng test cũ thấy "WDA unhealthy", recycle một runner vốn đang sống,
rồi lặp lại — mất 2–3 phút mỗi lần mà không sửa được gì.

Đo trực tiếp trên máy, runner mới, TikTok foreground:

| Lệnh session đầu tiên | window/size | tap |
|---|---:|---:|
| `window/size` | timeout (12–20 s) | — |
| `/actions` tap | — | timeout |
| `appium/settings` (prime) rồi window/size | 107–690 ms | 393–601 ms |

4/4 lần chạy có prime đều pass; mọi lần không prime đều kẹt. Sau khi **bỏ hẳn**
`autoDismissAlerts` và thêm bước prime, phiên live chạy 18/18 video, 0 lỗi request.

Sửa tại `crates/ios-driver/src/wda.rs`:
- `session_capabilities()` — bỏ `autoDismissAlerts` + `defaultAlertAction`.
- `create_session()` → `prime_session()` — gửi `POST /session/{id}/appium/settings`
  (`snapshotMaxDepth: 1`, `customSnapshotTimeout: 2`) trước mọi lệnh khác; retry 4×
  rồi báo lỗi `Timeout` thay vì trả về một session đã chết.

### 1.2 P0 — Toạ độ like/follow/comment sai vị trí

Code cũ hard-code `like = (0.92, 0.42)`, `follow = (0.91, 0.34)`, `comment = (0.92, 0.52)`.
Đo trên frame thật 750×1334 (logical 375×667):

| Nút | Toạ độ thật (logical) | Code cũ tap vào |
|---|---:|---|
| avatar | 236 | — |
| follow "+" | 263 | **227 → trúng avatar → mở trang profile** |
| tim | 312 | **280 → khoảng trống giữa follow và tim** |
| bình luận | 377 | **347 → khoảng trống** |

Nghĩa là **like chưa bao giờ trúng tim**, còn follow thì mở profile → rời feed →
"vuốt không ăn". Số đo trùng khớp bảng của TOOL TIKTOK (layout 2: 259/313/371).

Sửa: `crates/core/src/screen.rs::find_action_rail()` — dò **badge follow đỏ** trên
frame rồi suy ra tim (+51) và bình luận (+113) theo offset cố định. Tự thích ứng cả
hai layout TikTok; fallback layout 2 khi đã follow (badge biến mất).

### 1.3 P0 — Popup không tự đóng

Thay bằng detector chạy theo **frame event của MJPEG stream** (kênh usbmux riêng, app
vẫn mở sẵn để vẽ tile) — không gọi `GET /screenshot` của WDA, không lịch "mỗi N video".

- `crates/core/src/screen_watch.rs` — task riêng theo UDID, stop token riêng, cooldown riêng.
- Chỉ decode khi digest byte của frame đổi → feed đứng yên tốn 0 CPU.
- Giới hạn 3 FPS phân tích.
- Yêu cầu **2 frame liên tiếp** cùng loại + cùng vị trí (sai số 0.03) mới tap.
- Sau tap: cooldown 1.6 s → xác nhận popup biến mất ở frame sau; tối đa 3 lần rồi báo và dừng.

### 1.4 P1 — Nhiều relay/runner cùng một UDID

Nguyên nhân bổ sung phát hiện được: **3uTools tự chạy XCTest Runner (`notes.3u`) trên
máy**. iOS chỉ cho một XCTest session, nên runner của ta mở được HTTP nhưng luồng test
bị chặn. Đây là nguồn nhiễu xuyên suốt các vòng test cũ.

Sửa: `crates/ios-driver/src/supervisor.rs`
- `DeviceSlot` — một async lock cho mỗi UDID; start/stop/recycle/launch đều trong lock.
- `ProcessRegistry` — ghi PID + fingerprint ra đĩa; khi app khởi động lại thì thu hồi
  child cũ, **kill theo PID và chỉ khi command-line khớp fingerprint** (không `pkill` mù).
- Port relay sticky theo UDID trong dải 18100–18163, không trùng nhau.

### 1.5 P1 — Trôi vào phòng LIVE

Vuốt dọc trong phòng LIVE **không thoát ra** mà cuộn nội dung của phòng, nên phiên kẹt
lại và mọi tap theo toạ độ rail đều vô nghĩa. Thêm `ScreenKind::LiveRoom` (nhận diện
qua pill "+ Follow" đỏ ở đầu phòng: đo được R−G 152–176 trong LIVE so với 34 ở feed);
hành động thoát = bấm ✕ góc trên phải.

---

### 1.6 Hành vi quá đều

Bản cũ roll xác suất độc lập cho từng video, nên nhìn như bot: rải đều 40 % tim
suốt phiên. Thêm `human_behavior.rs::MoodCycle` — mỗi "nhịp" kéo dài vài video:

| Nhịp | Tim | Bình luận | Follow | Thời lượng xem |
|---|---:|---:|---:|---:|
| lướt nhanh | ×0 | ×0 | ×0 | ×0.55 |
| thả tim nhiều | ×2.2 | ×0.5 | ×1.6 | ×1.0 |
| hay bình luận | ×1.2 | ×3.0 | ×1.4 | ×1.45 |

Độ dài nhịp: lướt 4–12 video, tim 3–8, bình luận 2–5; tần suất 50/35/15 %.
Xác suất cấu hình được nhân theo nhịp nên trung bình phiên vẫn bám cấu hình.

---

## 2. File/hàm đã sửa

| File | Thay đổi |
|---|---|
| `crates/ios-driver/src/wda.rs` | Bỏ `autoDismissAlerts`; `prime_session()`; lỗi phân loại `UiError`; telemetry mọi request; `tap_native()` |
| `crates/ios-driver/src/telemetry.rs` | **mới** — p50/p95/max theo endpoint, đếm lỗi theo lớp, JSONL qua `RIVIU_WDA_TRACE` |
| `crates/ios-driver/src/supervisor.rs` | **mới** — `DeviceSlot`, `SlotMap`, `ProcessRegistry`, `OwnedChild` |
| `crates/ios-driver/src/pmd.rs` | Viết lại theo supervisor; port sticky; `ui_session_cached()` |
| `crates/ios-driver/src/stream.rs` | Frame dùng `Arc<Vec<u8>>`; `StreamHub` implement `FrameSource` |
| `crates/core/src/driver.rs` | `UiError`/`UiErrorKind`/`ui_error_kind()`; `tap_native()` |
| `crates/core/src/frame_source.rs` | **mới** — abstraction để core không phụ thuộc ngược vào ios-driver |
| `crates/core/src/screen.rs` | **mới** — phân loại màn hình, `find_action_rail`, comment drawer, LIVE |
| `crates/core/src/screen_watch.rs` | **mới** — watcher popup theo frame event |
| `crates/core/src/screen_match.rs` | NCC coarse-to-fine, cache template, truy cập slice thô |
| `crates/core/src/nurture/` | Viết lại toàn bộ flow, tách 3 module (xem §8) |
| `crates/core/src/human_behavior.rs` | `MoodCycle` — nhịp hành vi theo đợt |
| `crates/core/src/openai_client.rs` | vilao.ai; vision comment + pool dựng sẵn; sanitize |
| `apps/desktop/src-tauri/src/bin/live_nurture_test.rs` | Flags, JSONL, exit code |
| `sidecars/pymobiledevice3/riviu_pmd.py` | `--restart-wda` luôn kill bundle rồi chờ port đóng |

**Đã gỡ các workaround sai nghĩa trong `nurture.rs` cũ**: `ensure_ready()` luôn `Ok(())`,
`watch_and_clear_popups()` chỉ sleep, `clear_onboarding_screens()` có nhánh `attempt == 2`
không bao giờ chạy, `clear_overlays_quick()` chỉ dismiss alert, launch mù cuối phiên,
status `round … clear popups` khi không hề quét popup.

---

## 3. Kết quả live test

| # | Cấu hình | Video | Tim | Follow | Popup đóng | Recovery | Ghi chú |
|---|---|---:|---:|---:|---:|---:|---|
| smoke1 | trước khi tìm ra `autoDismissAlerts` | 6 | 1 | 0 | 1 | 0 | 46 s chết ở đầu phiên |
| smoke2–7 | — | 0 | 0 | 0 | 0 | 1 | runner kẹt, recycle không lên |
| smoke8 | **bỏ `autoDismissAlerts`** | 12 | 2 | 1 | 0 | 0 | 0 lỗi request |
| smoke10 | + thoát LIVE | 12 | 4 | 1 | 2 | 0 | kết thúc ở TikTok |
| final | + tap native cho ô nhập | 18 | 2 | 0 | 15 | 0 | 285 s, 0 lỗi request |
| **live 15p** | + nhịp hành vi | **47** | 1 | 0 | 0 | **0** | 912 s, 0 lỗi request; lộ 2 lỗi logic (§9) |
| **verify 6p** | + sửa 2 lỗi đó | **39** | 2 | 0 | **3** | **0** | 362 s, **39 vuốt → 39 video** |

### Latency (vòng final, release)

| Endpoint | n | p50 | p95 | max |
|---|---:|---:|---:|---:|
| `tap.actions` | 38 | 464 ms | 1198 ms | 1560 ms |
| `swipe.actions` | 18 | 1664 ms | 2474 ms | 2474 ms |
| `tap.native` | 1 | 414 ms | 414 ms | 414 ms |
| `session.create` | 1 | 4 ms | — | 4 ms |
| `session.prime` | 1 | 5 ms | — | 5 ms |
| `window.size` | 1 | 155 ms | — | 155 ms |
| `keys` (gõ text) | 1 | 1041 ms | — | 1041 ms |

- Request chậm nhất cả phiên: **2474 ms** (yêu cầu: không quá 12 s).
- **0 request lỗi** ở mọi lớp (transport/timeout/session/http).
- `relay_start` 8.2 s, **0 hard recycle**.
- Popup detection: 15 popup đóng trong 285 s, không có false tap nào quan sát được.

### Benchmark `/wda/tap` vs `/actions` (20 lần mỗi loại, stack khoẻ)

| Endpoint | n | p50 | p95 | max | lỗi |
|---|---:|---:|---:|---:|---:|
| `/wda/tap` | 20 | 447 ms | 706 ms | 706 ms | 0 |
| `/actions` | 20 | 439 ms | 1728 ms | 1728 ms | 0 |

Sau khi prime, **cả hai đều ổn định, không endpoint nào lỗi** — khác với giả định trong
note rằng `/actions` ổn định hơn. Vẫn giữ `/actions` làm primary cho tap/swipe vì nó
không phụ thuộc app quiescence (TikTok không bao giờ đứng yên), cùng lý do swipe đã
dùng nó; `/wda/tap` làm fallback khi `/actions` bị từ chối bằng lỗi HTTP thật, và làm
primary riêng cho ô nhập text (xem §5.1).

### Process/port theo UDID

Trước và sau vòng final: **1 `wda-proxy`, 1 `tidevice relay` (18100), 1 `tidevice xctest`,
1 stream**. Không còn relay 18101/18102 như các vòng cũ.

---

## 4. Đối chiếu tiêu chí

| Tiêu chí (note.md) | Kết quả |
|---|---|
| 10 video liên tục | ✅ 18 |
| ≥3 tap like thành công | ⚠️ 4 ở smoke10, 2 ở final (xem §5.2) |
| ≥8/9 swipe đổi video ngay lần đầu | ✅ 18/18 |
| Không hard recycle | ✅ 0 |
| Không request nào treo quá 12 s | ✅ max 2.47 s |
| Không relay/xctest trùng UDID | ✅ |
| Popup thật được đóng, không false tap | ✅ 15 popup; "Add phone" thật bắt đúng (score 0.975) |
| Không lock/unlock, không về Home, không mở Calendar | ✅ |
| Không restart TikTok nếu đang foreground | ✅ log `TikTok đã mở sẵn — reuse` |
| Kết thúc vẫn ở TikTok | ✅ |
| Bình luận | ❌ chưa gửi được (§5.1) |
| Live 30 phút | ❌ chưa chạy (§5.4) |

---

## 5. Việc còn lại — chưa xong

### 5.1 Bình luận: nội dung đúng, nhưng chưa focus được ô nhập ❌

**Nội dung bình luận đã kiểm chứng là chính xác và bám caption.** Sinh trên 3 frame
thật khác nhau qua vilao.ai:

| Video | Comment sinh ra |
|---|---|
| cây hoa giấy nở hồng | *"Hoa giấy hồng nở kín cây nhìn mê quá trời 🌸"* |
| quán nướng ban đêm, caption "Tiệm nướng Trạm Dừng Chill" | *"View đêm chill thế này, ăn nướng là đúng bài luôn 😍"* |
| chợ Đà Lạt, caption "Pov: đi uống trà Cherry trong chợ Đà Lạt" | *"Đi chợ Đà Lạt mà mê nhất mấy ly trà cherry này á 😋"* |

Cả ba đều nhận đúng chủ thể và bắt được cả từ khoá trong caption ("chill", "trà
cherry", "chợ Đà Lạt"). Pool comment dựng sẵn đầu phiên cũng chạy (30 câu).

**Chưa gửi được**: tap vào ô "Thêm bình luận…" không bật bàn phím, nên `/wda/keys`
gõ vào hư không. Đã thử và đều thất bại:

| Cách | Kết quả |
|---|---|
| `/actions` tap 60 ms | không focus |
| `/actions` tap giữ 200 ms | không focus |
| `/wda/tap` (XCUICoordinate) tại x = 60 / 120 / 150 / 200 | không focus |
| `/wda/touchAndHold` 0.2 s | không focus |
| tap hai lần liên tiếp | không focus |
| tìm `XCUIElementTypeTextField` / `TextView` ở depth 10 | element không tồn tại |
| nâng `snapshotMaxDepth` lên 20 hoặc 50 rồi tap | **runner treo ngay** |

Nghĩa là có một mâu thuẫn cứng: `snapshotMaxDepth: 1` là bắt buộc để không treo
runner, nhưng ở depth 1 thì WDA không focus được ô nhập của TikTok.

Chưa thử: pasteboard (`/wda/setPasteboard`) + long-press để hiện menu Paste (lần
thử đầu bị lệch vì drawer đã đóng trước khi long-press); hoặc dãy emoji phản ứng
nhanh trong drawer trống — một chạm là gửi được, nhưng chỉ ra được emoji.

**Không có nguy cơ đăng nhầm**: luồng chỉ bấm Gửi sau khi *thấy* nút Gửi chuyển đỏ,
nên gõ hụt chỉ tốn một lần bỏ qua. Ô nhập nằm ở y≈0.96 — trùng vị trí thanh
navigation dưới của feed — nên mọi tap ở đó đều verify drawer đang mở ngay trước
đó; một lần probe thiếu bước này đã nhảy sang tab TikTok Shop.

Vì chưa gửi được lần nào nên **cũng chưa xác minh được tài khoản có bị hạn chế
bình luận hay không**.

### 5.2 Xác nhận like chưa ổn định ⚠️

Vòng final: 2 xác nhận thành công, vài lần "tap gửi được nhưng icon không đổi". Ngưỡng
hiện tại `redness` tăng > 40 trong 2.5 s. Cần đo redness tim trước/sau trên frame thật
để chỉnh; cũng có thể do stream trễ hơn cửa sổ xác nhận.

### 5.3 Trang "Chọn chủ đề" chưa có capture thật ⚠️

Detector dùng tổ hợp 3 đặc trưng độc lập (sáng + gần trung tính + pill CTA hồng), có
test chống false-positive với video trắng, nhưng **chưa gặp trang thật** trong các vòng
test. Toạ độ nút "Bỏ qua" `(0.24, 0.93)` kế thừa từ bản cũ, **chưa xác minh**. Code đã
phòng: xác nhận sau khi tap, tối đa 3 lần rồi dừng và báo, đồng thời dump frame để
hiệu chỉnh khi gặp thật.

### 5.4 Live 30 phút ❌ — chưa chạy

Điều kiện trước khi chạy: (a) tắt 3uTools để không có XCTest runner thứ hai;
(b) xử lý xong §5.1 nếu muốn đo cả bình luận.

```bash
export PATH="$HOME/Library/Python/3.9/bin:$PATH"
cd /Users/levanlanh/Desktop/Din/Riviu_managers_phone
tidevice -u <UDID> kill notes.3u          # tắt runner của 3uTools
cargo build -p riviu-managers-phone --bin live_nurture_test --release
RIVIU_AI_API_KEY=<key> RIVIU_FRAME_DUMP=/tmp/riviu-frames/live30 \
RIVIU_WDA_TRACE=/tmp/riviu-live/live30.jsonl \
./target/release/live_nurture_test --udid <UDID> \
  --minutes 30 --videos 200 --like-prob 40 --comment-prob 25 --follow-prob 5 \
  --jsonl /tmp/riviu-live/live30-summary.jsonl
```

### 5.5 Ổn định XCTest của thiết bị ⚠️

Trong quá trình test, thiết bị vài lần rơi vào trạng thái không start được XCTest
(`timeout chờ WDA trên device:8100`), phải reboot mới hết. Nguyên nhân chính là runner
của 3uTools tranh kênh, cộng với việc kill/restart runner lặp lại. Sau khi có prime fix
thì không còn cần recycle nữa nên rủi ro giảm hẳn, nhưng **chưa được xác nhận qua một
phiên dài**.

---

## 6. Cấu hình AI

- `base_url` mặc định: `https://api.vilao.ai/v1`
- `model` mặc định: `cd/gpt-5.5` (API trả về `gpt-5.6-sol`)
- API key **không nằm trong repo**: lấy từ ô cài đặt trong app, hoặc biến môi trường
  `RIVIU_AI_API_KEY` cho harness. Key đã gửi qua chat nên được coi là đã lộ và xoay vòng lại.

---

## 7. Kiểm thử tự động

56 test pass (`cargo test --workspace`), gồm:

- Feed thật (fixture `crates/core/tests/fixtures/feed-iphone8.jpg`) không match nút X.
- Sheet dán lên feed thật → tìm đúng tâm ✕ trong sai số 8 px.
- Video trắng sáng không bị nhận là onboarding; Home screen không bị nhận là feed.
- Rail dò ra từ frame thật, đúng layout 2; toạ độ like cũ lệch >40 px so với tim thật.
- Toạ độ stream @2x map đúng sang WDA points.
- Digest frame: đổi 1 byte là phân tích lại; frame y hệt thì bỏ qua.
- Hai frame liên tiếp mới phát event; sighting lệch vị trí thì đếm lại từ đầu.
- Per-UDID lock: job thứ hai bị queue, thiết bị khác không bị chặn.
- Registry chỉ kill PID có command-line khớp fingerprint.
- Lỗi transport/timeout/session/http phân loại đúng; chỉ lỗi HTTP mới được fallback endpoint.
- Sanitize comment: bỏ `<think>`, ngoặc kép, đánh số; loại output quá dài; cắt theo số từ.
- NCC coarse-to-fine cho cùng kết quả với quét vét cạn.

Hiệu năng detector (debug build, frame 750×1334): feed có compose bar **8.3 µs/frame**
(short-circuit ở compose bar), frame không có bar phải chạy template match **160 ms/frame**.
Release nhanh hơn nhiều; watcher giới hạn 3 FPS nên dư sức.

---

## 8. Refactor (26/07 tối)

`nurture.rs` trước đây là một file 1118 dòng với `run_session()` dài 474 dòng.
Đã tách thành module:

| File | Dòng | Trách nhiệm |
|---|---:|---|
| `nurture/mod.rs` | 686 | Engine, điều phối phiên, helper frame dùng chung |
| `nurture/actions.rs` | 340 | 4 gesture của feed + đóng drawer, mỗi cái tự xác nhận |
| `nurture/recovery.rs` | 143 | `Outcome`, `Budget`, thang recovery |

Lý do tách theo trục này: ba nhóm đó thay đổi vì ba lý do khác nhau — gesture đổi
khi TikTok đổi giao diện, recovery đổi khi hành vi thiết bị đổi, điều phối đổi khi
yêu cầu nghiệp vụ đổi.

Ngoài ra: `cargo build --workspace` sạch **0 warning**; 63 test pass.

Đã thêm `AGENTS.md` ở gốc repo — hướng dẫn cho agent tiếp nhận, gồm danh sách
ràng buộc "đừng làm lại", kiến trúc, quy trình hiệu chỉnh detector. **File đó phải
được cập nhật cùng lúc với mọi thay đổi kiến trúc.**

---

## 9. Live 15 phút (26/07, 20:36) và hai lỗi nó lộ ra

Cấu hình: `--minutes 15 --videos 400 --like-prob 35 --comment-prob 20 --follow-prob 6
--watch-min 4 --watch-max 12`.

Kết quả: **47 video trong 912 s, 0 recovery, 0 request lỗi**, request chậm nhất
1051 ms, kết thúc ở TikTok. Nhịp hành vi chuyển 5 lần, đi qua đủ 3 nhịp.

Hai lỗi logic lộ ra và đã sửa:

### 9.1 Xác nhận vuốt quá gắt → vuốt đúp, bỏ sót video

115 lệnh vuốt nhưng chỉ 47 video được xác nhận. Cửa sổ chờ frame đổi là 1.4 s,
trong khi stream chỉ đẩy khi có thay đổi và chạy ~7 FPS — nên nhiều lần vuốt thật
sự ăn nhưng bị báo "không ăn", rồi vòng lặp vuốt lại lần nữa và **nhảy mất một video**.

Sửa: nới `SWIPE_SETTLE` lên 2.4 s, và thêm `wait_for_new_frame()` so digest trên
frame thô, không giải mã JPEG (so digest vốn không cần decode).

Kiểm chứng bằng vòng verify 6 phút: **39 lệnh vuốt → 39 video**, tỉ lệ 1:1.

### 9.2 Kết luận phiên sai — báo `partial` cho phiên khoẻ

`total_videos` là **trần**, không phải mục tiêu. Phiên chạy theo đồng hồ dừng khi
hết giờ với trần còn nguyên (47/400), và luật cũ `videos < trần/2 && có lỗi` kết
luận `partial` — nói với người vận hành rằng một phiên 47 video hoàn toàn khoẻ đã
hỏng.

Sửa: theo dõi **lý do dừng**. Chỉ hạ xuống `partial` khi phiên dừng vì hết video
(chứ không phải hết giờ) mà vẫn chưa đạt nửa trần, hoặc khi chạy được dưới 3 video
và có lỗi. Có test hồi quy `a_timed_run_that_did_its_work_is_done_not_partial`.

Vòng verify 6 phút sau khi sửa: **`done` — 39 video, 2 tim, 3 popup đóng, 0 recovery,
0 request lỗi**, chậm nhất 1725 ms.

### 9.3 Vì sao số tim thấp

Trong 15 phút chỉ 1 tim. Không phải lỗi: 7/8 lần thử đọc ra "đã tim từ trước", và
kiểm tra lại frame thật cho thấy **đúng là đã like** (tim đỏ, 47.6K lượt). Tài
khoản test đã like gần hết nội dung FYP của nó qua nhiều vòng test. Đã thêm fixture
`feed-heart-liked.jpg` và test hồi quy phân biệt tim đỏ (redness ≈ 124) với tim
trắng (≈ −5…+10).

---

## 10. Vòng test bình luận riêng (26/07, 21:20–21:35)

Sau khi user cấp quyền thủ công trên máy cho icon trong drawer, chạy lại vòng test
tập trung vào bình luận: `--like-prob 0 --comment-prob 100 --follow-prob 0
--steady chatty` (thêm cờ `--steady` để ghim nhịp hành vi, không bị pha loãng bởi
các đợt lướt).

Kết quả: **16 video, 0 bình luận, 11 popup đóng, 0 recovery, 0 request lỗi**.
5 lần gõ text (`keys` n=5) nhưng nút Gửi chưa bao giờ đỏ → luồng bỏ qua, không gửi.

Chụp lại từng bước bằng `probe_send.py` cho kết quả dứt khoát: sau khi native-tap
vào ô nhập, **ảnh chụp trước và sau giống hệt nhau** (bottom-mean 241.5 không đổi),
placeholder "Thêm bình luận…" vẫn nguyên → bàn phím không lên, ký tự gõ ra rơi vào
hư không.

Bổ sung vào danh sách đã loại trừ:

| Cách | Kết quả |
|---|---|
| nâng depth 30 **khi drawer đang mở** rồi tìm `XCUIElementTypeTextView` | query 10.4 s, 0 element, rồi treo runner |
| tap nút "Bình luận" giữa drawer trống (187, 499) | không focus |
| tap icon ảnh / sticker / @ cạnh ô nhập | không focus |

**Kết luận**: WDA không focus được ô nhập bình luận của TikTok trên stack này.
Ở `snapshotMaxDepth: 1` không cử chỉ nào ăn; mọi cách nâng depth đều treo runner.
Việc cấp quyền làm **tài khoản** bình luận được bằng tay, nhưng không đổi được
giới hạn của WDA.

Đường khả thi nhất còn lại: **dãy emoji phản ứng nhanh** trong drawer — một chạm
gửi được một bình luận thật, không cần bàn phím.

---

## 11. Đối chiếu TOOL TIKTOK — nguyên nhân thật của việc không bình luận được

Đọc `TOOL TIKTOK/scripts/tiktok/shared/wda_touch.py`, `wda_typing.py`,
`wda_session.py` và `modules/wda_manager.py`, rồi thử port từng cách sang stack này.

### Họ làm khác ta ở đâu

`wda_touch.py` mở đầu bằng "Quy tắc bất biến":
- TAP = `POST /wda/swipe` với delta 1 px (**không** dùng `/wda/tap`)
- LONG PRESS = `/wda/touchAndHold`, DOUBLE TAP = `/wda/doubleTap`
- **Không dùng W3C Actions API** — ghi rõ lý do: "TikTok session-steal"

`wda_session.py:390,423`: họ **attach vào session sẵn có** của WDA (đọc `sessionId`
từ `/status`), chỉ `POST /session` với capabilities **rỗng hoàn toàn**
(`{"capabilities":{"firstMatch":[{}]}}`) khi không có session nào sống.

### Vì sao không port được

Thử lần lượt trên máy thật:

| Cách của họ | Kết quả trên agent custom của dự án |
|---|---|
| sessionless `POST /wda/swipe` (tap 1 px) | **HTTP 404** — endpoint không tồn tại |
| `POST /session/{id}/wda/swipe` | **HTTP 400** — payload không khớp |
| thay bằng `/wda/dragfromtoforduration` delta 1 px | tap **chạy** (mở được drawer) nhưng **vẫn không focus ô nhập** |
| attach session từ `GET /status` | build này **không trả `sessionId`** trong `/status` |
| capabilities rỗng, không prime | **treo runner ngay** |

### Nguyên nhân

**Hai bên chạy hai bản WebDriverAgent khác nhau.** `modules/wda_manager.py:894,130`
cho thấy TOOL TIKTOK dùng **Facebook WebDriverAgent chuẩn**
(`com.facebook.WebDriverAgentRunner`, sideload qua 3uTools). Dự án này dùng agent
custom `com.riviu.managersphone.agent.xctrunner`, và trên `tidevice applist` của
máy test **chỉ có agent custom này**.

Bảng khác biệt đã đo:

| | FB WDA (TOOL TIKTOK) | Agent custom (dự án) |
|---|---|---|
| `/wda/swipe` sessionless | có | 404 |
| `sessionId` trong `/status` | có | không |
| Cần `snapshotMaxDepth: 1` để khỏi treo | không | bắt buộc |
| Focus được ô nhập bình luận | có | **không** |

### Đề xuất

Cài `com.facebook.WebDriverAgentRunner.xctrunner` lên thiết bị và trỏ driver sang
đó — sửa `AGENT_BUNDLE` trong `crates/ios-driver/src/pmd.rs` và `--bundle-id` của
`wda-proxy`. Nếu build đó hành xử như bên TOOL TIKTOK thì bình luận chạy được, và
nhiều khả năng bỏ được luôn ràng buộc `snapshotMaxDepth: 1` cùng các workaround
quanh nó.

Đây là thay đổi trên thiết bị của user (cần IPA + ký), nên chưa tự làm.

---

## 12. Vì sao TOOL TIKTOK bình luận được — nguyên nhân cuối cùng

Sau khi loại trừ hết phía code, so trực tiếp hạ tầng hai bên.

### Đối chứng chứng minh stack của dự án không hỏng

Chạy đúng luồng gõ text trong **app Cài đặt** trên cùng thiết bị, cùng agent:

| Bước | Kết quả |
|---|---|
| `/source` depth 30 | 75 KB / 9,4 s — 165 Cell, 126 StaticText, 106 Button |
| tìm `XCUIElementTypeSearchField` | n=1, rect y=123 |
| `POST /element/{id}/click` | **bàn phím iOS hiện đầy đủ** (vùng đáy 244,9 → 220,2) |
| `GET /element/active` | trả về element — có thứ đang giữ focus |
| `/wda/keys` + `element/value` | ô hiện **"helloabc"** |

WDA, agent, thiết bị, ảnh chụp, `/wda/keys` — tất cả đúng. Ảnh chụp của WDA **có**
bắt được bàn phím.

Cùng thao tác trên TikTok: `/source` chỉ ra `Other`, liệt kê element **timeout 90 s**
(Cài đặt: 9,4 s), không có element text nào để click.

### Khác biệt thật: agent

TOOL TIKTOK **không chạy WebDriverAgent thường**:

| Nguồn trong TOOL TIKTOK | Nội dung |
|---|---|
| `wda_session.py:59` | `X-RT-Token: RTmmo-…` — "WDA build RT-MMO (idbagent.ipa Jun 2026)" |
| `wda_session.py:39` | `device_port = 8906` |
| `wda_manager.py:129` | `8906, # WDA idbagent/TrollStore (confirmed — binary default)` |
| `wda_client_fixed.py:155` | build check `X-RT-Token` trên mọi endpoint |

Họ dùng **agent vá sẵn** (`idbagent.ipa`/`dairack.ipa`) cài qua **TrollStore**, chạy
port **8906**. TrollStore cài app với entitlement tuỳ ý, không bị sandbox như app ký
bằng chứng chỉ Apple Development — nên agent đó làm được việc WDA thường không làm
được, trong đó có focus ô nhập bình luận của TikTok.

Dự án này build **Appium WDA 16.0.0 gốc**, ký chứng chỉ Development, port 8100.
Máy test (`tidevice applist`) chỉ có `com.riviu.managersphone.agent.xctrunner` —
chưa có TrollStore/idbagent, nên chạy TOOL TIKTOK trên máy này sẽ fail ngay ở bước
kết nối.

### Đường đi tiếp

Cài agent vá (TrollStore + idbagent/dairack) rồi trỏ driver sang port 8906 kèm
header `X-RT-Token`. Engine hiện tại dùng lại gần như nguyên vẹn — chỉ đổi
`AGENT_BUNDLE`, port và header trong `crates/ios-driver/src/wda.rs`.

Ràng buộc: TrollStore phụ thuộc phiên bản iOS — máy này iOS 16.7.15, cần kiểm tra
khả năng cài trước khi tính tiếp.

### Chi phí AI đo được (để tham chiếu khi bật comment)

| Chỉ số | Giá trị |
|---|---|
| Vision comment | 3.724 token in / ~21 token out / 5,4 s / **$0,00486** |
| 1.000 comment | **$4,86** |
| Pool 30 câu đầu phiên | 2.087 in / 312 out / **$0,0057** (một lần) |
| Nếu resize ảnh 375×667 q70 | **$0,0033**/comment (−32%), chất lượng không đổi |

Đơn giá lấy từ mặc định app ($1,25/$10 per 1M). Số token là số đo thật.

---

## 13. Đã ship: bình luận bằng emoji do AI chọn

Sau khi xác định text không đi được qua WDA thường, chuyển sang đường **đi được**
và đã đưa vào engine.

### Cách hoạt động

Điểm mấu chốt tìm được: ô "Thêm bình luận…" **không phải** control mở composer —
tap vào nó không có tác dụng gì với touch tổng hợp. **Icon emoji ở thanh dưới
drawer** `(299, 639)` mới là thứ mở được composer thật.

1. tap icon bình luận trên rail (dò theo frame)
2. tap icon emoji → composer mở
3. `choose_emoji_reaction()` — model vision chọn 1 trong 6 cảm xúc hợp video
4. `find_emoji_grid()` dò lưới emoji trên chính frame đó rồi tap đúng ô
5. chờ nút gửi đỏ đậm (bằng chứng emoji đã vào ô nhập)
6. tap gửi, chờ nút tắt (bằng chứng đã gửi)

Lưới emoji **phải dò theo frame**: TikTok chèn mục "Đã sử dụng gần đây" sau lần
bình luận đầu, làm mọi ô dịch xuống — toạ độ cứng trúng đúng một lần rồi trượt.

### Kết quả live

| Vòng | Video | Tim | **Bình luận** | Popup đóng | Recovery | Lỗi request |
|---|---:|---:|---:|---:|---:|---:|
| emoji #1 | 10 | 1 | **3** | 5 | 0 | 0 |
| emoji #2 | 17 | 4 | **3** | 7 | 0 | 0 |

Tỉ lệ gửi ~30% số lần thử; thất bại đều an toàn (đóng composer sạch, không gửi
nhầm) vì chỉ bấm gửi sau khi *thấy* nút đỏ.

### Chi phí

Chọn emoji rẻ hơn sinh comment text nhiều: output chỉ 1 chữ số thay vì ~21 token.
Input vẫn ~3.700 token (ảnh) → khoảng **$0,0047/lần**, hoặc **$0,0032** nếu
resize ảnh xuống 375×667 trước khi gửi.

### Vẫn còn nguyên đường text

`generate_vision_comment()` và toàn bộ test của nó giữ nguyên. Ngày có agent vá
(TrollStore + idbagent, port 8906, header `X-RT-Token` — xem §12), chỉ cần đổi
chiến lược trong `do_comment` là có comment text tiếng Việt do AI viết.

---

# Phụ lục — máy thứ hai `05101fdb…` (27/07/2026)

iPhone 8, iOS 16.7.15, **TikTok 45.8.0**, không SIM. Đưa vào dùng và chạy live.

## A. Kênh Instruments chết — chẩn đoán và cách sửa

Triệu chứng: `tidevice launch` và `tidevice xctest` đều
`socket.timeout: _ssl.c:1112: The handshake operation timed out`.

Đã loại trừ đúng cách thay vì đoán — mở **từng** lockdown service riêng:

| Service | Kết quả |
|---|---|
| `com.apple.testmanagerd.lockdown.secure` | OK 67 ms |
| `com.apple.debugserver.DVTSecureSocketProxy` | OK 79 ms |
| `com.apple.instruments.remoteserver.DVTSecureSocketProxy` | `ConnectionTerminatedError` sau 10 s |

Hai cái đầu OK ⇒ pairing, TLS và DDI đều tốt (DDI cũng đã xác nhận có chữ ký
hợp lệ). Chỉ daemon Instruments phía máy bị kẹt → **reboot máy** là xong: sau
reboot service mở trong **19 ms**.

Developer Mode đang bật và chứng chỉ đã trust từ trước — hai giả thuyết dễ đổ
lỗi nhầm nhất, đều sai.

Bẫy khi chờ reboot: `tidevice info` trả lời được trong ~10 s đầu **khi máy chưa
tắt hẳn**; máy còn rụng khỏi USB thêm ~30 s nữa. Poll bằng `tidevice list`.

## B. Ba lỗi thật mà máy này lộ ra

Không lỗi nào trong số này thấy được trên máy cũ — đều đã sửa và có test.

### B.1 Hộp thoại hệ thống "iPhone chưa được Kích hoạt"

Máy không SIM tự bật hộp thoại này vài phút một lần. Nó nằm trên nền bị làm
tối, frame phân loại `Unknown`, engine vuốt mãi → **0 video suốt 10 phút**.

`ScreenKind::SystemAlert` + `find_system_alert()` khớp 3 dấu hiệu (ruột hộp
sáng ≥ 140, nền ngoài tối ≤ 70, chữ xanh ≥ 0.04) và **chỉ trả về nút trái** —
chỗ iOS đặt Bỏ qua / Cancel.

### B.2 Composer mở vào sticker pack, không phải lưới emoji

TikTok nhớ tab cuối. Sticker cũng vàng nên `find_emoji_grid()` khớp, nhưng tap
sticker không chèn gì → nút gửi không sáng. **Mất toàn bộ lượt comment.**

Sửa: tap `COMPOSER_EMOJI_TAB (0.464, 0.538)` trước khi đọc lưới; trượt ô thì thử
ô kế bên rồi ô hàng dưới; mọi nhánh thoát dùng `close_comment_ui()` (đóng **và**
xác nhận đã về feed — một lần tap không đóng nổi composer chồng lên drawer).

Trước sửa **0/9**, sau sửa **4/12 video có bình luận**.

### B.3 Thẻ LIVE trong feed — không có thanh hành động

Thẻ LIVE và frame chuyển cảnh vẫn hiện thanh compose nên vẫn là `Feed`, nhưng
không có rail. Engine rơi về layout mặc định và tap mù: **14 video liên tiếp,
0 tim**.

`rail_icons_present()` đòi ≥ 2 icon trắng cách đều 55–80 pt. Không có rail thì
chỉ vuốt tiếp. Sau sửa: **10 tim / 12 lần thử thật**, 14 thẻ được bỏ qua đúng.

Cùng lúc đổi ngưỡng xác nhận tim sang **tuyệt đối** `LIKE_FILLED_REDNESS = 90`
(đo: đầy 111–123, rỗng −26…59). Ngưỡng tương đối `before + 40` cũ hỏng cả hai
chiều trên video nền đỏ.

## C. Kết quả live trên máy mới

| Vòng | Cấu hình | Video | Tim | Bình luận | Follow | Recovery | Lỗi request |
|---|---|---:|---:|---:|---:|---:|---:|
| run3 | sau khi thêm `SystemAlert` | 51 | 5 | 1 | 1 | 0 | 0 |
| cmt | comment 100%, trước sửa tab ☺ | 9 | – | **0** | – | 0 | 0 |
| cmt2 | sau sửa tab ☺ | 12 | – | **4** | – | 0 | 0 |
| like2 | trước cổng rail | 14 | **0** | – | – | 0 | 0 |
| like3 | sau cổng rail | 20 | **10** | – | – | 0 | 0 |
| **final2** | tất cả, 12 phút | **66** | **19** | 1 | 3 | **0** | **0** |

Vòng `final2`: 725 s, tim 19/20 lần thử (**95 %**), 30 thẻ không có rail được bỏ
qua đúng, swipe p50 768 ms, tap p50 410 ms, **không có request lỗi**.

## D. Chưa giải quyết

Bình luận **text** vẫn chặn y như máy cũ — kết luận §5.1 không đổi. Đường duy
nhất có bằng chứng vẫn là WDA vá (TrollStore + idbagent, port 8906, header
`X-RT-Token`).
