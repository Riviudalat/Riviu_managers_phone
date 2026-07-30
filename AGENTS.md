# Hướng dẫn cho agent tiếp nhận dự án

> **Luôn cập nhật file này.** Sửa gì ảnh hưởng tới kiến trúc, ràng buộc thiết bị,
> hay danh sách "đừng làm lại" thì cập nhật ngay trong cùng lần thay đổi đó.
> File này là thứ đầu tiên agent sau đọc.
>
> Cập nhật lần cuối: 30/07/2026.

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
WDA stock `com.riviu.managersphone.agent.xctrunner`; backend bình luận chữ dùng
RT-MMO standalone `com.mrph.svc`.

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

## 3. Kiến trúc

### 3.1 Đọc màn hình qua frame stream, không qua WDA

```
iPhone MJPEG :9100/:9093 ──usbmux──► riviu_pmd.py stream ──stdout──► StreamHub
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
`ProcessRegistry` ghi PID ra đĩa để lần chạy sau thu hồi được child mồ côi. Trên
Windows, process root còn nằm trong kill-on-close Job Object; proxy Python dùng
Job Object lồng để relay cũng chết khi chỉ proxy bị force-stop.

`DeviceControlPlane` la owner duy nhat cua UI lifecycle va stream budget. Moi
destructive await (park/preempt/start-stop stream/close context) phai do worker
so huu tu truoc await; huy task caller chi lam rot response, khong huy request dang
chay. Worker dispatch song song theo lock tung UDID, con chon capacity/victim dung
capacity gate va khoa target + victim sau khi revalidate. Khong quay lai mot worker
global await tuan tu vi stop bi ket tren may A se chan sai may B/C. Background
reserve phai atomic voi foreground owner/FIFO waiter; background start, stop va
shutdown drain deu phai giu exact stream token/proof, failure thi quarantine.

Thu tu desktop Exit bat buoc: `nurture.stop_all()` + `jobs.stop_all()`, dung/join
background sampler, `jobs.shutdown()` de danh thuc Wait va join Script task, sau do
moi `control.shutdown_cleanup()`. Khong boc WDA request bang timeout de ep JobQueue
dung; cancellation chi danh thuc Wait/acquire an toan, request WDA dang chay tu ket
thuc theo deadline cua chinh request.

### 3.6 Nhịp hành vi

`human_behavior.rs::MoodCycle` — mỗi "mood" kéo dài vài video:
`Skimming` (lướt, không tương tác) → `Liking` (tim nhiều) → `Chatty` (bình luận).
Xác suất cấu hình được nhân theo mood nên trung bình phiên vẫn bám cấu hình.

### 3.7 Roadmap điều khiển iPhone thống nhất (chốt 28/07/2026)

Kiến trúc đích là **một sản phẩm Riviu, hai engine đang hoạt động**:

- `Riviu Agent` trên iPhone: stream, gesture, text, clipboard và UI automation.
- `Riviu Device Bridge` trên desktop: usbmux/lockdown/DVT/RSD, app, media/file,
  device info, log, reboot và backup/restore.

Không dồn lockdown/backup vào IPA và không dồn lifecycle UI vào sidecar. Một
`DeviceController` với lock theo UDID, capability snapshot và typed error sẽ phối
hợp hai kênh. Product flow cuối chỉ có một `RiviuAgent.ipa`; WDA stock được giữ
tạm như rollback artifact trong giai đoạn chuyển đổi, không còn là fallback im
lặng cho job cần text.

Đời iPhone/iOS mới phải được xử lý bằng capability negotiation và transport
adapter (`LegacyUsbmuxTransport`, `RsdTransport`), không hard-code model. Agent
health phải công bố `agentVersion`, `protocolVersion` và `features`; release
manifest ánh xạ dải iOS/Xcode sang artifact đã test, có checksum và rollback N-1.

MDM/supervision là **phase sau** nhưng đã dành interface `AdminControl`: remote
erase, clear passcode, restrictions, OS update policy, ADE và Activation Lock
escrow. Phase hiện tại triển khai Agent + Device Bridge, tương ứng gần đầy đủ
quyền vận hành qua USB. Thiết kế đầy đủ:
`docs/superpowers/specs/2026-07-28-riviu-unified-iphone-control-design.md`.

### 3.8 Hướng bỏ phụ thuộc RT-MMO (chốt 29/07/2026)

Đã chọn hướng **source-equivalent reconstruction**: dùng một commit Appium WDA
được pin làm baseline, phân tích Mach-O/DWARF/Objective-C metadata và hành vi live
của RT-MMO để viết source Riviu theo các contract có test. Không patch/rebrand
binary rồi gọi đó là source của Riviu, và không reverse toàn bộ desktop EXE trước.

Artifact `RiviuAgent.ipa` hiện tại phải giữ nguyên như production oracle + rollback
cho tới khi candidate do Riviu build vượt đủ gate: standalone bootstrap, protected
auth, fresh session trước MJPEG, native gesture, clipboard/Unicode và bình luận chữ
TikTok có frame xác nhận. HTTP 200 từ `/wda/keys` không phải bằng chứng text thành
công. Candidate chưa qua gate text không được quảng bá feature `text` và desktop
không được tự chuyển sang nó.

Source đích nằm riêng dưới `sidecars/wda/riviu-agent/`. Oracle ghi WDA `15.1.4`,
còn `sidecars/wda/WebDriverAgent/` hiện là stock `16.0.0`; baseline 15.1.4 phải
được pin/extract vào cache riêng, không ghi đè cây stock. Forensic tooling/report
nằm ở `tools/rtmmo-re/` và `docs/re/rtmmo-agent/`, luôn ghi SHA-256 và redact
token/UDID. Mac là build/sign authority; binary production không được overwrite
trong lúc A/B test.

Thiết kế chi tiết:
`docs/superpowers/specs/2026-07-29-riviu-agent-source-reconstruction-design.md`.

- Gate A forensic inventory đã **PASS**; Project 2 chỉ được dùng các delta và
  bằng chứng đã version trong `docs/re/rtmmo-agent/`. Xem `gate-a.md` trước khi
  sửa standalone host hoặc WDA baseline.
- Oracle đo được 4 Mach-O ARM64: outer executable là FAT container một slice;
  ba runtime image có `cryptId=0`, còn `MH_DSYM` không có encryption load
  command (`cryptId=null`). Đừng biến command vắng mặt của dSYM thành lỗi Gate.
- WDA `15.1.4` đã được verify integrity và extract riêng dưới ignored
  `target/rtmmo-re/baselines/package`; stock WDA `16.0.0` vẫn giữ nguyên.
- Gate phải recompute inventory trực tiếp từ IPA truyền bằng `--ipa`, bắt buộc
  khớp tuyệt đối file inventory, rồi recompute delta từ đúng npm tarball đã
  verify, so source tree theo byte và ràng buộc đồng thời SHA-256
  tarball/source/inventory; không chấp nhận report chỉ tự khai version/gitHead.
  Các lệnh `baseline-diff`/`gate-a` bắt buộc giữ `--archive`; `gate-a` còn bắt
  buộc giữ `--ipa` và bộ ba `--baseline-source`/`--baseline-archive`/
  `--baseline-lock`.
- Inventory lọc ObjC class/selector khỏi control byte/type encoding, chỉ lưu
  Mach-O symbol có dynamic scope (không tính private extern), cùng DWARF function
  ranges + line table. Route contract có 8 typed entry và
  static inventory chỉ xác nhận đủ 8 **path**; method/auth/session/body vẫn là
  contract assertion cho tới khi contract test/live probe riêng xác nhận. Không
  gọi `path-confirmed` là runtime parity.
- `verify-redaction` phải quét cả raw bytes lẫn decoded JSON leaf, reject duplicate
  key; `ArchiveData` không được derive/debug-print raw IPA entry bytes.
- Runtime image đã stripped, dSYM chỉ còn ba hàm runner. Gate A không tuyên bố đã
  phục hồi feature call graph và Project 2 phải thêm contract/probe trước khi sửa
  một delta theo feature; không suy diễn call edge từ tên selector.

### 3.9 Project 2 Riviu Agent candidate (checkpoint Windows 29/07/2026)

Source candidate nam o `sidecars/wda/riviu-agent/` theo mo hinh pinned overlay:
`Scripts/prepare.py` verify npm tarball WDA 15.1.4, baseline digest
`f40eadb1e1d9872ad5a0574a5146cdbf5e0d04768ccb1f1701b289d50e4ee8f8`, roi
apply dung thu tu nam patch co SHA-256 trong `baseline-lock.json`. Source sinh ra
chi nam trong ignored `target/riviu-agent/source`; khong vendor de len Git va khong
sua `sidecars/wda/WebDriverAgent/` stock 16.0.0.
Digest sau patch phai dung
`2ca158cde4b2307957670680a6cd136b6c360d6f175303f1d012f7488e82c4cc`;
`prepare.py` khoa `git -c core.autocrlf=false` de giu LF cua upstream. Khong tai
sinh patch voi line-ending churn lam delta Objective-C thanh thay toan file.
Digest tinh moi regular file va canonical mode (`0644` hoac `0755`), gom ca
`project.pbxproj`, build config, `.plist` va executable bit cua build script;
khong duoc thu hep source attestation ve mot danh sach suffix hoac bo mode. Tren
POSIX, prepare phai dat mode that tu tar de `embed-runner-icon.sh` chay duoc.

Candidate protocol v2 dung `RIVIU_AGENT_TOKEN` (toi thieu 32 byte UTF-8), header
`X-Riviu-Token`, control `8916`, MJPEG `9094`. Chi exact `GET /status` duoc mien
auth. Protected health tra `agentVersion=0.1.0`, `protocolVersion=2`, logical
`375x667` va feature dung bon muc `stream/tap/swipe/clipboard`; Project 2 tuyet
doi chua advertise `text` hoac `pushMedia`.

Sessionless `/wda/tap` va `/wda/swipe` cua candidate dung truc tiep
`XCPointerEventPath` -> `XCSynthesizedEventRecord` ->
`FBXCTestDaemonsProxy.synthesizeEventWithRecord:timeout:error:`. Orientation lay
tu `XCUIDevice.sharedDevice.orientation` va map local, khong query active app/AX.
Synthesis co deadline 5 giay va phai kiem ca callback error lan BOOL result; body
khong phai exact dictionary hoac number khong finite tra invalid-argument. Khong
doi hai handler nay ve `/actions`, `XCUICoordinate`,
`pressForDuration:thenDragToCoordinate:` hoac `fb_waitUntilStable`; cac duong
high-level do da biet co the wedge TikTok. Route element legacy van thuoc baseline,
khong phai candidate native route.

Patch stream bind MJPEG vao loopback, doc header toi da 8192 byte trong 5 giay va
bat buoc cung `X-Riviu-Token` truoc khi nhan client. Health chi advertise `stream`
va `state=ready` khi MJPEG bind thanh cong; bind loi phai tra feature con lai voi
`state=degraded`. Build gate chay Objective-C target `UnitTests` truoc runner
`build-for-testing` va chi khi thanh cong moi ghi `objectiveCUnitTests=PASS` vao
candidate manifest.

Patch 4 khai bao truc tiep sau key attestation trong embedded
`WebDriverAgentRunner.xctest/Info.plist`: source SHA-256, xcconfig SHA-256,
protocol `<integer>2</integer>`, Objective-C test `PASS`, Xcode version va Xcode
build. Nam string lay tu explicit
xcodebuild setting voi `INFOPLIST_EXPAND_BUILD_SETTINGS=YES`; khong dung custom
`INFOPLIST_KEY_RiviuAgent*` vi target upstream dung plist san va Xcode khong tao
arbitrary key theo cach do. Build chi chap nhan runner bundle exact
`com.riviu.managersphone.agent.xctrunner`; xcconfig phai khop digest khoa
`2bed5a711927df27a86b2e2f7237bad99406b3cbbf5fccb09f8ce03fc58f53ae`;
manifest phai derive sau field tu app da codesign verify, khong tu khai lai tu
command-line. Build rehash full source va xcconfig sau Objective-C unit test, sau
runner build va sau runtime finalization; thay doi giua chung phai fail.
Voi Xcode >=26, truoc packaging phai co du `Testing.framework/Testing`,
`_Testing_Foundation.framework/_Testing_Foundation`, `lib_TestingInterop.dylib`
va `libXCTestSwiftSupport.dylib`. Hai dependency device thieu duoc copy tu active
iPhoneOS platform; sau do sign dependencies -> xctest -> outer app va chay lai
`codesign --verify --deep --strict`. Khong duoc silently package closure thieu.
Patch 5 ep runtime clipboard set/get dung exact schema cua `control-v2.json`.

Probe Gate B/C o `Scripts/probe_gate_bc.py` dung pymobiledevice3 10.1.0 + Pillow
11.3.0. Probe bat buoc nhan candidate manifest, verify manifest/source/xcconfig/
IPA SHA-256,
uninstall exact bundle, fresh-install IPA va doi chieu installed bundle/version/
build/payload/executable/signer truoc launch. DVT
`ProcessControl.launch(environment=...)` truyen token, `USE_IP=127.0.0.1`,
`USE_PORT`, `MJPEG_SERVER_PORT` va `WDA_PRODUCT_BUNDLE_IDENTIFIER`; token khong nam
trong raw manifest, decompressed IPA, prepared source, locked xcconfig, argv,
guarded log hay report. Preflight phai recompute xcconfig SHA-256 va khop manifest
truoc khi tinh bang chung `xcconfigTokenScanClean`; subprocess Rust verify evidence
phai nhan ban sao environment da xoa `RIVIU_AGENT_TOKEN`.
Control relay mo truoc de health + fresh session, con MJPEG
relay va reader chi mo sau session.

Nguong live co dinh, khong duoc ha qua CLI: 5 cold launch, 50 tap, 20 swipe va
300 giay stream. Fixture luon la `FIXTURE_ONLY`, khong bao gio PASS. MJPEG phai
auth 401/401/200, decode JPEG that, >=1 FPS, max gap <=2 giay, reconnect <=1 va
health + active-session check moi 5 giay. Moi cycle phai <=5 giay, completion gap
<=5.5 giay va schedule lateness <=0.5 giay; khong duoc catch-up count sau stall.
Gesture dung mean luma delta tren vung
Settings da dinh nghia so voi frame control khong action; khong polling
`GET /screenshot`. Unicode probe phai focus + clear Settings SearchField, go
`/wda/keys`, roi GET text read-back byte/noi dung dung; HTTP ACK khong du.
Clipboard chi duoc do sau khi foreground candidate bang `kill_existing=false`,
xac nhan PID truoc/sau khong doi va `/wda/activeAppInfo` tra dung candidate bundle
voi cung PID; ACK khi Settings con foreground khong phai bang chung clipboard.
Moi cold launch phai co witness process cu bien mat, hai port dong va DVT launch
tra PID moi on dinh. Lookup PID sau protected health, fresh session va JPEG dau
phai van dung PID launch; truoc vong ke hoac cleanup cuoi, terminate phai tra lai
dung PID da xac nhan. Nam PID fingerprint phai khac nhau. JSON va hai gate Markdown
publish theo transaction co rollback, khong de lai evidence tron neu replace loi.
Report JSON/Markdown chi publish sau `rtmmo-re verify-redaction`, va cleanup phai
dung sampler/relay, terminate candidate, xac nhan ca hai device port da dong.
Project 2 chua noi candidate vao desktop nen khong danh PASS cho soft/hard runtime
recovery: moi control/session fault lam Gate C fail; budget recovery thuoc Project
4. O phase nay chi MJPEG reader duoc reconnect co gioi han toi da mot lan.

Trang thai hien tai: source/contract/build/probe fixture tren Windows da PASS;
B0, Gate B va Gate C van `PENDING_MAC_DEVICE`. HTTP port hoac `/status` 200 khong
chung minh automation readiness. B0 can 5 cold plain-launch co protected health,
fresh automation session va JPEG dau tien theo dung thu tu. Cho toi luc gate live
dat, desktop khong chuyen candidate va production `sidecars/wda/RiviuAgent.ipa` +
`agent-manifest.json` phai giu nguyen (SHA-256 lan luot
`8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea` va
`e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a`). Xem
`docs/superpowers/specs/2026-07-29-riviu-agent-standalone-control-parity-design.md`
va `docs/re/riviu-agent/`.

### 3.10 Handoff bat buoc khi mo du an tren Mac

Agent tiep nhan tren Mac phai tiep tuc dung checkpoint Project 2 hien tai, khong
lap lai forensic/Gate A va khong ghi de production IPA. Muc tieu dau tien la build
candidate, chay B0/Gate B/Gate C tren iPhone that, roi moi danh gia text/comment.

```bash
cd <REPO_ROOT>
export PATH="$HOME/Library/Python/3.9/bin:$PATH"
export UDID=<DEVICE_UDID>
export TEAM_ID=<APPLE_DEVELOPER_TEAM_ID>

