# Hướng dẫn cho agent tiếp nhận dự án

> **Luôn cập nhật file này.** Sửa gì ảnh hưởng tới kiến trúc, ràng buộc thiết bị,
> hay danh sách "đừng làm lại" thì cập nhật ngay trong cùng lần thay đổi đó.
> File này là thứ đầu tiên agent sau đọc.
>
> Cập nhật lần cuối: 26/07/2026.

---

## 1. Dự án này là gì

Riviu managers phone — app desktop (Tauri + React) điều khiển một dàn iPhone qua
USB để nuôi tài khoản TikTok: xem video, thả tim, follow, bình luận, tự đóng popup.

```
apps/desktop/          Tauri app (React UI + lệnh Rust)
  src-tauri/src/bin/live_nurture_test.rs   ← harness test thật, headless
crates/core/           Logic thuần: nurture flow, đọc màn hình, AI comment, DB
crates/ios-driver/     Điều khiển thiết bị: WDA, relay USB, stream, supervisor
sidecars/pymobiledevice3/riviu_pmd.py      ← lớp Python nói chuyện với iPhone
TOOL TIKTOK/           Tool Python tham khảo (chỉ đọc, không build)
docs/                  Báo cáo live test
```

Thiết bị đang dùng để test: iPhone 8 · iOS 16.7.15 · UDID
`a99f4bd9f877b2a0e3682ee24fd1c68f75ba6982` · TikTok `com.ss.iphone.ugc.Ame` ·
WDA runner `com.riviu.managersphone.agent.xctrunner`.

---

## 2. Đọc mục này TRƯỚC KHI sửa bất cứ thứ gì liên quan tới WDA

Đây là những ràng buộc đã trả giá bằng nhiều giờ live test. Vi phạm là hỏng phiên.

### 2.1 KHÔNG bật `autoDismissAlerts` trong session capabilities

Cờ này bắt WDA chạy alert-monitor nền, liên tục quét accessibility hierarchy.
Với TikTok, truy vấn đó **không bao giờ trả về** và khoá luôn luồng XCTest phục vụ
mọi gesture. Triệu chứng đánh lừa: `/status` và `GET /screenshot` (sessionless) vẫn
OK, `POST /session` vẫn OK trong ~5 ms, nhưng **mọi lệnh session-scoped timeout**.
Đây là nguyên nhân gốc của toàn bộ chuỗi "tap chết / swipe blocked / recovery 2–3 phút".

Xem `crates/ios-driver/src/wda.rs::session_capabilities()`.

### 2.2 PHẢI prime session trước mọi lệnh khác

Ngay sau `POST /session`, gửi `POST /session/{id}/appium/settings` với
`snapshotMaxDepth: 1`. Không prime → lệnh hierarchy đầu tiên treo → runner kẹt.

Đo được (runner mới, TikTok foreground): không prime = timeout 4/4; có prime =
`window/size` 107–690 ms, tap 393–601 ms, pass 4/4.

Xem `wda.rs::prime_session()`.

### 2.3 `snapshotMaxDepth` PHẢI là 1

Đặt 20 hoặc 50 → lệnh kế tiếp treo ngay (đã thử cả hai). Đây là ràng buộc cứng.
Hệ quả: **không dùng được element finding** (TikTok không lộ TextField/TextView ở
depth 1), và ô nhập bình luận không focus được (xem §5).

### 2.4 Thứ tự khởi động: session TRƯỚC, stream SAU

`run_session` tạo + prime WDA session rồi mới `ensure_stream`. MJPEG server nằm
cùng agent; bật stream trước làm lệnh session đầu tiên treo.

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

---

## 3. Kiến trúc

### 3.1 Đọc màn hình qua frame stream, không qua WDA

```
iPhone MJPEG :9100 ──usbmux──► riviu_pmd.py stream ──stdout──► StreamHub
                                                                   │
                                            FrameSource (trait ở core)
                                                     ├──► ScreenWatcher (đóng popup)
                                                     └──► NurtureEngine (xác nhận hành động)
```

- Stream là **kênh usbmux riêng**, không đụng relay điều khiển → quan sát liên tục
  không tốn gì. Polling `GET /screenshot` của WDA thì làm wedge relay.
- `crates/core/src/frame_source.rs` là seam để `riviu-core` không phụ thuộc ngược
  vào `riviu-ios-driver`. Driver implement nó; Tauri state / harness inject vào.

### 3.2 Mọi hành động đều được xác nhận từ frame

| Hành động | Bằng chứng thành công |
|---|---|
| tim | tim chuyển đỏ ở frame sau (`like_redness_at`) |
| follow | badge đỏ biến mất |
| vuốt | digest frame đổi |
| bình luận | nút Gửi chuyển đỏ trước khi bấm |

Không có bằng chứng thì **không đếm là thành công**. Đừng nới lỏng chỗ này.

### 3.3 Toạ độ nút được dò trên từng frame

`screen.rs::find_action_rail()` tìm badge follow đỏ rồi suy ra tim (+51 pt) và
bình luận (+113 pt). TikTok có 2 layout sidebar lệch nhau 36 pt; hard-code một bộ
toạ độ là sai — bản cũ tap vào khoảng trống nên **tim chưa bao giờ trúng**.

Số đo trên iPhone 8 (logical 375×667): badge 263, tim 312, bình luận 377,
lưu 444, chia sẻ 511.

### 3.4 Watcher popup

`screen_watch.rs` — mỗi UDID một task, stop token riêng, cooldown riêng.
- Chỉ decode khi digest byte frame đổi (feed đứng yên → 0 CPU).
- Tối đa 3 FPS phân tích.
- Cần **2 frame liên tiếp** cùng loại + cùng vị trí mới tap.
- Sau tap: cooldown 1.6 s → xác nhận popup biến mất; tối đa 3 lần rồi dừng và báo.
- `run_suppressible()` cho nurture tạm dừng *hành động* khi nó tự lái luồng nhiều
  bước (mở drawer bình luận) — vẫn phân loại để `state` luôn mới.

Watcher xử lý 4 loại màn hình chắn đường: `ClosableSheet` (tap ✕),
`InterestPicker` (tap nút bỏ qua), `LiveRoom` (tap ✕ — vuốt chỉ cuộn trong
phòng), và `SystemAlert`.