python3 -m pip install -r sidecars/wda/riviu-agent/requirements-mac.txt

# Cache nay la ignored; neu copy workspace sang Mac ma thieu thi lay dung npm tarball.
mkdir -p target/rtmmo-re/baselines
test -f target/rtmmo-re/baselines/appium-webdriveragent-15.1.4.tgz || \
  npm pack appium-webdriveragent@15.1.4 \
    --pack-destination target/rtmmo-re/baselines
test "$(shasum -a 256 target/rtmmo-re/baselines/appium-webdriveragent-15.1.4.tgz | awk '{print $1}')" = \
  "0c52fc0dcc6f837287be02a593d96d8ef28563c90b4d41f629830e84878f6bbb"

# Desktop, harness va moi XCTest runner khac phai dung truoc live gate.
tidevice -u "$UDID" kill notes.3u || true
tidevice -u "$UDID" kill com.mrph.svc || true
tidevice -u "$UDID" kill com.riviu.managersphone.agent.xctrunner || true

python3 sidecars/wda/riviu-agent/Scripts/build_candidate.py \
  --udid "$UDID" --team-id "$TEAM_ID"

RIVIU_AGENT_TOKEN="$(openssl rand -hex 32)" \
python3 sidecars/wda/riviu-agent/Scripts/probe_gate_bc.py \
  --udid "$UDID" \
  --manifest target/riviu-agent/artifacts/0.1.0/candidate-manifest.json

cargo run -q -p rtmmo-re -- verify-redaction \
  --input docs/re/riviu-agent/candidate-probes.json \
  --input docs/re/riviu-agent/gate-b.md \
  --input docs/re/riviu-agent/gate-c.md
```

Khong ha cac nguong live qua CLI. Ket qua chap nhan phai co environment
`LIVE_MAC_DEVICE`, `gateB=PASS`, `gateC=PASS`, `gateStatus=PASS`, 5 PID witness
khac nhau va cleanup sach. Neu mot gate fail, giu production artifact, sua dung
failure dau tien va chay lai toan bo probe.

Ngay ca khi B/C PASS, candidate van chua duoc goi la thay the day du RT-MMO:
feature list van chi co `stream/tap/swipe/clipboard`. Buoc ke tiep tren Mac la them
gate TikTok comment end-to-end: foreground link/video fixture, fresh session truoc
MJPEG, focus composer, Unicode read-back/armed-send frame, tap Send va frame xac
nhan comment da gui. Chi sau khi gate nay PASS moi advertise `text`, noi candidate
vao desktop o Project 4 va thay production artifact theo transaction co rollback.

### 3.11 Proxy/supervision checkpoint (29/07/2026)

iPhone test hien tai da duoc doc truc tiep qua MobileConfiguration:
`IsSupervised=false`, `ConfigurationSource=0`, `OrderedIdentifiers=[]` va khong co
configuration profile. Proxy trong desktop hien chi la CRUD/export cau hinh; chua
co bang chung proxy da duoc ap len iPhone va chua co verify public IP/rollback.

Duong chinh thong de ap proxy HTTP toan may la payload
`com.apple.proxy.http.global`. Payload nay can thiet bi duoc quan ly va Apple chi
ho tro voi Automated Device Enrollment; supervision phai duoc thiet lap trong luc
prepare/activation va thuong doi hoi xoa, prepare lai may. Tham khao:
`https://support.apple.com/en-euro/guide/deployment/dep7ba46fcd/web` va
`https://support.apple.com/en-ca/guide/apple-configurator-mac/apd9e4f64088/mac`.

Khong duoc bao `applied` tren may hien tai. Capability snapshot phai cong bo ro
`proxyApply=unsupported_unsupervised`; apply/test/rollback chi chay khi thiet bi da
supervised/enrolled. Neu ve sau chon VPN/Network Extension thi coi do la mot engine
rieng, can entitlement va gate live rieng, khong tron voi export proxy hien co.

Quyet dinh san pham ngay 29/07/2026: **giu nguyen du lieu va trang thai cac may
hien tai; khong erase/prepare lai fleet cho phase Tuong tac**. Nuoi account, mo link
TikTok, xem/tim/follow/comment/save/share, dieu phoi nhieu may va quan ly account
van di qua Riviu Agent + Device Bridge, khong phu thuoc supervision. Proxy phase
hien tai chi gom kho cau hinh, gan proxy cho device/account, kiem tra endpoint va
trang thai `manual_required`; khong tu khai da ap proxy he thong.

De phase sau cac tinh nang can supervision/MDM: Global HTTP Proxy hoac Always On
VPN, cai app im lang, kiosk/restriction, bat buoc cap nhat iOS, Lost Mode, restart/
shutdown tu xa va Activation Lock escrow/bypass. Remote wipe/clear passcode co the
la lenh MDM tren mot so enrollment khong supervised, nhung fleet hien tai cung chua
enroll MDM; khong dua chung vao phase Tuong tac.

Day la danh sach deferred day du cho roadmap hien tai: moi quan tri khi khong cam
USB (device lock, wipe, clear passcode), Wi-Fi/profile im lang, Global HTTP Proxy,
Always On VPN, silent managed-app install/remove, Single App Mode/kiosk, restriction
va Home Screen policy, OS update policy, Lost Mode, remote restart/shutdown va
Activation Lock. Khong tao menu, stub command hay dependency MDM cho cac muc nay
trong phase hien tai; chi giu `AdminControl` interface va capability typed de noi
lai ve sau. Supervision cung khong mo quyen doc password, keychain, du lieu sandbox
cua TikTok hay full filesystem; khong ghi cac quyen do vao product scope.

### 3.12 TikTok Interaction Campaign (reviewed design 29/07/2026)

Thiet ke o
`docs/superpowers/specs/2026-07-29-tiktok-interaction-campaign-design.md`. Phase nay
tao `InteractionCampaignEngine` rieng, actor `device:<udid>:default`, hai mode
`All/RoundRobin`, campaign default + override tung link, run-now/one-time va
partial-and-continue. Chi video/photo post duoc chay; profile/LIVE/music/shop/search
va short link khong resolve ra video/photo phai bi reject typed.
Schema/planner duoc de san cho nhieu account tren mot may, nhung phase nay
`interaction_list_accounts`, AllOnline, preview/start/schedule chi expose binding
`is_default=1`. Explicit non-default phai thanh `AccountSwitchUnsupported` va zero
device work; chua co account-switch capability thi khong duoc chay slot thu hai.