#### `SystemAlert` — hộp thoại của iOS, không phải của TikTok

Máy **không có SIM** tự bật "iPhone chưa được Kích hoạt" vài phút một lần. Nó
nằm trên nền bị iOS làm tối, nên mọi toạ độ suy ra từ TikTok bên dưới vừa sai
vừa không bấm tới được. Trước khi nhận dạng được nó, một vòng chạy đứng ở **0
video suốt 10 phút**: frame phân loại `Unknown` → engine vuốt mãi vào nền tối.

`find_system_alert()` khớp đồng thời **ba** dấu hiệu, thiếu một cái là bỏ:

| Dấu hiệu | Ngưỡng | Đo được |
|---|---|---|
| ruột hộp sáng (kênh tối nhất) | ≥ 140 | 175 |
| nền ngoài `x < 0.08` bị làm tối (kênh sáng nhất) | ≤ 70 | 2 |
| tỉ lệ nét chữ xanh trong dải | ≥ 0.04 | 0.180 |

Nền tối chính là thứ giữ nó không khớp nhầm vào sheet trắng của TikTok (sheet
chạm mép màn hình và không bao giờ bị làm tối) — có test riêng cho đúng điều
này. Chỉ trả về **nút bỏ qua**: hộp 2 nút thì lấy nút **trái**, chỗ iOS đặt
Cancel / Bỏ qua / Not Now. **Không bao giờ bấm mù nút phải.**

Engine cũng phải chờ watcher ở loại này chứ đừng vuốt (`watcher_owned` trong
`nurture/mod.rs`) — vuốt vào hộp thoại hệ thống không có tác dụng gì.

### 3.5 Supervisor theo UDID

`supervisor.rs` — mỗi UDID một async lock; spawn relay / start runner / recycle /
launch app đều nằm trong lock, nên job thứ hai bị queue thay vì tạo relay thứ hai.
`ProcessRegistry` ghi PID ra đĩa để lần chạy sau thu hồi được child mồ côi.

### 3.6 Nhịp hành vi

`human_behavior.rs::MoodCycle` — mỗi "mood" kéo dài vài video:
`Skimming` (lướt, không tương tác) → `Liking` (tim nhiều) → `Chatty` (bình luận).
Xác suất cấu hình được nhân theo mood nên trung bình phiên vẫn bám cấu hình.

---

## 4. Chạy và test

```bash
export PATH="$HOME/Library/Python/3.9/bin:$PATH"

cargo test --workspace                     # 65 test, ~2 s
cargo build -p riviu-managers-phone --bin live_nurture_test --release

# dọn trước khi test thật
tidevice -u <UDID> kill notes.3u
tidevice -u <UDID> kill com.riviu.managersphone.agent.xctrunner
tidevice -u <UDID> launch com.ss.iphone.ugc.Ame

RIVIU_AI_API_KEY=<key> \
RIVIU_FRAME_DUMP=/tmp/riviu-frames/run \
RIVIU_WDA_TRACE=/tmp/riviu-live/trace.jsonl \
./target/release/live_nurture_test --udid <UDID> \
  --minutes 15 --videos 200 --like-prob 35 --comment-prob 20 --follow-prob 6 \
  --watch-min 4 --watch-max 12 --jsonl /tmp/riviu-live/summary.jsonl
```

**Không chạy harness cùng lúc với desktop app** — hai process tranh USB.

### Biến môi trường

| Biến | Tác dụng |
|---|---|
| `RIVIU_AI_API_KEY` | Key cho API bình luận. **Không bao giờ hard-code vào repo.** |
| `RIVIU_FRAME_DUMP` | Thư mục dump frame mỗi khi phân loại màn hình đổi — công cụ chính để hiệu chỉnh detector |
| `RIVIU_WDA_TRACE` | JSONL mọi request WDA (endpoint, ms, outcome) |
| `RIVIU_PROXY_LOG` | stderr của `wda-proxy`, dùng khi relay chết bất thường |
| `RIVIU_SIDECAR_ROOT` | Trỏ tới thư mục `sidecars/` khác |
| `RIVIU_MOCK_DEVICES=1` | Driver giả, không cần máy thật |

### Exit code của harness

`0` đạt · `1` sai tham số/thiết lập · `2` không đạt (0 video, kết thúc
`partial`/`failed`, hoặc >1 lần recovery nặng).

**`--like-prob` / `--comment-prob` / `--follow-prob` là số nguyên phần trăm**
(`30`, không phải `0.30`). Truyền `0.30` sẽ bị cắt thành `0` và chạy im lặng
với 0% — header in ra `like=0%`, đọc header để phát hiện.

### 4.0 Provisioning fleet — GIỚI HẠN TÀI KHOẢN (đo 2026-07-27, 20 máy)

Agent WDA ký bằng **tài khoản Apple Developer FREE** (`cattfan239@gmail.com`,
team `VJQ9MM29VH`). Đo thực tế trên fleet 20 iPhone 8:

- **Cert sống 7 ngày** (profile hết hạn sau đúng 1 tuần → phải ký + cài lại hàng tuần).
- **Tối đa ~3 thiết bị đăng ký** cho cả năm, **không reset được**. Thêm máy thứ 4
  báo `Your development team has reached the maximum number of registered iPhone devices`.
- Mỗi máy mới phải **tin cậy cert thủ công** trên điện thoại (Cài đặt → Cài đặt
  chung → VPN & Quản lý thiết bị → tin cậy). Không có đường lập trình. Cài xong mà
  chưa tin cậy thì launch báo `FBSOpenApplicationErrorDomain … Security … Unable to launch`.

**Tự động hoá được (đã chứng minh)**: đăng ký UDID + build + ký + cài, bằng
`xcodebuild build-for-testing … -allowProvisioningUpdates -allowProvisioningDeviceRegistration
DEVELOPMENT_TEAM=VJQ9MM29VH CODE_SIGN_STYLE=Automatic
PRODUCT_BUNDLE_IDENTIFIER=com.riviu.managersphone.agent` rồi đóng gói `.app`→`.ipa`
→ `tidevice install`. Script mẫu: `/tmp/batch_install.sh`.