Copy Link khong con la action `Off/Required/Probability`: no la identity precondition
bat buoc va phai hien ro tren UI. Moi assignment phai set clipboard sentinel, mo
Share -> Copy Link, resolve lai neu clipboard la short URL, roi so ca `contentId` va
post kind. Read-back ro nhung sai/stale thi `TargetUnverified`; khong biet tap/read da
xay ra chua thi `Uncertain`; ca hai deu khong chay side effect.
Moi `TargetIdentityCopyLink` attempt giu `identity_copy_intent` append-only; assignment
giu `current_identity_attempt_no` + intent projection va phai update cung transaction.
Truoc tap Copy Link phai persist `issued`; cung attempt do khong duoc tap lai.
Crash/read-back mo ho la `Uncertain/TargetIdentityAmbiguous`, tach khoi deterministic
`Failed/TargetUnverified`. Operator Retry Failed chi append attempt Pending/None moi
sau identity Confirmed hoac terminal pre-Copy co intent van None; khong reset/reuse
row cu. Moi identity attempt co toi da hai Opening attempt trong cung run, va restart
khong tu dong resume mot Opening dang chay.
Retry transaction chi duoc reopen assignment/campaign terminal
`Partial|Failed|Interrupted`, dua actor bi anh huong ve `Eligible`, va phai giu moi
assignment/action/actor da thanh cong khong doi. `Succeeded`, `Uncertain`,
`Cancelled` va skipped khong duoc reopen.
Clipboard cu chi giu bounded trong RAM; evidence chi hash/length/type. Nhanh fail/
cancel phai restore, startup chi clear sentinel co namespace cua Riviu. Clipboard
capability phai cong bo `TargetBackgroundSafe` hoac `AgentForegroundRequired`; mode
thu hai phai foreground Agent/TikTok co PID/bundle proof va tao final fresh text
session sau lan switch cuoi. Gate 0 gom locator Share/Copy Link, clipboard va
`openUrl`; HTTP ACK hay feature `clipboard` trong manifest khong du.
Gate 0 chi qualify transport/geometry/reference Copy Link contract. No khong duoc
map `TargetIdentityCopyLink` hoac Watch thanh production Ready; exact tuple con phai
co `interaction_runtime` cua Gate G2, duoc tao tu live report cua chinh Rust
executor identity/Watch. Thieu key nay thi start/schedule fail closed truoc lease.

`DeviceWorkCoordinator` la owner duy nhat cho Nurture/Interaction/Script/Repair,
manual tap/swipe/type/home, Group Sync va Open on Device. Moi MJPEG producer, ke ca
tile nen, phai giu permit cua cung `StreamBudgetManager`; mac dinh 1, hard max 2.
Interaction lay `DeviceExclusive` truoc, stop stream nen cung UDID, inspect/repair
khong giu stream capacity, roi atomic revoke + retag mot background permit thanh
`UiWithStream`; foreground demand phai preempt sampler nen va budget=1 khong duoc tu
deadlock. Lifecycle dung `repair_install_only_locked`: chi install khi app thieu/
metadata lech, verify auth nhung khong tao session/MJPEG; auth/session/stream fail
khong reinstall. Truoc khi uninstall/install, owned stream bat buoc da duoc dung qua
`stop_owned_stream`; install-only fail closed thay vi tu clear producer ben ngoai
`StreamBudgetManager`. Primitive stop nay cung invalidate cached session cu truoc
install-only inspection; session Interaction moi chi duoc tao sau foreground. Identity
tra ve phai khop bundle/version/build cua cung lan
metadata inspection truoc khi launch + protected health duoc chap nhan. Sau do moi
foreground TikTok -> session dung profile -> MJPEG -> frame dau. Khong goi
`preflight_agent()`/`repair_agent_locked()` generic trong path nay vi hai path do tu
dung readiness session + stream va pha fresh-text sequence.
`StreamStopProof.child_stopped=true` chi duoc emit sau khi exact owned child da exit
trong bounded wait; timeout phai giu ownership va proof unconfirmed de quota khong bi
tha som. Ordinary Interaction chi dung protected relay ma install-only vua xac nhan,
khong cold-launch Agent sau khi TikTok foreground. FreshText tren stock fail closed.
AgentStatus lan luot la session-pending sau stop, stream-pending sau session va Ready
chi sau JPEG dau tien decode thanh cong.

SQLite la nguon runnable work duy nhat; in-memory channel chi wake dispatcher. Queue,
claim, state va revision phai commit truoc worker. Sau crash chi job con hoan toan
`Queued`/`WaitingCapacity` moi tu resume; campaign da vao `Preparing` phai freeze de
manual retry, intent side effect da commit thanh `Uncertain`, phan con lai
`Interrupted`. Comment phai prepare + persist exact text truoc khi type; Comment,
Repost va Direct Message phai persist `effect_intent=issued` truoc final tap.
Hai `TextNotArmed` lien tiep phai recovery trong cung lease: stop stream, tang
generation, fresh text session, MJPEG frame dau va swap dong thoi executor/watcher
session handle; `TextNotSent` la `Uncertain`, khong retry.

Khong sua production IPA/manifest de them `openUrl`. Capability driver phai bind vao
artifact/protocol/driver/transport/iOS/TikTok build/layout/detector/clipboard mode/
point geometry/orientation tuple. Inspect phai doc TikTok metadata + transport tu
Device Bridge. Manifest `375x667` khong phai runtime proof; chua qualify profile moi
thi fail closed ngoai exact 375x667 portrait, khong tap toa do iPhone 8 len may moi.
`inspect-device-capabilities` chi duoc mo lockdown/RSD provider va mot lan
InstallationProxy `get_apps` cho TikTok + Agent. Phai verify lai SHA-256 IPA truoc
sidecar I/O, lay UDID tu provider (khong echo input), hash signer identity o bien
Rust va fail closed khi app/identity thieu. Metadata inspect luon tra
`protected_auth_ready=false`, `geometry=None`; proof auth va geometry phai den tu
buoc protected runtime rieng, khong lay lai `AgentStatus` cache. Lockdown inspection
bat buoc `autopair=false`, khong tao pairing state/Trust prompt. RSD host/port la
mot cap typed, thieu mot nua thi reject va moi provider da tao phai close ca khi
connect/inventory loi.
Gate 0 trait path hien chi tu chon legacy usbmux cho fixture iOS 16; helper RSD la
primitive endpoint tuong minh, chua phai auto-selection qua `Arc<dyn DeviceDriver>`.
Khong advertise RSD end-to-end cho toi khi transport adapter theo UDID so huu va
truyen endpoint vao control plane.
Project 2 candidate chua tu dong thay RT-MMO. Direct Message/OCR, Save va Repost chi
expose sau fixture + live gate.
Moi capability G2/G4 chi duoc promote sau full regression. Promotion phai giu mot
snapshot registry goc xuyen suot, rollback neu focused check/package/staging/commit
loi, va chi seal transaction sau commit; nhieu action khong duoc overwrite snapshot.
Proxy hien dung `device_meta.proxy_id` lam nguon mutable duy nhat; thay proxy/revision
phai xoa endpoint/manual confirmation cu. Chi CRUD/assign/desktop endpoint check +
`manual_required`; khong dua MDM deferred o section 3.11 vao phase nay.

### 3.13 Interaction Gate 0 checkpoint Windows (30/07/2026)

G0.1-G0.11 da xong phan source/fixture; G0.12 van `PENDING_MAC_DEVICE`. Production
`interaction-capabilities.json` bat buoc giu `qualifications: []` cho toi khi exact
Mac/device report PASS duoc review va hash. Khong tao `interaction_start`, khong map
Watch/`TargetIdentityCopyLink` thanh Ready va khong suy capability tu feature list
trong manifest. HTTP adapter trong moi production `WdaProfile` khoi tao deny-all;
chi exact registry tuple moi duoc gan route contract.

Probe chinh la `tools/interaction-gate0/probe.py`; fixture hien co 37 test va
`vision_ocr.swift`. Live probe chi chay legacy usbmux tren fixture iPhone 8/iOS 16,
cold-witness PID Agent cu bien mat + hai device port dong, uninstall/fresh-install
dung IPA da hash va doi chieu payload/executable/signer truoc khi launch. Probe pin
exact `pymobiledevice3==10.1.0` + `Pillow==11.3.0`, foreground TikTok, protected
`POST /session`, sau do moi mo relay/reader MJPEG. Reader giu mot connection unbounded
cho mot generation, reject `Content-Length`, EOF, frame freeze, geometry drift va
gap >2 giay. Moi `/url`/tap lay sequence boundary sau khi correct-auth response xong;
chi frame strict-newer moi duoc lam evidence. Khong quay lai `_read_first_jpeg` lap
ket noi hoac dung `latest_frame_sequence` cu lam action boundary.

Geometry Gate 0 lay physical width/height/scale tu MobileGestalt voi `autopair=false`,
orientation tu protected sessionless `/wda/deviceOrientation`, roi doi chieu exact
frame `750x1334` -> logical `375x667` portrait truoc moi tap. Tuyet doi khong dung
`/window/size`, session-scoped orientation, element lookup/click, WDA screenshot hay
bat ky TikTok AX hierarchy nao trong probe RT-MMO; cac route do da biet 404/wedge.

Share duoc do tu chuoi glyph trang tren frame cung generation. Share sheet moi duoc
OCR bang macOS Vision revision 3, accurate, `en-US` + `vi-VN`, language correction,
confidence >=0.55; phai co dung mot Copy Link match. Hai tap deu la protected
sessionless `/wda/swipe` lech 1 point, va moi route exercised phai tu ghi bang chung
missing/wrong/correct auth `401/401/200` tren chinh request dung. `GET /status` chi la
session-id witness, khong phai protected readiness.
Neu fail sau khi mo Share sheet, cleanup phai tap ngoai sheet va xac nhan rail feed
tro lai truoc khi dung stream. Ke ca correct-auth tap Share timeout cung phai danh
dau sheet co the da mo; cleanup lay frame strict-newer roi moi quyet dinh dismiss,
khong duoc tin frame cache truoc tap. Final health cung lay sequence boundary sau
session/PID/geometry check va chi chap nhan frame moi con tuoi <=2 giay. Short link
chi duoc resolve HTTPS trong exact host TikTok, moi redirect deu validate va counter
stateful toi da nam hop.

Truoc live probe, clipboard iPhone fixture phai la exact plaintext
`RIVIU_GATE0_CLIPBOARD_FIXTURE_V1`; gia tri khac fail truoc write. Evidence/cleanup
chi tuyen bo restore controlled plaintext bytes, khong tuyen bo snapshot duoc moi
rich pasteboard representation. Day la fixture precondition, khong ha thanh restore
"toan bo clipboard" khi API oracle chi qualify schema plaintext.

Probe phai doi chieu executable trong installed app voi `Payload/<app>/Info.plist`
cua IPA da hash, hash signer identity, va publish du artifact/Agent/adapter/transport/
iOS/TikTok/clipboard/geometry/detector/layout/route-contract tuple. Raw token, UDID,
clipboard cu va ba target URL khong duoc vao report. Clipboard cu phai restore ca khi
case PASS; cleanup chay het reader -> relay -> generation invalidate -> exact PID
terminate -> local/device port proof, loi bat ky buoc nao lam gate FAIL.

Report pair dung journal co prior/staged SHA-256, fsync, verify ca hai destination
truoc commit va recovery ngay dau `main`; process chet giua hai replace phai rollback
byte-exact truoc device work. Production IPA/manifest van phai khop
`8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea` va
`e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a`.

Len Mac: dong desktop/harness/3uTools, cai pymobiledevice3 10.1.0 + Pillow 11.3.0,
giu may unlocked/TikTok signed-in, chay exact command trong
`docs/re/interaction-gate0/README.md`. Neu mot route, PID, frame, OCR, restore,
cleanup hay tuple field khong chac chan thi giu registry rong va ghi typed failure;
khong ha nguong hoac them bypass CLI.

Review integration 30/07 con khoa cac invariant sau. `NurtureRuntime` reserve mot
stop token/UDID va `begin_shutdown` atomically chan start moi truoc khi signal tat ca;
stop trong thoi gian stagger va stop da set truoc `run_session` phai ket thuc job voi
0 session/0 stream. Guarded clipboard production phai di qua `DeviceControlPlane`:
background mode giu generation; Agent-foreground mode chuyen ticket cho per-UDID
cleanup worker **truoc** destructive await. Caller cancel chi mat response; worker van
phai restore target, final session, replacement stream roi cleanup dung generation.
Loi truoc stop tra lai context cu; loi sau stop phai cleanup/quarantine theo progress.
Hard recycle khong duoc tang generation o giua hai stop proof; proof thu hai phai co
`old_generation` bang `new_generation` cua proof truoc.

`AppState` phai truyen exact registry tu `DriverBundle` vao `DeviceControlPlane`.
Moi lan metadata inspect phai xoa capability da negotiate ve deny-all va **khong** tu
gan route: PMD metadata co chu dich `protected_auth_ready=false`, `geometry=None`.
Chi `negotiate_interaction_capabilities` voi complete runtime snapshot trong cung
exclusive context moi duoc gan exact-match `UiCapabilities` vao profile theo UDID.
Inspect fail, repair/reinstall, session hoac stream da ton tai deu khong duoc giu/doi
capability cu. Desktop Agent Preflight/Repair chi dung install-only readiness; khong
duoc goi generic path tao session/MJPEG ngoai `StreamBudgetManager`.

`DeviceExclusiveContext` chi nhan exact UI capacity token sau khi `complete_transfer`
thanh cong. `start_interaction_session` phai doi chieu token van con live va dung UDID
trong `StreamBudgetManager`; marker lich su hoac reservation da drop khong duoc phep
tao session. Token duoc mang sang `UiSessionContext` va phai khop capacity khi start
stream. Sau do control moi goi `confirm_interaction_stream_stopped`: primitive nay chi
giu slot lock, reject neu con owned MJPEG producer/cached session va ghi current
generation lam lifecycle handoff; no khong duoc stop/start process hay tang generation.
Nho vay ca truong hop khong co background victim van co stop witness truoc session ma
khong tao cancellation window bang mot destructive stop thu hai.

`prepare_device` va desktop fallback phai dung ngay neu install-only repair loi; khong
duoc roi sang generic prepare. `JobQueue` bat buoc theo thu tu install-only ->
session-only -> steps, khong tao MJPEG. Moi loi sau khi session duoc tao, gom ca lay
session va tao artifact directory, van phai dong `UiSessionContext` dong bo de
invalidate cached session truoc khi tra loi; drop context don thuan khong du.

Registry parser, schema va HTTP adapter dung chung `is_valid_protected_route_path`,
khong cho `//`, `..`, query, fragment hay template brace lot qua parse roi moi fail
luc gui. Root `liveReportSha256` phai la lowercase SHA-256 doc lap; clipboard co route
thi bat buoc co contract ID canonical, khong duoc sinh `:set`/`:get` tu ID rong.
Fixture HTTP Gate 0 tren Windows phai doc het POST body truoc khi tra 401; dong socket
voi request bytes chua doc co the bien response auth dung thanh `WinError 10053`.

### 3.14 Flow V2 visual automation design (30/07/2026)

Flow keo-tha se khong gan canvas truc tiep vao `JobQueue` v1. Authoring dung graph
schema v2 voi node/edge/layout, Rust compile thanh immutable execution plan, va
runtime luu progress rieng theo `flow_run -> device_run -> node_attempt`. Release dau
chi cho mot duong tuyen tinh. Import JSON v1 chi bao dam supported subset; wait >60s,
XPath/predicate, target xung dot, Terminate truoc gate F1 va action thieu evidence phai
tra typed diagnostic, khong tu doi semantics. Sau gate F1, Terminate dung config
bundle typed + bang chung `ProcessAbsent`. Graph storage duoc giu ngay tu dau cho
`If`/`Repeat`, TikTok action va dependency nhieu may ve sau.

Do `riviu-script-engine` dang phu thuoc `riviu-core`, Flow model va compiled IR phai
nam trong `riviu-core::flow::model`; parser/compiler nam trong script-engine va
runtime core chi doc compiled type cua core. Khong tao dependency nguoc tu core sang
script-engine. Lua chon `One`/`Selected`/`AllEligible` la typed input cua `flow_run`,
khong nam trong revision; exact UDID duoc resolve + snapshot luc start run va khong
dong vao plan hash.