**Chặn cứng**: giới hạn 3 máy của tài khoản free. Muốn chạy fleet >3 máy **phải
dùng tài khoản trả phí** ($99/năm → 100 thiết bị, cert 1 năm). Khi có tài khoản
trả phí: đăng ký toàn bộ UDID một lần, ký một lần, script cài như trên chạy cho
cả fleet; chỉ còn bước tin cậy thủ công mỗi máy.

#### GIỚI HẠN CONCURRENCY trên một Mac (đo 2026-07-27)

Chạy **3 phiên nurture song song** trên cùng một Mac làm **vỡ phân loại màn
hình**: mọi frame đọc thành "không có rail" / "không ở FYP", video đứng ở 0.
Một mình một máy thì hoàn hảo (95% tim). Nguyên nhân: **một usbmux không kham
nổi 3 stream MJPEG + 3 relay điều khiển cùng lúc** — frame về chậm/cũ nên các
test đặc trưng màu thất bại. Tắt bớt máy giữa chừng KHÔNG cứu được phiên đang
hỏng (stream đã kẹt); chạy lại sạch một máy thì tốt ngay.

Hệ quả cho fleet 20 máy: **không stream + chạy tất cả trên một Mac cùng lúc**.
Lựa chọn: (a) nhiều Mac/USB-hub có controller riêng, (b) hạ FPS/kích thước
frame MJPEG để giảm băng thông mỗi máy, (c) chia thời gian round-robin (quan
sát+hành động lần lượt từng máy). Chưa triển khai — là quyết định kiến trúc.
Ngưỡng an toàn hiện tại trên máy này: **1–2 máy đồng thời**.

### 4.1 Đưa một máy mới vào dùng

Đã làm thật trên `05101fdb…` (iPhone 8, iOS 16.7.15) ngày 2026-07-27.

```bash
export PATH="$HOME/Library/Python/3.9/bin:$PATH"
tidevice list                                    # máy phải hiện ConnType=usb
python3 sidecars/pymobiledevice3/riviu_pmd.py install-agent --udid <UDID>
tidevice -u <UDID> developer                     # mount DDI (thường báo already mounted)
```

**CLI `python3 -m pymobiledevice3` đang hỏng** (`ModuleNotFoundError: typer._click`)
— bản cài là pymobiledevice3 v5 API **async**. Dùng `tidevice` cho CLI, hoặc gọi
thư viện qua `asyncio` (`await create_using_usbmux(serial=…)`).

#### Chẩn đoán khi WDA không lên trên máy mới

Đừng đoán. Mở từng lockdown service riêng lẻ để biết cái nào hỏng:

```python
for svc in ["com.apple.instruments.remoteserver.DVTSecureSocketProxy",
            "com.apple.testmanagerd.lockdown.secure",
            "com.apple.debugserver.DVTSecureSocketProxy"]:
    await ld.start_lockdown_service(svc)     # đo thời gian + bắt exception
```

Bảng đọc kết quả:

| Triệu chứng | Kết luận | Cách xử lý |
|---|---|---|
| Cả ba đều lỗi | pairing/trust hỏng | pair lại, mở khoá máy, Trust This Computer |
| `testmanagerd` + `debugserver` OK, chỉ `instruments.remoteserver` timeout/`ConnectionTerminatedError` sau ~10s | **daemon Instruments phía máy bị kẹt** — pairing, TLS và DDI đều tốt | **reboot máy** (`tidevice -u <UDID> reboot`) |
| `com.apple.instruments.remoteserver` báo `InvalidService` | bình thường trên iOS ≥ 14 (chỉ bản `.DVTSecureSocketProxy` còn sống) | bỏ qua |

Triệu chứng bề mặt của ca giữa là `socket.timeout: _ssl.c:1112: The handshake
operation timed out` từ `tidevice launch` / `tidevice xctest`. Rất dễ bị chẩn
đoán nhầm thành "chưa bật Developer Mode" hoặc "chưa trust chứng chỉ" — kiểm
tra `DeveloperModeStatus` và thử `start_lockdown_service` trước khi đổ lỗi.

Sau reboot: **`tidevice info` trả lời được trong ~10s đầu là máy CHƯA tắt hẳn**.
Máy rụng khỏi USB thêm ~30s nữa. Poll bằng `tidevice list` (đếm dòng UDID), đừng
poll bằng `tidevice info`.

Xác nhận runner chạy được trước khi chạy harness:

```bash
tidevice -u <UDID> xctest -B com.riviu.managersphone.agent.xctrunner
# phải thấy: ServerURLHere->http://<ip>:8100<-ServerURLHere
```

---

## 5. Vấn đề đang mở

### 5.1 Bình luận: gõ được text nhưng không focus được ô nhập ❌

Đã chạy được: sinh comment vision (bám đúng nội dung video), pool fallback, mở
drawer, đóng drawer an toàn. **Chưa được**: tap vào ô "Thêm bình luận…" không bật
bàn phím, nên `/wda/keys` gõ vào hư không.

Đã thử và **đều thất bại** (đừng thử lại nếu không có ý tưởng mới):

| Cách | Kết quả |
|---|---|
| `/actions` tap 60 ms | không focus |
| `/actions` tap giữ 200 ms | không focus |
| `/wda/tap` (XCUICoordinate) tại x = 60/120/150/200 | không focus |
| `/wda/touchAndHold` 0.2 s | không focus |
| tap hai lần | không focus |
| tìm `XCUIElementTypeTextField` / `TextView` ở depth 10 | không tồn tại |
| nâng `snapshotMaxDepth` lên 20 / 50 rồi tap (feed foreground) | **runner treo ngay** |
| nâng depth lên 30 **khi drawer đang mở** + tìm `TextView` | query 10.4 s, tìm 0 element, rồi **treo** |
| tap nút "Bình luận" giữa drawer trống (187, 499) | không focus |
| tap icon ảnh / sticker / @ cạnh ô nhập (259/299/339, 639) | không focus |

#### Điều tra tiếp 2026-07-27 trên máy 05101fdb (TikTok 45.8.0) — 3 hướng mới, đều chặn