React chi so huu draft/layout. Validation, action catalog, capability/resource
contract, canonical hash, persistence, retry va execution deu thuoc Rust. Canvas dung
`@xyflow/react`; node khong duoc expose raw HTTP/WDA/shell. Moi device segment chi
giu mot `DeviceControlPlane` context va khong giu lease may A trong khi cho may B.
Authoring config la JSON theo schema, nhung compiler bat buoc doi sang
`CompiledActionConfig` typed; runtime khong parse lai authoring `serde_json::Value`.
Canonical compiled JSON van chua revision; execution hash bo duy nhat top-level
revision, nen save chi doi layout/revision giu nguyen hash con config/action version/
context/capability/flow ID van lam hash doi.
Import `@xyflow/react/dist/style.css` trong `main.tsx` truoc `index.css` de CSS Riviu
la lop override cuoi; khong chen CSS import sau cac rule trong `index.css`.
Release 1 giu mot ownership chain/device, chi upgrade
`Exclusive -> UiSession -> UiWithStream`, khong close/reacquire giua cac node phu
thuoc. Tap/swipe/type va moi side effect chi `Succeeded` sau typed postcondition;
ACK transport khong phai evidence. Node can frame phai reserve exact stream capacity,
inject `FrameSource` va van giu session-truoc-MJPEG.
`clear_and_advance` phai phat event generation moi; verifier generation cu fail ngay
`StaleGeneration`, co deadline + cancellation, khong doi frame moi vo han.
Flow Screenshot phai publish frame tu exact owned stream generation + hash artifact,
khong goi WDA `GET /screenshot`.

Attempt state phai commit intent truoc effect; nonterminal effect khi restart phai
reconcile thanh proved success/proved non-delivery/`Uncertain`. Khong retry
`Uncertain` tap/swipe/type. Artifact dung path UUID nam trong managed root, publish
temp -> validate/hash -> atomic rename -> DB transaction va reconcile orphan khi mo
app. Flow shutdown phai stop + join het worker truoc
`DeviceControlPlane::shutdown_cleanup()`.

DB da co transaction migration runner trong `crates/core/src/db/migrations.rs`.
`schema_migrations` version 1 ghi nhan exact pre-Flow schema; version 2 tao bay bang
Flow va cac index. DB legacy khong co ledger chi duoc bootstrap khi rong hoac khop
chinh xac table/column/PK/unique fingerprint; partial/unknown schema phai tra
`UnknownLegacySchema` truoc khi tao ledger. Tung migration co transaction rieng va
ledger row chi commit cung schema. Khong quay lai batch migration
`CREATE TABLE IF NOT EXISTS` cho version moi. Moi connection tao qua `Database::conn`
phai bat foreign keys va busy timeout; migrate connection con bat WAL. Upgrade khong
seed lai hay ghi lai row cua exact legacy DB.

Flow authoring persistence nam trong `crates/core/src/db/flows.rs`. Moi save tao mot
row `flow_revisions` bat bien trong `IMMEDIATE` transaction, so
`expected_revision` voi projection hien tai truoc khi validate payload, va chi chap
nhan document + compiled plan cung `flow_id`/next revision. Repository recompute
execution SHA-256, luu exact canonical compiled JSON va khong tu tang/sua object tu
compiler. `expected_revision=None` chi tao ID moi; stale writer tra typed
`RevisionConflict`. Read-back phai kiem lai schema/identity/canonical JSON/hash;
khong hydrate record DB bi hong. Layout-only revision co authoring JSON moi nhung giu
nguyen execution hash.

TikTok campaign khong duoc ha thanh chuoi tap generic.
`InteractionCampaignEngine` hien chua co implementation; node TikTok release sau chi
duoc them sau khi engine/public facade + G0-G3 da implement va qualify. Flow
A-comment/B-reply chi mo khi output A la artifact co comment identity da qualify;
text/handle don thuan khong phai identity proof.

`sidecars/pymobiledevice3/riviu_pmd.py` hien co hai false product surfaces: `terminate`
chi tra best-effort success ma chua terminate, `syslog` chi tra sample text. Terminate
phai duoc implement + verify truoc khi vao action catalog; syslog de ngoai Flow release
1 cho toi khi os_trace path that co contract/live test.

Thiet ke day du:
`docs/superpowers/specs/2026-07-30-riviu-flow-v2-design.md`. User da duyet thiet ke;
implementation duoc chia thanh bon gate bat buoc F0 Foundation -> F1 Runtime -> F2
Desktop -> F3 Acceptance tai:

- `docs/superpowers/plans/2026-07-30-riviu-flow-v2-roadmap.md`;
- `docs/superpowers/plans/2026-07-30-riviu-flow-v2-foundation.md`;
- `docs/superpowers/plans/2026-07-30-riviu-flow-v2-runtime.md`;
- `docs/superpowers/plans/2026-07-30-riviu-flow-v2-desktop.md`;
- `docs/superpowers/plans/2026-07-30-riviu-flow-v2-acceptance.md`.

Commit chua checkpoint ke hoach nay la rollback baseline truoc F0. Ngay truoc source
edit dau tien, worktree phai sach, dat `RIVIU_PRE_F0_COMMIT=$(git rev-parse HEAD)`
va ghi full hash vao checkpoint F0; cac handoff F1-F3 giu nguyen hash do, khong
day baseline tien theo implementation commit.

Checkpoint pre-F0 da khoa ngay 30/07/2026 tai
`805056790d890046384ad7a578cc34a99088e799`; baseline Rust workspace va desktop
Vitest deu PASS tren worktree sach truoc source edit. Moi handoff F0-F3 phai lap lai
dung full hash nay lam rollback commit.

Khong bat dau gate sau khi gate truoc chua commit va qua day du lenh verify. Moi Flow
can UI session bat buoc co Launch App la executable node dau tien de compiler co
target bundle truoc fresh/ordinary session va MJPEG. Launch dau tien la mot durable
attempt: intent/effect -> foreground dung mot lan -> session -> stream neu can ->
verify; no khong duoc dispatch lai trong loop. Pure bridge Wait/Terminate khong can
Launch; Terminate tu mang exact bundle ID.
Coordinate picker chi giu frame hien co trong bo nho, acquire `ManualControl` de lay
exact capability/geometry cho bundle cua Launch App dau tien, roi release; no khong tao stream/session va khong goi WDA
screenshot. Point luu full `QualifiedGeometry` profile digest, kich thuoc anh va
orientation; runtime mismatch phai fail truoc dispatch. Terminate App nam ngoai
catalog o F0, roi duoc bat trong F1 sau khi bounded DVT terminate, read-only process
query, exact bundle proof, Python contract test va Rust ownership/integration test deu
PASS. Legacy va Flow deu phai di qua cung `DeviceControlPlane` lock theo UDID; recovery
chi `ReadProcess`, khong kill lai de doan retry. F0.1 model/catalog, F0.2 compiler,
F0.3 legacy importer va F0.4 migration runner da commit; F0.5 immutable revisions da
co optimistic/concurrency/integrity tests. Buoc tiep theo la F0.6 full foundation
gate; runtime F1 chua duoc bat.

Projection attempt co `retryAllowed` do backend tinh tu durable state va proof
reconciler. Frontend chi an/hien nut theo field nay; khong tu suy retry tu action
name. `Uncertain` Tap/Swipe/Type Text luon false. `retry_safe` mac dinh 0 trong DB
va chi transaction reconciler cua idempotent-set moi duoc set 1 kem event proof.

Trong mot release compatibility, page Flow giu tab `Legacy` mount nguyen
`ScriptsPanel` + `ScheduleBlock`, va Jobs JSON runner van truy cap duoc. Rollback
chi tat Flow UI/commands; khong downgrade DB va khong xoa/ghi lai legacy row.