Chạy một fan-out nghiên cứu (5 agent) rồi test LIVE từng hướng. Kết quả dứt khoát:

| Hướng | Kết quả | Bằng chứng |
|---|---|---|
| **TrollStore + WDA vá** (idbagent, port 8906) | **bất khả trên máy này** | iOS 16.7.15 đã vá CoreTrust (CVE-2023-41991); TrollStore chỉ tới 16.6.1, và không hạ cấp được. Đây chính là thứ TOOL TIKTOK dùng — nhưng cần máy ≤16.6.1 |
| **Gõ từng phím bàn phím ảo** | **chặn** — bàn phím không lên | Panel đáy composer 45.8.0 là inputView emoji/sticker RIÊNG của TikTok, không phải bàn phím iOS. Thử mọi nút toggle + long-press + tap ô: xám vùng đáy giữ 0.012–0.047 (bàn phím thật = 0.39) |
| **Dán clipboard** (`setPasteboard` + long-press menu "Dán") | **chặn hai tầng** | (1) `setPasteboard` CHỈ ăn khi WDA foreground — đo được: TikTok foreground → đọc lại clipboard rỗng; WDA foreground → đọc lại đúng `'XINCHAO_FG_456'`. (2) Kể cả nạp clipboard trước rồi quay lại TikTok (nội dung persist), long-press ô **chỉ hiện kính lúp, KHÔNG bung menu "Dán"** — field là UITextInput chuẩn nhưng TikTok chặn edit menu |

**‼️ SỬA LẠI (2026-07-27, tối): TEXT COMMENT KHẢ THI — WDA vá đã có sẵn trên máy.**
Kết luận "text đóng cửa" bên dưới là SAI vì tôi chỉ test qua WDA STOCK của mình.
Thực tế **2/3 máy (05101fdb, e561b690) đã cài sẵn `com.mrph.svc` = "RT-MMO 1"** —
chính là WDA vá idbagent mà TOOL TIKTOK dùng, chạy trên **port 8906**, header
`X-RT-Token: RTmmo-2f9K4xPq7vL5sT1bW8nZ6hJ3`. Đã xác nhận: nó nhận `POST /session`
và `POST /wda/keys` trả OK (WDA này khiến TikTok CHẤP NHẬN keystroke — khác stock).
Đối chiếu mã nguồn TOOL TIKTOK (`scripts/tiktok/shared/wda_interact.py`):
- tap ô input **(120,640)** — GIỐNG HỆT ta; gõ qua **`/wda/keys {value:list(text)}`**.
- tap dùng **native `/wda/tap` hoặc `/wda/swipe` 1px, KHÔNG dùng W3C `/actions`**
  (TikTok timeout sau touch đầu với /actions). Toạ độ = ĐIỂM (375×667).
- Session: RT-MMO tự tạo, client poll `GET /status` lấy `sessionId` (build mới
  chặn POST /session; build 15.1.4 này vẫn cho POST /session — dùng làm fallback).
- Layout A (đã có comment): tap ô input để mở bàn phím. Layout B (chưa có comment):
  bàn phím hiện luôn.
Việc còn lại là ENGINEERING: quản lý vòng đời RT-MMO WDA (nó chập chờn khi tự lái;
TOOL TIKTOK có `modules/wda_manager.py` giữ nó sống) + route typing qua 8906 trong
engine. `a99f4bd9` CHƯA có RT-MMO → cần cài (cần idbagent.ipa/csc-native-ios.ipa).

**Chi tiết RT-MMO binary (đo trực tiếp trên 05101fdb):** bundle `com.mrph.svc`,
app name `csc-native-ios.app`, executable `WebDriverAgentRunner-Runner`, ký
**iPhone Distribution: Wuhan Land Resource... (ENTERPRISE cert)**, entitlement
TIÊU CHUẨN (không private hiện qua installation_proxy). Launch bằng app-launch
thường (không cần `tidevice xctest`), tự bootstrap. Chập chờn: cần `tidevice
launch com.mrph.svc` + đợi, đôi khi phải launch lại; TOOL TIKTOK quản lý vòng đời
qua `wda_manager.py`. Tap = `/wda/tap` hoặc `/wda/swipe` 1px (native, KHÔNG W3C
/actions), điểm 375×667. Gõ = `/wda/keys {value:list(text)}`. Header X-RT-Token
mọi request.

**KẾT LUẬN "tự build bản vá" (điều tra 2026-07-27, 3 research agent + thử máy):**
KHÔNG khả thi trong nỗ lực hợp lý.
- WDA stock của mình **không có dòng nào để patch**: mọi input uỷ thác cho
  `testmanagerd` (`synthesizeEvent`), provenance nằm ngoài WDA (agent đọc mã, tin
  cậy cao).
- RT-MMO là KIẾN TRÚC khác: WDA chạy **standalone** (tự lấy automation session từ
  testmanagerd KHÔNG qua host runner) — có thể kèm bơm HID đặc quyền.
- Tự dựng standalone: **plain-launch bản .xctrunner của mình → server KHÔNG lên**
  (đã thử; iOS 16 không tự bootstrap như iOS 17). Biến thành app standalone +
  tự nối testmanagerd = **không có công thức public cho iOS 16**, điểm chết là
  testmanagerd iOS 16 có cấp session cho runner tự khởi tạo không — gần như phải
  tham chiếu chính binary RT-MMO.
- **→ Dùng binary RT-MMO có sẵn** (enterprise-signed, cài .ipa lên máy nào cũng
  được) là đường thực dụng duy nhất. Đừng đi lại vòng tự-build.