Text read-back Flow chi cho locator union `accessibilityId|className`; `className`
can cho Settings SearchField da co trong Gate B/C. XPath, predicate va class chain
van bi cam. Profile stock khong advertise read-back va tuyet doi khong nang
`snapshotMaxDepth` de co gang tim element.

---

## 4. Chạy và test

```bash
export PATH="$HOME/Library/Python/3.9/bin:$PATH"

cargo test --workspace                     # toàn bộ Rust workspace
cargo build -p riviu-managers-phone --bin live_nurture_test --release

# dọn trước khi test thật
tidevice -u <UDID> kill notes.3u
tidevice -u <UDID> kill com.riviu.managersphone.agent.xctrunner
tidevice -u <UDID> launch com.ss.iphone.ugc.Ame

RIVIU_AI_API_KEY=<key> \
RIVIU_WDA_BACKEND=rt-mmo \
RIVIU_RTMMO_TOKEN=<token> \
RIVIU_RTMMO_IPA=<path-to-current-RiviuAgent.ipa> \
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
| `RIVIU_WDA_BACKEND` | Chỉ đọc ở harness. Desktop bỏ qua biến này và luôn dùng Unified Agent; `stock` chỉ là rollback/debug tường minh. |
| `RIVIU_RTMMO_TOKEN` | Desktop chỉ nhập một lần vào OS credential store; harness đọc ở biên binary. **Không hard-code hoặc đọc biến này trong driver library.** |
| `RIVIU_RTMMO_IPA` | Chỉ là override cho harness. Desktop dùng `sidecars/wda/agent-manifest.json` + `RiviuAgent.ipa` và bắt buộc khớp SHA-256. |
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

## 5. Trạng thái bình luận

### 5.1 Bình luận chữ qua RT-MMO: ĐÃ LIVE XÁC NHẬN

Ngày 28/07/2026 đã nối xong đường text end-to-end trong code:

- `wda.rs`: profile RT-MMO, header auth cho mọi request, logical size 375×667,
  không probe endpoint thiếu; text job dùng `POST /session` mới sau khi TikTok đã
  foreground. Nếu build không cho POST mới fallback session tự tạo từ `/status`.
- `pmd.rs` + `riviu_pmd.py`: chọn đúng một backend; launch app với đủ ba env;
  relay `8906`, stream `9093`; retry launch có giới hạn; token qua environment;
  kiểm tra/cài IPA từ `RIVIU_RTMMO_IPA` khi bundle thiếu. Chế độ
  `--bootstrap-only` restart agent text mà không sinh relay thứ hai.
- `nurture/actions.rs`: sinh vision comment trước khi mở UI, pool là fallback;
  tap input `(120,640)` qua native tap, `/wda/keys` nhận một phần tử chứa cả câu;
  chỉ tap Gửi
  khi drawer ban đầu đúng `Open` và một **frame mới** chuyển `Open -> SendArmed`;
  chỉ tăng counter/ghi DB khi frame trở lại `CommentDrawer::Open` (nút đã tắt).
  Nếu drawer đã `SendArmed` trước lúc gõ thì đó là draft cũ: đóng và bỏ lượt,
  tuyệt đối không append/gửi rồi gán nhầm nội dung mới.
- `UiSession::supports_text_input()`: RT-MMO đi đường chữ; stock giữ nguyên emoji
  fallback. Một lượt RT đã gõ nhưng không armed sẽ đóng UI và bỏ lượt, không trộn
  thêm emoji vào draft có thể còn sót.
- Recovery cứng chỉ dành cho `UiErrorKind::Transport`; session/timeout chỉ thử
  session mềm, không recycle agent vì health probe. Với job comment, cả soft/hard
  recovery đều dựng fresh text session, **mở lại MJPEG thành công**, rồi mới thay
  đồng thời session của feed + watcher. Fresh RT session đã dừng stream cũ; bỏ
  bước `ensure_stream` ở recovery sẽ làm engine chạy tiếp mà không có frame producer.

Đã bundle/cài/trust release `idbagent.ipa` hợp lệ (SHA-256
`8A24847099495FF70B998522692C43F00DD16B90F698BDA6953A73F5D33002EA`). Hai IPA
trong `C:\RouterMMO iOS\resources\` có cert hết hiệu lực (`0xe8008018`), đừng thay
bản bundle bằng chúng.

Live proof 28/07/2026 trên máy test:

- probe ASCII: comment `Hay qua ban oi`, count `2 -> 3`, ảnh
  `%LOCALAPPDATA%\Temp\riviu-live\manual-comment-probe-20260728-1601\12-sent.png`;
- probe Unicode: comment `Hay quá 🔥`, count `6 -> 7`, ảnh
  `%LOCALAPPDATA%\Temp\riviu-live\ordered-unicode-probe\03-sent.png`;
- harness chạy đúng product path tự bootstrap: `comments=1`, log
  `đã gửi bình luận chữ (xác nhận nút gửi tắt)`, không request lỗi, artifact
  `%LOCALAPPDATA%\Temp\riviu-live\harness-comment-product-fix-20260728-172255`.
- sau hardening DVT env/strict MJPEG/recovery: harness 2 video gửi `comments=1`,
  fresh session 8 ms, `keys` 623 ms, 0 request lỗi, 0 recovery, artifact
  `%LOCALAPPDATA%\Temp\riviu-live\final-comment-20260728-182428`.
- bản release cuối sau generation/retry/stock-order hardening, chạy
  `--steady chatty`: 3 video gửi **2 bình luận chữ**, cả hai xác nhận nút Gửi tắt,
  `keys` 418–536 ms, 0 recovery, artifact
  `%LOCALAPPDATA%\Temp\riviu-live\final-chatty-20260728-185724`.

Comment thứ hai của vòng harness bị video đổi sang LIVE đúng lúc tap rồi bật modal
chính sách; frame dump xác nhận đây là chuyển màn hình, không phải text session
regression. Không nới invariant frame để đếm lượt đó.

#### Lịch sử điều tra stock WDA (giữ lại để không thử lại)

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
`X-RT-Token: <token>`. Đã xác nhận: nó nhận `POST /session`
và `POST /wda/keys` trả OK (WDA này khiến TikTok CHẤP NHẬN keystroke — khác stock).
Đối chiếu mã nguồn TOOL TIKTOK (`scripts/tiktok/shared/wda_interact.py`):
- tap ô input **(120,640)** — GIỐNG HỆT ta; gõ qua **`/wda/keys {value:list(text)}`**.
- tap dùng **native `/wda/tap` hoặc `/wda/swipe` 1px, KHÔNG dùng W3C `/actions`**
  (TikTok timeout sau touch đầu với /actions). Toạ độ = ĐIỂM (375×667).
- Session: RT-MMO tự tạo, client poll `GET /status` lấy `sessionId` (build mới
  chặn POST /session; build 15.1.4 này vẫn cho POST /session — dùng làm fallback).
- Layout A (đã có comment): tap ô input để mở bàn phím. Layout B (chưa có comment):
  bàn phím hiện luôn.
Phần engineering này đã hoàn tất ngày 28/07/2026 trong `wda.rs`, `pmd.rs`,
`riviu_pmd.py` và `nurture/actions.rs`; lịch sử dưới đây chỉ là bằng chứng dẫn tới
thiết kế hiện tại.

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
- **idbagent.ipa PUBLIC**: `github.com/okeroxy/idbagent/releases`. Luôn kiểm tra
  release mới: cert enterprise của bản cũ có thể hết hiệu lực. Nếu signing team
  đổi, uninstall đúng bundle cũ rồi cài sạch; sau đó trust profile thủ công.
- **Launch PHẢI kèm ENV** `USE_PORT=8906`, `MJPEG_SERVER_PORT=9093`,
  `FARM_KEY=<token>` (thiếu env làm agent chập chờn/không bind). Production sidecar
  truyền dict env trực tiếp qua pymobiledevice3 DVT `ProcessControl.launch`; không
  đưa token vào argv của CLI con. Cần Developer Mode + DDI mount (`tidevice developer`).
- Sau kill/restart, port control cũ phải đóng trong cửa sổ bounded; không đóng thì
  fail bootstrap, không launch đè rồi nhận nhầm readiness của process cũ.
- **Session**: liveness/reuse thường poll `GET /status`. Riêng text job phải
  foreground TikTok rồi `POST /session {"capabilities":{"firstMatch":[{}]}}`;
  chỉ fallback status khi build trả đúng HTTP 404/405/501 chặn POST. Lỗi 401/500,
  response thiếu session id, transport hay timeout phải fail phiên mới, không
  attach một status session có thể đã stale.
- **Tap/swipe = sessionless `POST /wda/swipe`** với `delay/fromX/fromY/toX/toY`;
  tap dùng delta 1 px. Không fallback W3C `/actions` cho build RT-MMO hiện tại.
- **Gõ = `POST /session/{sid}/wda/keys {"value":["<cả câu>"]}`**. List từng ký
  tự có thể ACK mà không chèn; payload cả câu đã xác nhận bằng frame trên máy.
  `wda/setPasteboard`/`getPasteboard` cũng có.
- **Header `X-RT-Token: <token>` mọi request.**
- **Màn hình**: RT-MMO KHÔNG dùng `/screenshot` tin cậy (dùng MJPEG :9093) và
  KHÔNG có `/wda/window/size` (404). Quan sát bằng MJPEG :9093 hoặc `tidevice
  screenshot` (kênh DVT riêng).
- **Toạ độ**: logical 375×667; input đã live-chốt `(120,640)`. Engine dò rail
  per-frame cho icon bình luận; không hard-code layout rail.
- **CẢNH BÁO acc**: automation nhiều làm TikTok bật popup "Trạng thái tài khoản"
  (acc có nguy cơ) — cần tiết chế nhịp + xử lý popup này như các popup khác.

**Đã xác nhận end-to-end** bằng ảnh chữ trong ô, nút Gửi armed, comment hiện trong
list và harness production tăng counter; xem live proof ở §5.1.

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
comment đóng cửa hoàn toàn **qua stock WDA** trên iOS 16.7.x. RT-MMO standalone
enterprise-signed là đường đang chạy; không cần TrollStore trên máy test này.

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
| `scripts/tiktok/shared/wda_session.py:59-61` | `X-RT-Token: <token>` — token của build RT-MMO, giữ ngoài repo. |
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

Đoạn kết luận cũ "máy chưa có idbagent/TrollStore" đã hết hiệu lực: máy hiện có
RT-MMO enterprise-signed `com.mrph.svc`, chạy standalone trên iOS 16.7.15 và đã
gửi comment chữ như live proof ở đầu mục. Không quay lại hướng TrollStore hay tự
build WDA vá.

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

---

## 8. Unified Agent Runtime (28/07/2026)

> Mục này thay thế mọi kết luận cũ trong §5 nói rằng text comment phải hạ iOS,
> dùng TrollStore hoặc chỉ còn emoji. Các kết luận đó chỉ đúng với stock WDA;
> runtime sản phẩm hiện dùng agent `com.mrph.svc` đã kiểm chứng text comment thật.

- Desktop resolve `DriverConfig` một lần tại composition root. `crates/ios-driver`
  không được đọc `RIVIU_WDA_BACKEND`, `RIVIU_RTMMO_TOKEN` hoặc
  `RIVIU_RTMMO_IPA`; stock không phải fallback của desktop.
- Artifact chính là `sidecars/wda/RiviuAgent.ipa`, mô tả bởi
  `agent-manifest.json`. Luôn kiểm tra SHA-256 trước mọi lần cài. Stock
  `Riviumanagersphone.ipa` chỉ còn là rollback/debug artifact.
- Artifact RT-MMO đã chốt là release `777wealth.app` cập nhật ngày `2026-07-24`,
  SHA-256 `8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea`, profile
  enterprise Beijing `chuvendor` hết hạn `2027-07-24`. Ngày 28/07 đã live-pass
  install, launch, protected `/wda/locked` và MJPEG trên iPhone 8 iOS 16.7.15.
- Không dùng bản Wuhan `csc-native-ios.app`, SHA-256
  `628b4b3b36dbe2fa1e4c753d1d7b004443d00c829bf8581a28101ab499b7cb5a`: identity
  đã bị thu hồi và install trả `0xe8008018`, dù profile ghi hạn `2026-08-07`.
- Build hiện dùng token RT-MMO cố định: `FARM_KEY` tuỳ ý vẫn bị protected endpoint
  từ chối. Lần chạy desktop đầu phải nhận `RIVIU_RTMMO_TOKEN` đúng một lần rồi
  migrate vào OS credential account `agent-auth-token`; không sinh token ngẫu nhiên,
  không ghi token vào manifest, SQLite, frontend hoặc log. Một env token tường minh,
  không rỗng phải ghi đè keyring cũ để phục hồi máy từng lưu token sai; không có env
  thì desktop/harness đọc lại keyring.
- Token agent nằm trong credential store native của hệ điều hành, account
  `agent-auth-token`. SQLite chỉ lưu `agent.settings.v1` với `autoRepair`.
- Mỗi UDID có `AgentStatus` cache và dùng cùng slot lock với relay/session/stream.
  Generic health command `agent_preflight` vẫn phải kiểm tra metadata cài đặt,
  protected auth, session và frame MJPEG; không được khôi phục cache boolean kiểu
  "đã thấy bundle là xong". Rieng Interaction execution khong goi command nay:
  no dung non-mutating inspect + atomic foreground/session/MJPEG transition o §3.12
  de khong tao session/stream truoc fresh-text sequence.
- Metadata chỉ khớp artifact khi đồng thời đúng bundle/version/build, payload app
  `777wealth.app` và signer identity trong manifest. Bản Wuhan có cùng
  `com.mrph.svc` / `1.0` / `1` nhưng payload/signer khác nên bắt buộc repair.
- Repair dừng stream trước, xoá session, dừng relay, chỉ gỡ đúng bundle trong
  manifest, kiểm checksum rồi mới cài và dựng lại theo thứ tự session-trước-stream.
  Auto-repair chỉ chạy khi app thiếu hoặc metadata artifact lệch; lỗi protected auth,
  session hay MJPEG không được reinstall lặp. Background poll backoff 30 giây rồi
  thử dựng lại transport khi state `Error`; state `Missing` / `RepairRequired` chỉ
  tiếp tục sau lần Check/Repair tường minh.
- Ordinary unified session chỉ điều khiển màn hình và phải báo
  `supports_text_input=false`. Chỉ fresh session tạo sau khi TikTok foreground mới
  báo `true`. Nếu fresh transition lỗi, xoá trạng thái nửa chừng và phục hồi ordinary
  session + stream theo best effort trước khi trả lỗi gốc.
- Desktop expose `agent_get_settings`, `agent_save_settings`, `agent_list_statuses`,
  `agent_preflight`, `agent_repair` và `agent_bulk_repair`. Nút Agent của sản phẩm
  phải gọi nhóm lệnh này; các lệnh re-sign Apple ID/stock chỉ dành cho rollback/debug
  và không cung cấp text comment tin cậy.
- Nurture job có `commentProb > 0` phải generic-preflight toàn bộ UDID trước khi báo
  started. Interaction comment job dung atomic inspect/foreground/fresh-session/
  MJPEG path o §3.12, khong goi generic preflight. Ca hai engine phai chan neu
  driver/session khong quang ba text capability; khong tu roi ve emoji fallback.
- Hai kết quả `TextNotArmed` liên tiếp phải dựng fresh text session mới, mở lại stream
  rồi thay đồng thời session của feed và watcher. `TextNotSent` không được retry vì
  trạng thái gửi là mơ hồ.
- Milestone hiện tại chỉ hoàn thiện runtime Agent hợp nhất và text comment. Các phase
  2-6 của capability control plane vẫn chưa triển khai; MDM/full fleet policy thuộc
  phase 3 và được để lại cho kế hoạch sau.