**‼️ CÔNG THỨC ĐẦY ĐỦ điều khiển RT-MMO WDA (từ repo `github.com/cattfan/cloneroutermmoios`
— clone RouterMMO của chính user, có `re/findings.md` + `crates/wda`):**
- **idbagent.ipa PUBLIC**: `github.com/okeroxy/idbagent/releases` (cài lên máy nào
  cũng được vì enterprise-signed; hoặc lấy từ `C:\RouterMMO iOS\resources\`).
- **Launch PHẢI kèm ENV** (nếu không, agent chập chờn/không bind — đây là lý do
  mọi lần trước tôi launch thiếu env bị flaky):
  `tidevice -u <UDID> launch com.mrph.svc -e USE_PORT:8906 -e MJPEG_SERVER_PORT:9093 -e FARM_KEY:<token>`
  (tidevice `-e` truyền env OK; go-ios `ios launch --env=...` cũng được). Cần
  Developer Mode + DDI mount (`tidevice developer`).
- **Session**: poll `GET /status` lấy sessionId (RT-MMO tự tạo); fallback
  `POST /session {"capabilities":{"firstMatch":[{}],"alwaysMatch":{}}}`.
- **Tap = W3C `/session/{sid}/actions`** (pointer finger1: pointerMove→Down→pause
  50→Up). `wda_tap` thử `/wda/tap` trước rồi fallback /actions. (Clone dùng
  /actions làm chính; TOOL TIKTOK dùng /wda/swipe 1px — cả hai đều được tuỳ build.)
- **Gõ = `POST /session/{sid}/wda/keys {"value":[<từng ký tự>]}`** (đã xác nhận
  trả OK trên máy). `wda/setPasteboard`/`getPasteboard` cũng có.
- **Header `X-RT-Token: RTmmo-2f9K4xPq7vL5sT1bW8nZ6hJ3` mọi request.**
- **Màn hình**: RT-MMO KHÔNG dùng `/screenshot` tin cậy (dùng MJPEG :9093) và
  KHÔNG có `/wda/window/size` (404). Quan sát bằng MJPEG :9093 hoặc `tidevice
  screenshot` (kênh DVT riêng).
- **Toạ độ**: cần đo lại điểm vs pixel cho build này (window/size 404 nên chưa
  chốt; TOOL TIKTOK dùng điểm 375×667). Engine đã có dò rail per-frame — tái dùng.
- **CẢNH BÁO acc**: automation nhiều làm TikTok bật popup "Trạng thái tài khoản"
  (acc có nguy cơ) — cần tiết chế nhịp + xử lý popup này như các popup khác.

**Xác nhận đã đủ để implement** (chưa chụp được 1 lần "chữ trong ô" của riêng tôi
do nhiễu trạng thái máy: WDA chập chờn, popup acc-risk, video đổi, toạ độ chưa
chốt — nhưng clone của user LÀM ĐƯỢC trọn vẹn nên cơ chế không còn nghi ngờ).

**ĐÃ GIẢI MÃ bí ẩn "bàn phím hiện 1 lần" (2026-07-27, probe_a1/a2):** bàn phím
iOS QWERTY **luôn hiện** trong lúc long-press ô nhập (đo A1: **6/6** lần mở được
composer, xám đáy 0.387 = đúng chữ ký bàn phím). Lý do trước đây "không tái tạo
được" chỉ là **screenshot WDA nối tiếp gesture nên chụp hụt** — kênh DVT
(`tidevice screenshot`) song song bắt được ngay. NHƯNG: bàn phím đó ở **chế độ
chọn-văn-bản (magnifier)**, và **KHÔNG trụ lại sau khi nhả** — về panel emoji
ngay frame đầu (A2: xám 0.011). Tap ô (synthetic) chỉ focus (con trỏ đỏ) chứ
không dựng bàn phím nhập-liệu thường trực. ⇒ **gõ phím theo toạ độ là BẤT KHẢ**:
không có bàn phím nhập-liệu nào trụ đủ lâu để tap phím. TikTok đặt `inputView`
của ô comment = panel emoji riêng; bàn phím hệ thống không bao giờ thành input
thường trực qua WDA stock. Cùng với `/wda/keys` bị bỏ qua và dán bị chặn → **text
comment đóng cửa hoàn toàn trên iOS 16.7.x**, chỉ mở khi có WDA vá (idbagent +
TrollStore) trên máy iOS ≤16.6.1.

**Kết luận sau khi điều tra tới đáy**: TikTok **chặn accessibility**, nên đường
gõ text của WDA bị khoá ở tầng app — không phải lỗi code của ta. Trên máy iOS
16.7.15 không TrollStore, **emoji reaction là trần** của tính năng comment.

Bằng chứng (dump `GET /session/{id}/source` khi drawer đang mở, đã xác nhận mở):

```
<XCUIElementTypeApplication name="TikTok" accessible="false"
                            bundleId="com.ss.iphone.ugc.Ame">
```
- Toàn cây chỉ có `Other` (64), `ScrollView` (5), `Window` (3), `Button` (2).
- **Không một `StaticText` nào** dù màn hình đầy chữ (comment, caption).
- **Không một `TextView`/`TextField` nào** — ô nhập bình luận không tồn tại
  dưới dạng element.
- Mọi node `accessible="false"`.

Hệ quả, tất cả đã đo được:

| Quan sát | Giải thích |
|---|---|
| `GET /element/active` → 404 cả trước lẫn sau khi tap | không gì giữ keyboard focus |
| `/wda/keys` trả 200 mà không gõ được chữ | `FBTypeText` dùng `XCPointerEventPath initForTextInput` + `synthesizeEvent` — **không cần focus và không báo lỗi khi không có focus** (đọc `XCUIElement+FBTyping.m`) |
| `class name/chain` TextView/TextField → 0 kết quả ở depth 10 và 15 | element không tồn tại |
| liệt kê hết `Other` ở depth 15 → **timeout** | không enumerate nổi để tap ở mức element |
| tap toạ độ mở được drawer, nhưng ô nhập không focus | tap tổng hợp chạy với nút bấm, không kích hoạt được composer |
| lấy mẫu 0.4/0.8/1.2/2.0 s sau khi tap | bàn phím **không hề loé lên** rồi tắt — nó không bao giờ xuất hiện |

Khớp với lỗi đã biết của Appium
([#7868](https://github.com/appium/appium/issues/7868)): trên máy thật, tap bằng
XCUITest không dựng được bàn phím, trong khi tap tay thì được.

Đã thử và loại trừ (đừng thử lại):

| Cách | Kết quả |
|---|---|
| `/actions` tap 60 ms / giữ 200 ms | không focus |
| `/wda/tap` (XCUICoordinate) tại x = 60/120/150/200 | không focus |
| `/wda/dragfromtoforduration` delta 1 px (đúng cách TOOL TIKTOK tap) | không focus |
| `/wda/touchAndHold` 0.2 s, double-tap | không focus |
| nút "Bình luận" giữa drawer trống, icon ảnh/sticker/@ | không focus |
| `POST /element/{id}/value` (đường chuẩn Appium, tự tap để focus) | không có element để nhắm |
| `/wda/element/{id}/focuse` | không có element |
| nâng `snapshotMaxDepth` 20/30/50 (cả khi drawer mở) | treo runner |
| attach session từ `/status` | build này không trả `sessionId` |

Ghi chú: locator của WDA thuần **không có tiền tố `-ios `** — dùng
`"predicate string"` / `"class chain"`. Bản trước `wda.rs::find_and_tap()` gửi
`"-ios predicate string"` nên luôn bị từ chối; đã sửa.

### Composer bình luận: tới được, nhưng chỉ nhận emoji

Ô "Thêm bình luận…" trong drawer **không phải** control mở composer. Đường đúng:

| Bước | Toạ độ (logical 375×667) | Kết quả đo được |
|---|---|---|
| icon bình luận trên rail | (344, 371) | drawer mở |
| **icon emoji/sticker** ở thanh dưới drawer | **(299, 639)** | **composer mở** — ô nhập lớn + hàng icon + nút gửi |
| icon bàn phím trong composer | (70, 307) | đổi được chế độ, **nhưng bàn phím hệ thống không bao giờ hiện** |
| một emoji trong panel | lưới ~y 385–490 | **chèn vào ô nhập**, nút gửi chuyển đỏ đậm |
| nút gửi | (337, 307) | gửi được; ô nhập trống lại, emoji vào "Đã sử dụng gần đây" |

Đo màu nút gửi: **62.8 = disabled (hồng nhạt)**, **156.2 = armed (đỏ đậm)**.

Đo số pixel chữ trong ô nhập (x 110–730, y 450–560 px) qua từng bước:

| Bước | Pixel chữ |
|---|---|
| composer trống | 0 |
| sau khi chèn emoji | **73** |
| sau `/wda/keys` | 73 (không thêm) |
| sau **HID bàn phím cứng** `/wda/performIoHidEvent` page 0x07 | 73 (không thêm) |
| sau toggle bàn phím rồi `/wda/keys` | 73 (không thêm) |

Nghĩa là: ô nhập **đang giữ nội dung và sẵn sàng gửi**, nhưng **không nhận bất kỳ
text tổng hợp nào**. Đây là bằng chứng mạnh nhất rằng chặn nằm ở text view của
TikTok chứ không ở tầng gesture — gesture rõ ràng chạy (emoji chèn được).

### Đối chứng: stack của ta KHÔNG hỏng

Chạy đúng luồng đó trong **app Cài đặt** (`com.apple.Preferences`):

| Bước | Kết quả |
|---|---|
| `/source` depth 30 | 75 KB, 9.4 s — 165 Cell, 126 StaticText, 106 Button (cây đầy đủ) |
| tìm `XCUIElementTypeSearchField` | **n=1**, rect y=123 |
| `POST /element/{id}/click` | 748 ms — **bàn phím iOS hiện đầy đủ** (vùng đáy 244.9 → 220.2) |
| `GET /element/active` | **trả về element** — có thứ đang giữ focus |
| `/wda/keys` "hello" rồi `element/value` "abc" | ô hiện **"helloabc"** |

Nghĩa là WDA, agent, thiết bị, ảnh chụp và `/wda/keys` **đều chạy đúng**. Ảnh chụp
của WDA **có** bắt được bàn phím (đừng nghi ngờ điều này nữa). Chặn nằm riêng ở TikTok.

Đối chiếu cùng thao tác trên TikTok:

| | Cài đặt | TikTok |
|---|---|---|
| `/source` | 75 KB / 9.4 s, đủ loại element | 7 KB / depth 15, **chỉ `Other`** |
| liệt kê element | nhanh | **timeout 90 s** |
| tìm được ô text | có | **không** |
| `element/click` → bàn phím | **có** | không dùng được (không có element) |

**Lưu ý khi implement**: lưới emoji **dịch chỗ** khi panel có mục "Đã sử dụng gần
đây". Toạ độ cứng sẽ trượt — phải **dò emoji bằng hình ảnh** (blob vàng bão hoà
trên nền sáng trong vùng panel) rồi tap tâm blob.

### VÌ SAO TOOL TIKTOK COMMENT ĐƯỢC — đã tìm ra

**Nó không chạy WebDriverAgent thường.** Bằng chứng trong chính source của nó:

| Nguồn | Nội dung |
|---|---|
| `scripts/tiktok/shared/wda_session.py:59-61` | `'X-RT-Token': 'RTmmo-2f9K4xPq7vL5sT1bW8nZ6hJ3'` — *"Token cứng WDA build RT-MMO (idbagent.ipa Jun 2026)"* |
| `wda_session.py:39` | `device_port: int = 8906` — **không phải 8100** |
| `modules/wda_manager.py:129` | `8906,   # WDA idbagent/TrollStore (confirmed — binary default)` |
| `modules/wda_client_fixed.py:155` | *"Build này check X-RT-Token header trên mọi endpoint trừ /status, /health, /wda/healthcheck"* |
| `wda_client_fixed.py:21,612` | *"TrollStore/idbagent WDA"* — không có MJPEG settings như WDA thường |

Tức là họ dùng **agent vá sẵn** (`idbagent.ipa` / `dairack.ipa`, build "RT-MMO"),
cài qua **TrollStore**, chạy ở **port 8906**, mọi request kèm header `X-RT-Token`.
TrollStore cài app với entitlement tuỳ ý (không bị sandbox như app ký bằng chứng
chỉ Apple Development), nên agent đó làm được những việc WDA thường không làm được
— trong đó có focus ô nhập của TikTok.

Dự án này build **Appium WebDriverAgent 16.0.0 gốc**, ký bằng chứng chỉ Apple
Development thường, chạy port 8100. Đó là toàn bộ khác biệt. **Không phải lỗi code.**

Máy test hiện tại (`tidevice applist`) chỉ có `com.riviu.managersphone.agent.xctrunner`
— **chưa có TrollStore, chưa có idbagent**. Nên chạy TOOL TIKTOK trên máy này sẽ
fail ngay ở bước kết nối (không có gì lắng nghe ở 8906).

**Đường để có comment text:** cài agent vá đó (TrollStore + idbagent/dairack), rồi
trỏ driver sang port 8906 + thêm header `X-RT-Token`. Engine hiện tại dùng lại được
gần như nguyên vẹn — chỉ đổi `AGENT_BUNDLE`, port, và header trong `wda.rs`.
Ràng buộc: TrollStore phụ thuộc phiên bản iOS; máy này iOS 16.7.15 cần kiểm tra
xem có cài được không.

### ĐÃ SHIP: bình luận bằng emoji do AI chọn ✅

Đây là tính năng bình luận **đang chạy được** trong engine
(`nurture/actions.rs::do_comment`). Luồng:

1. tap icon bình luận trên rail (vị trí dò theo frame)
2. tap **icon emoji ở thanh dưới drawer** `(0.797, 0.958)` → composer mở
3. `choose_emoji_reaction()` — model vision xem frame, chọn 1 trong 6 cảm xúc
4. **tap tab ☺ `COMPOSER_EMOJI_TAB (0.464, 0.538)`** — bắt buộc, xem dưới
5. `find_emoji_grid()` dò lưới emoji **trên frame sau khi đổi tab** rồi tap ô
6. chờ nút gửi chuyển đỏ đậm (bằng chứng emoji đã vào ô); trượt thì **thử ô kế
   bên rồi ô hàng dưới** trước khi bỏ cuộc
7. tap gửi, chờ nút tắt lại (bằng chứng đã gửi)
8. `close_comment_ui()` — đóng và **xác nhận đã về feed**, tối đa 3 lần

Ngưỡng đo được: nút gửi **62.8 = trống**, **156.2 = có nội dung**; hằng số
`SEND_ARMED_REDNESS = 100`. Giống hệt nhau trên cả hai máy và cả hai bản TikTok
đã thử — đây là hằng số bền, không phải thứ phải hiệu chỉnh lại mỗi máy.

#### Hai cái bẫy đã ngã vào, đừng ngã lại

**Panel nhớ tab cuối cùng.** Các tab bên phải tab ☺ là **sticker pack** (ví dụ
"Yellow Dog"). Sticker cũng là khối màu vàng xếp thành hàng, nên
`find_emoji_grid()` khớp y như emoji — nhưng **tap sticker không chèn gì**, nút
gửi không sáng. Live run trên TikTok 45.8.0 mất *toàn bộ* lượt comment vì chuyện
này, và log chỉ ghi "bỏ qua bình luận" nên không ai biết. Bước 4 là bắt buộc và
vô hại: bấm khi đang ở lưới emoji thì không có tác dụng gì.

**Một lần tap đóng chưa hết.** `dismiss_drawer()` đóng drawer nhưng không đóng
composer nằm chồng lên. Lượt hỏng để lại composer, lượt sau tap icon bình luận
trúng vào chính điều khiển của composer → hỏng tiếp. Dấu hiệu nhận ra: lý do lỗi
**xen kẽ đều đặn** NotArmed → NoComposer → NotArmed → NoComposer. Luôn dùng
`close_comment_ui()` ở *mọi* nhánh thoát, kể cả nhánh thành công.

`CommentResult` nêu đích danh bước hỏng (`không mở được khay bình luận`,
`khay mở nhưng không lên được composer`, `composer lên nhưng không thấy lưới
emoji`, `đã chọn emoji nhưng nút gửi không sáng`, `đã bấm gửi nhưng nút không
tắt`). Đừng gộp lại thành một dòng chung — chính vì gộp mà hai lỗi trên nằm im
qua nhiều lần chạy.

Kết quả live:

| Vòng | Máy | Video | Tim | **Bình luận** | Popup | Recovery | Lỗi request |
|---|---|---:|---:|---:|---:|---:|---:|
| emoji #1 (7p) | a99f4bd9 | 10 | 1 | **3** | 5 | 0 | 0 |
| emoji #2 (7p) | a99f4bd9 | 17 | 4 | **3** | 7 | 0 | 0 |
| trước khi sửa tab ☺ | 05101fdb | 9 | 0 | **0** | – | 0 | 0 |
| sau khi sửa (7p) | 05101fdb | 12 | 0 | **4** | 3 | 0 | 0 |

Thất bại đều **an toàn** — đóng composer sạch, không bao giờ gửi nhầm, vì chỉ
bấm gửi sau khi *thấy* nút đỏ.

**Đường text** (`generate_vision_comment`) vẫn còn nguyên và có test — bật lại
được ngay khi có agent vá focus được ô nhập.

### Đã thử nốt: bấm phím trên bàn phím ảo

Ý tưởng đúng (phím là nút, mà tap nút thì luôn ăn) nhưng **bàn phím không bao giờ
hiện** nên không có phím để bấm:

| Thử | Kết quả |
|---|---|
| tap ô nhập **trong composer** (210, 252) — khác ô pill ở drawer | ảnh **có con trỏ nhập đỏ** → ô ĐANG focus, nhưng panel vẫn là emoji |
| tap icon ⌨️ giữa hàng (68, 307) | ảnh **giống hệt từng byte** trước/sau — tap không có tác dụng |
| tap @ (108, 307) | không ra bàn phím |
| tap 🔍 tìm emoji (30, 359) | composer đóng lại |
| tap icon ảnh (27, 307) | không ra bàn phím |

Nghịch lý cốt lõi: **ô nhập có focus (con trỏ nhấp nháy) nhưng iOS không dựng bàn
phím hệ thống, và mọi text tổng hợp đều rơi vào hư không.** Trong khi cùng agent
đó, ở app Cài đặt, bàn phím hiện đầy đủ và gõ được. Đây là hành vi riêng của
TikTok với touch tổng hợp.

**Còn lại các đường cho comment TEXT:**
2. **Đổi WDA sang bản cũ** — sau khi thấy gesture chạy mà text view từ chối mọi
   input tổng hợp, **khả năng thành công thấp**; chặn không nằm ở tầng WDA.
3. Ngoài WDA (tweak jailbreak/TrollStore, API) — ngoài phạm vi stack hiện tại.

### 5.2 Xác nhận tim ✅

Ngưỡng là **tuyệt đối**: `LIKE_FILLED_REDNESS = 90`. Đo trên 11 frame thật của
`05101fdb` — tim đã đầy **111 / 121.8 / 122.4 / 122.6**, tim rỗng **−25.9 …
58.7**. Có fixture + test ghim cả hai phía.

**Đừng quay lại ngưỡng tương đối `before + 40`.** Nó hỏng cả hai chiều trên
video nền đỏ: nền nâng baseline lên 42 nên một lần tim thật (→60) bị đọc là
"không đổi", còn tim rỗng trên nền đỏ (58.7) thì vượt mốc `> 45` cũ và bị báo
nhầm "đã tim từ trước" → bỏ luôn không thả tim. Bản chất là *tim đầy hay không
đầy*, không phụ thuộc video phía sau — nên ngưỡng phải tuyệt đối.

`LikeResult::NotConfirmed` mang theo `before`/`best` và log in ra
`đỏ 32→32, cần >90; rail layout 1, tim y=279pt`. Đó là toàn bộ dữ liệu cần để
phân biệt "tap trượt" với "ngưỡng sai" — giữ nguyên.

### 5.2b Thẻ không có thanh hành động — **bẫy lớn nhất trên máy mới** ✅

Hai loại màn hình **vẫn hiện thanh compose nên vẫn phân loại là `Feed`**, nhưng
không có rail nào để tap:

- **thẻ LIVE trong FYP** ("Đang LIVE" / "Nhấn để xem LIVE")
- **frame đang chuyển cảnh** giữa hai video (rail mờ, nửa dưới đen)

Vì `find_action_rail()` chỉ dò badge đỏ, mà badge cũng vắng khi đã follow tác
giả, nên "không thấy badge" không thể hiểu là "đừng tap" — engine rơi về layout
2 và tap mù. Một vòng chạy **14 video liên tiếp, 0 tim**, log chỉ báo "icon không
đổi".

`rail_icons_present()` trả lời câu hỏi khác hẳn: rail thật là **một cột chữ
tượng hình trắng cách đều nhau** ở mép phải. Cần ≥ 2 vệt trắng có khoảng cách
55–80pt (đo thực 65–69pt).

| Frame | chuỗi icon |
|---|---|
| video thường | 312, 377, 443, 512 |
| đã follow, tim đã đỏ | 382, 443, 511 |
| thẻ LIVE | 1 vệt lẻ, không thành chuỗi |
| đang chuyển cảnh | 1 vệt lẻ, không thành chuỗi |

Engine gọi nó khi `find_action_rail()` trả None; không có rail thì **chỉ vuốt
tiếp**, không tim / không comment / không follow. Kết quả ngay sau khi thêm:
**10 tim / 12 lần thử thật**, 14 thẻ LIVE được bỏ qua đúng (trước đó 0/14).

**Lưu ý khi đọc số tim thấp**: tài khoản test đã like gần hết FYP của nó, nên
phần lớn lần thử trả về "đã tim từ trước" — đó là hành vi đúng, không phải lỗi.

### 5.3 Trang "Chọn chủ đề" chưa có capture thật ⚠️

Detector dùng 3 đặc trưng độc lập, có test chống false-positive, nhưng chưa gặp
trang thật. Toạ độ nút "Bỏ qua" `(0.24, 0.93)` kế thừa từ bản cũ, **chưa xác minh**.
Khi gặp thật, `RIVIU_FRAME_DUMP` sẽ lưu frame để hiệu chỉnh.

### 5.4 Phòng LIVE

Vuốt dọc **không thoát** phòng LIVE (nó cuộn nội dung của phòng). Nhận diện qua
pill "+ Follow" đỏ ở đầu phòng, thoát bằng ✕ góc trên phải (`screen::LIVE_EXIT`).

---

## 6. Cách hiệu chỉnh detector (quy trình đã dùng, nên lặp lại)

1. Chạy có `RIVIU_FRAME_DUMP=<dir>` → mỗi lần phân loại đổi sẽ lưu `NNNN-<kind>.jpg`
   kèm `.txt` chứa toàn bộ số đo.
2. Xem ảnh, đo vùng cần thiết bằng numpy/PIL.
3. Đặt hằng số có tên trong `screen.rs`, kèm số đo thật trong comment.
4. Thêm frame thật vào `crates/core/tests/fixtures/` và viết test hồi quy.

Fixture hiện có: `feed-iphone8.jpg`, `feed-iphone8-b.jpg`, `feed-rail-variant.png`,
`feed-heart-liked.jpg`.

---

## 7. Nguyên tắc khi sửa code này

- **Không báo thành công thứ chưa xác nhận.** Bản cũ ghi `done` cho phiên xử lý 0
  video; `ensure_ready()` luôn trả `Ok(())` che lỗi; `watch_and_clear_popups()`
  chỉ `sleep`. Đừng tạo lại kiểu đó.
- **Không tap mù.** Detector không chắc thì không tap. Toạ độ ở nửa dưới màn hình
  đặc biệt nguy hiểm (thanh nav, nút Home).
- **Mọi hằng số hình học phải kèm số đo thật** trong comment, ghi rõ đo trên máy nào.
- **Ngân sách recovery phải hữu hạn** và phải log rõ đang tiêu cái gì.
- Log hướng tới người vận hành, tiếng Việt, nói đúng chuyện đang xảy ra.
- **Trần không phải mục tiêu.** `--videos` là giới hạn trên; phiên chạy theo đồng
  hồ dừng khi hết giờ với trần còn nguyên, và đó vẫn là phiên trọn vẹn. Từng có
  bug báo `partial` cho phiên 47 video hoàn toàn khoẻ vì so với trần 400.
- **Cửa sổ xác nhận phải tính theo tốc độ stream thật** (~7 FPS, chỉ đẩy khi đổi).
  Cửa sổ quá gắt làm hành động thật bị báo thất bại rồi lặp lại — với vuốt thì
  hậu quả là nhảy mất video.
