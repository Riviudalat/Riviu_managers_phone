# Hướng dẫn cho agent tiếp nhận dự án

> **Luôn cập nhật file này.** Sửa gì ảnh hưởng tới kiến trúc, ràng buộc thiết bị,
> hay danh sách "đừng làm lại" thì cập nhật ngay trong cùng lần thay đổi đó.
> File này là thứ đầu tiên agent sau đọc.
>
> **Cập nhật lần cuối:** 13/08/2026.

---

## 1. Dự án này là gì

**Riviu Manager** (tổ chức: **Riviu Tech**) — app desktop (Tauri + React) điều khiển
một dàn điện thoại qua USB để nuôi tài khoản TikTok: xem video, thả tim, follow,
bình luận, tự đóng popup. Hai nền tảng sau một control plane: iPhone qua
`crates/ios-driver`, Android qua `crates/android-driver`.

### 1.1 Tên: đổi ở desktop, KHÔNG đổi ở artifact iPhone (13/08/2026)

Đổi tên ngày 13/08/2026 từ `Riviumanagersphone`. Ranh giới này là **cố ý**, đừng
"làm nốt cho đồng bộ":

| Đã đổi | Giá trị mới |
|---|---|
| `productName` | `Riviu Manager` / `Riviu Manager Full` |
| `identifier` | `com.riviu.manager` / `com.riviu.manager.full` |
| Tiêu đề cửa sổ, sidebar, `index.html`, README, NOTICE | `Riviu Manager` |

| **Giữ nguyên, và vì sao** |
|---|
| `com.riviu.managersphone.agent[.xctrunner]` — nằm trong IPA đã ký, bị ghim SHA-256 ở §3.15 và trong `text-manifest.json`/`candidate-manifest.json`, và là `EXPECTED_BUNDLE_ID` của `probe_gate_bc.py`. Đổi = ký lại trên Mac + **tin cậy profile thủ công lại trên từng iPhone** (§4.0 nói rõ không có đường lập trình). |
| `sidecars/wda/Riviumanagersphone.ipa` và `sidecars/wda/branded/**` — CI gác bằng `git diff --exit-code` (`desktop-ci-cd.yml`). |
| `sidecars/wda/WebDriverAgent/**/Info.plist` — nằm trong digest của `legacy-wda-source-lock.json` (§3.18). Sửa một ký tự là vỡ lock, build đổ. |
| Thông báo "giữ app Riviumanagersphone" trong `riviu_pmd.py` và `crates/signing` — chúng nêu tên app **trên iPhone**, vốn không đổi. Đổi chữ mà không đổi app là chỉ sai chỗ cho người vận hành. |
| Literal `riviu-managers-phone` ở `state.rs::resolve_desktop_data_dir` và `SERVICE` trong `credentials.rs` — **không** suy ra từ `identifier`. Giữ nguyên chính là thứ bảo toàn SQLite (campaign, flow, cấu hình) và token agent trong Keychain. Đổi chúng là mất dữ liệu thuần, đổi lại con số 0. |
| Tên crate/binary `riviu-managers-phone` — không lộ ra người dùng, đổi thì lan sang workflow và `driver.ps1` `$ProcName` mà không được gì. |

Hệ quả đã biết và đã chấp nhận: `identifier` đổi nên máy đang chạy `v0.1.1` sẽ nhận
bản cập nhật kế tiếp thành **một app thứ hai nằm cạnh**, không phải nâng cấp đè.
Dữ liệu không mất (thư mục data không đổi), nhưng người vận hành phải tự gỡ bản cũ.

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

Watcher xử lý 4 loại màn hình chắn đường: `ClosableSheet` (tap ✕, gồm cả thẻ
khuyến mãi nổi có nút ✕ góc trái),
`InterestPicker` (tap nút bỏ qua), `LiveRoom` (tap ✕ — vuốt chỉ cuộn trong
phòng), và `SystemAlert`.

Toast TikTok `Bạn sẽ thấy ít quảng cáo như thế này hơn` không có nút đóng ổn
định: `screen.rs` đánh dấu nó là `ad_feedback_notice` trên một frame Feed nhưng
`feed_ready()` trả false, để watcher/nurture chờ toast tự biến mất trước khi
đọc caption hoặc gửi gesture. Không dùng tọa độ tap mù cho lớp toast này.

Ngay sau khi TikTok được đưa lên foreground, nurture chờ watcher xác nhận một
frame `Feed` (tối đa 12 giây). Vì vậy thông báo/popup đã có sẵn lúc khởi động
được đóng trước gesture đầu tiên; watcher vẫn chạy liên tục để xử lý popup phát
sinh giữa phiên. Không dùng `autoDismissAlerts` để làm việc này.

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

### 3.9 Project 2 Riviu Agent candidate (checkpoint Mac 04/08/2026)

Source candidate nam o `sidecars/wda/riviu-agent/` theo mo hinh pinned overlay:
`Scripts/prepare.py` verify npm tarball WDA 15.1.4, baseline digest
`f40eadb1e1d9872ad5a0574a5146cdbf5e0d04768ccb1f1701b289d50e4ee8f8`, roi
apply dung thu tu nam patch co SHA-256 trong `baseline-lock.json`. Source sinh ra
chi nam trong ignored `target/riviu-agent/source`; khong vendor de len Git va khong
sua `sidecars/wda/WebDriverAgent/` stock 16.0.0.
Digest sau patch phai dung
`c54c85ab5abafd6465dfa7f40933bf525d8016c928680d9f4153d9972115cf93`;
`prepare.py` khoa `git -c core.autocrlf=false` de giu LF cua upstream. Khong tai
sinh patch voi line-ending churn lam delta Objective-C thanh thay toan file.
Digest tinh moi regular file va canonical mode (`0644` hoac `0755`), gom ca
`project.pbxproj`, build config, `.plist` va executable bit cua build script;
khong duoc thu hep source attestation ve mot danh sach suffix hoac bo mode. Tren
POSIX, prepare phai dat mode that tu tar de `embed-runner-icon.sh` chay duoc.

Candidate protocol v2 dung `RIVIU_AGENT_TOKEN` (toi thieu 32 byte UTF-8), header
`X-Riviu-Token`, control `8916`, MJPEG `9094`. Chi exact `GET /status` duoc mien
auth. Protected health tra `agentVersion=0.1.0`, `protocolVersion=2`, logical
`375x667` va candidate mac dinh dung bon muc `stream/tap/swipe/clipboard`.
Artifact promoted rieng `sidecars/wda/RiviuAgent-text.ipa` chi them `text` khi
manifest da qua text gate; relay phai forward `RIVIU_AGENT_TEXT_CAPABLE=1` qua
DVT launch. Khong advertise `pushMedia`.

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
Envelope cua gesture co the tra `sessionId` moi khi focus composer lam XCTest
rotate session; WdaClient va live probe phai nhan id nay truoc `/wda/keys`. ACK
HTTP 200 van khong la bang chung text neu frame chua doi va nut Gui chua do.

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
Project 2 da noi candidate vao desktop theo mot profile rieng sau khi Gate B/C
PASS; candidate mac dinh van khoa text/comment theo feature list bon muc. Text
duoc promote thanh artifact rieng sau mot probe TikTok co frame that, khong thay
production oracle. Soft/hard runtime recovery van chua thuoc phase nay: moi
control/session fault lam Gate C fail; budget recovery thuoc Project 4. O phase nay
chi MJPEG reader duoc reconnect co gioi han toi da mot lan.

Desktop sidecar ap dung gioi han nay cho stream dai han: neu socket MJPEG dong
dot ngot, reader cho usbmux/agent san sang roi tao mot forwarder moi va thu lai
mot lan. Neu lan thu hai that bai thi producer ket thuc de supervisor/sampler
bao loi; khong retry vo han. Capture huu han (`max_frames`) khong reconnect de
giu dung so frame yeu cau. Moi lan doc MJPEG cung co deadline 2 giay; socket
khong EOF nhung khong tra byte moi duoc coi la stalled va di cung nhanh reconnect
co gioi han, tranh giu reader task song vo han voi sequence dung yen.

Trang thai hien tai: source/contract/build/probe fixture tren Windows da PASS; Mac
candidate build/sign va install identity tren hai iPhone that da PASS. Candidate
IPA da co ten hien thi `Riviu Agent` va logo chu R tu `logo.jpg`, duoc kiem tra
truoc khi ky lai. IPA candidate trong desktop la artifact rieng; production
`sidecars/wda/RiviuAgent.ipa` va `agent-manifest.json` van giu nguyen.
B0, Gate B va Gate C chinh thuc tren iPhone thu hai da `PASS` o buoc DVT
plain-launch, protected health/session/JPEG va cleanup.
Ngay 04/08/2026, evidence live ghi nhan 5/5 cold launch, 5/5 status identity,
5/5 session, 2,852 frame trong 300 giay, max gap 0.18 giay, 50 tap, 20 swipe,
Unicode read-back, clipboard byte-exact va cleanup sach. Trusted upgrade path
tren ca hai iPhone van duoc giu o `SUPPLEMENTAL_ONLY`; HTTP port hoac `/status`
200 rieng le khong chung minh automation readiness. Production artifact van duoc
bao toan voi SHA-256:
`8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea` va
`e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a`. Xem
`docs/superpowers/specs/2026-07-29-riviu-agent-standalone-control-parity-design.md`
va `docs/re/riviu-agent/`.

Text gate da co bang chung TikTok that, khong dung chuoi mau: probe
`sidecars/wda/riviu-agent/Scripts/probe_tiktok_comment.py` bat buoc nhan
`--comment-text`, reject `Riviu test`/fixture/placeholder va doi operator xac
nhan frame Send. Evidence build `2` o
`docs/re/riviu-agent/tiktok-comment-build2-live.json` cung `before.jpg`,
`drawer.jpg`, `armed.jpg`, `sent.jpg`; comment da gui la
`Quán cà phê này dễ thương quá ạ`. Promotion tao
`sidecars/wda/RiviuAgent-text.ipa` + `sidecars/wda/text-manifest.json` voi
`artifactVersion=0.2.0-text`, `bundleBuild=2`, `features` gom `text`, va app rieng
`/Applications/Riviumanagersphone Full.app` build voi `RIVIU_DEFAULT_AGENT_MODE=full`.
Sau lan uninstall/install de kiem build `2`, device 1 da approve Apple
Development va probe comment build `2` PASS; device 2 da cai cung IPA nhung van
cho approve profile truoc khi lay protected health/JPEG. Khong coi HTTP 401, port
mo, hoac evidence cua runner cu la bang chung B/C cua build moi.
Build record, patch/diff va rollback da duoc ghi tai
`docs/re/riviu-agent/full-build/verification.txt`,
`docs/re/riviu-agent/full-build/full-build.diff` va
`docs/re/riviu-agent/full-build/rollback.sh`; oracle backup nam trong
`target/riviu-agent/rollback/production-oracle/` (ignored).

### 3.10 Handoff bat buoc khi mo du an tren Mac

Agent tiep nhan tren Mac phai tiep tuc dung checkpoint Project 2 hien tai, khong
lap lai forensic/Gate A va khong ghi de production IPA. Candidate B0/Gate B/Gate C
da co evidence; lan tiep theo la trust profile, xac nhan build `2` va chay lai text
probe tren iPhone that truoc khi thay production artifact.

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
  --manifest target/riviu-agent/artifacts/0.1.0/candidate-manifest.json \
  --wait-for-trust

`--wait-for-trust` dung trong live run tuong tac: probe se dung sau fresh-install
de Trust/Verify Apple Development profile tren iPhone, roi nhan Enter de tiep tuc.
Flag nay khong ha nguong gate va mac dinh van tat.

Neu app candidate hien tai da duoc trust, dung `--reuse-trusted-install` cho vong
functional lap lai: installation_proxy se `Upgrade` IPA ma khong uninstall bundle,
giu lai approval. Report se mang `SUPPLEMENTAL_MAC_DEVICE`/`SUPPLEMENTAL_ONLY`;
Gate B/C chinh thuc van dung fresh-install mac dinh.

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
feature list mac dinh van chi co `stream/tap/swipe/clipboard`; desktop Full chi
advertise `text` tu manifest promoted rieng. Gate TikTok comment da co frame that
voi chuoi noi dung co nghia, nhung moi build moi phai lap lai fresh session truoc
MJPEG, focus composer, Unicode read-back/armed-send frame, tap Send va frame xac
nhan comment da gui truoc khi cap nhat artifact text. Production oracle van khong
doi trong phase nay.

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
F1.1 da them `GenerationFrameSource`/`GenerationFrameStream` trong core va fan-out
rieng trong `StreamHub`: frame chi duoc tra khi dung exact generation; subscriber cu
nhan `Advanced` ngay ca khi con frame buffered, hub dong tra `Closed` thay vi treo.
Flow Screenshot phai publish frame tu exact owned stream generation + hash artifact,
khong goi WDA `GET /screenshot`.

Attempt state phai commit intent truoc effect; nonterminal effect khi restart phai
reconcile thanh proved success/proved non-delivery/`Uncertain`. Khong retry
`Uncertain` tap/swipe/type. Artifact dung path UUID nam trong managed root, publish
temp -> validate/hash -> atomic rename -> DB transaction va reconcile orphan khi mo
app. Flow shutdown phai stop + join het worker truoc
`DeviceControlPlane::shutdown_cleanup()`.
F1.2 da implement `FlowArtifactStore`: label chi la metadata, path gom bon thanh
phan UUID canonical; staging va final deu nam trong mot root da canonicalize. Store
kiem exact JPEG/PNG decode, `sync_all` file staging, hash/size truoc rename, rollback
idempotent va startup reconcile stale staging, missing/hash mismatch cung orphan vao
`.quarantine`. `FlowArtifactRecord` nam trong model dung chung de Task 3 ghi DB cung
mot type, khong tao projection artifact thu hai.

F1.3 da implement durable run repository trong `crates/core/src/db/flow_runs.rs`.
Moi mutation run/device/attempt/artifact mo mot `IMMEDIATE` transaction rieng, tang
`flow_runs.event_revision` va chen `flow_events` trong cung transaction. `get_flow_run`
doc run/device/attempt/artifact trong mot deferred snapshot de khong tra projection
bi rach. Read-back recompute plan hash, doi chieu exact compiled node/action/side
effect, canonical raw `compiled_json`, revision-row hash va event ledger lien tuc
`1..=event_revision`; DB drift phai fail closed. Frozen `target_udids` luon sort
lexicographic. Tap/Swipe/Type Text sau dispatch chi duoc
`Uncertain`, tru khi exact proof cho thay request chua toi device.
Device run chi di `Queued -> Preflight` khi chua co snapshot, sau do
`Preflight -> Running` khi capability snapshot da duoc persist cung event. Attempt
dau tien va moi effect boundary chi duoc qua khi device dang `Running` voi snapshot.

Khong bypass cac durable boundary sau:

- `Queued -> IntentCommitted` bat buoc ghi canonical input va typed baseline; hai
  field nay bat bien neu attempt read-only duoc `Interrupted -> Queued` de reclaim.
- Evidence thanh cong chi duoc ghi moi tai `Verifying -> Succeeded`, phai co exact
  envelope `kind/matched/observedSha256/measurement`, `matched=true`, dung kind va
  identity/threshold cua compiled postcondition. Process proof phai bind `oldPid`
  vao process baseline; frame proof phai bind exact `generation` + `baselineSha256`.
  Screenshot khong di qua transition generic: chi `publish_artifact_and_succeed`
  duoc chen artifact row + `Succeeded` atomic; path phai la exact canonical
  `run/device/attempt/artifact.ext`, label/format/hash phai khop compiled Screenshot.
- `Verifying -> FailedVerified` bat buoc co typed `FlowErrorRecord` mang exact
  `attemptId`; read-back thieu diagnostic nay phai fail closed. Neu cung luu evidence
  thi envelope phai dung compiled postcondition, `matched=false`, bind exact
  baseline/locator/target va measurement phai thuc su khong dat postcondition; khong
  duoc chi tu khai false tren mot proof dang thanh cong.
- Proof transport non-delivery la exact JSON
  `{"kind":"transportNonDelivery","requestReachedDevice":false}`. Khong them key,
  khong suy tu timeout/ACK.
- Baseline JSON release-1 la exact typed object: `none` chi co `kind`; `process` co
  `kind,bundleId,pid` (`pid` null hoac so duong); `frame` co
  `kind,generation,jpegSha256,imageWidth,imageHeight,rgbBase64`, trong do RGB decode
  phai dung `width*height*3`. Doi schema nay phai doi repository + verifier + test
  cung luc, khong de runtime tu tao Value khac contract.
- `retry_safe` chi tu 0 thanh 1 mot lan cho `failedVerified` + `idempotentSet`, bang
  proof read-back `matched=false` dung compiled target. Terminate bat buoc exact
  bundle va post-PID = pre-effect PID da commit; proof duoc ghi cung event. Retry tao
  attempt number ke tiep; `create_flow_attempt` chi reopen sau khi attempt truoc
  thuc su `retryAllowed`, device dang `Failed`, error cua device tro dung attempt do,
  va release proof owner `Script` da duoc persist. Release proof cu nam trong
  `deviceRunTerminal` event truoc khi row projection duoc clear de reopen.
- Run projection chi duoc recompute khi tap device row khop exact frozen target
  snapshot. Thieu mot device khong duoc bao `Succeeded`; zero non-skipped la
  `Failed/NoEligibleDevice`. Device terminal bat buoc co release proof owner `Script`
  va khong con attempt active; successor `Queued` duoc giu lai de retry tiep tuc.
  Rieng `Succeeded` chi tu device `Running` co snapshot, khong con successor, va
  latest attempt cua moi compiled node deu `Succeeded`. Error co `attemptId` phai
  tro dung terminal failed attempt **moi nhat cua node** tren chinh device. `Skipped`
  chi hop le cho selection `AllEligible`, khi device con `Queued`/`Preflight` va chua
  co attempt; khong duoc doi device `Running` thanh skipped de che loi. Recompute doc
  full projection, khong doc rieng raw state. `get_flow_run`, recompute va startup
  loader deu phai mirror cac guard nay; moi device terminal deu cam attempt active
  con sot lai.

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

`terminate` khong con la false-success surface. Sidecar dung bounded DVT
`ProcessControl`: doc PID duong hien tai, kill dung PID do mot lan, poll den khi bundle
absent va tra exact `{ok,bundleId,oldPid,running}`; moi await cua operation/cleanup deu
co deadline rieng. `app-process` dung cung bounded setup/cleanup nhung chi doc, khong
kill. Rust chi nhan exact bon field, bundle phai khop, PID phai null hoac so duong va
`running == pid.is_some()`; payload best-effort cu, field thieu/thua, PID 0/sai kieu va
state mau thuan deu la protocol error. `ProcessAbsenceProof` va `AppProcessState` la
typed contract; `supports_verified_app_termination` mac dinh false. Pmd chi advertise
sau `ping` thanh cong voi `pymobiledevice3=true`, exact
`pymobiledevice3==10.1.0`, import duoc async `DvtProvider` + `ProcessControl`,
`sidecarProtocolVersion=2` va exact contract `verifiedProcessControl`; exit 2,
dependency thieu/sai version/sai API, sidecar cu/degraded hoac handshake sai deu fail
closed. Mock phai bat explicit.
`DeviceControlPlane::driver_contract_ids()` chi cong bo `verifiedProcessControl` theo
capability nay. Legacy JobQueue va Flow deu di qua context owned cua cung control-plane
lock theo UDID; khong goi sidecar truc tiep.

`syslog` van chi tra sample text, nen de ngoai Flow release 1 cho toi khi os_trace path
that co contract/live test.

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
orientation; runtime mismatch phai fail truoc dispatch. Terminate App da duoc bat trong
release-one catalog o F1 voi config exact bundle, resource Bridge, side effect
IdempotentSet, evidence ProcessAbsent, reconciliation ReadProcess va retry
IdempotentAfterRead. Compiler va legacy importer deu tao typed Terminate; raw HTTP/WDA/
Shell van bi gate. Recovery chi duoc `ReadProcess`, khong kill hoac redispatch de doan
retry: absent la ProcessAbsent, cung positive PID baseline la non-delivery, PID duong
khac la Uncertain.

Foundation F0 da dong ngay 30/07/2026. Source/verification commit range bao gom tu
`c5308d3c3878b0e40f8de925ad5fe3de632e1f08` through
`e98da9c880a23082bf51379516c155d615df99f4`; rollback van la
`805056790d890046384ad7a578cc34a99088e799`, khong day baseline theo implementation.
Gate da PASS `cargo fmt --all -- --check`, `cargo test --workspace` (377 passed,
1 ignored fixture writer), `cargo clippy --workspace --all-targets -- -D warnings`
va `git diff --check`. F0 van cam `Terminate`, moi TikTok/domain node va moi Flow
runtime/device dispatch; catalog chi co foundation release-1 da kiem chung. Gate tiep
theo la F1 tai
`docs/superpowers/plans/2026-07-30-riviu-flow-v2-runtime.md`; khong bat F2/F3 truoc
khi F1 co checkpoint rieng.

F1 Task 4 checkpoint ngay 31/07/2026: Python app-control tests PASS 10/10; Rust Pmd
tests PASS 33/33; DeviceControl PASS 33/33; JobQueue PASS 3/3; core Flow PASS 46/46;
toan bo Python sidecar PASS 42/42; script-engine Flow PASS 23/23. Khong lam lai
bounded terminate/parser/ownership/catalog o task sau. Task 6 phai dung proof va
read-only process route nay de persist evidence/reconcile, khong them duong terminate
rieng.

F1 Task 5 checkpoint ngay 31/07/2026: evidence tests PASS 15/15; WDA PASS 32/32;
mock PASS 15/15; Pmd PASS 33/33. `FlowCancellation` trong `flow` la primitive duy
nhat cho Task 6/7; khong tao token executor rieng. Frame baseline persist exact
generation + SHA-256 JPEG + width/height + RGB base64 va verifier chi dung
`GenerationFrameSource`. Moi `GenerationFrame` co sequence tang don dieu trong mot
generation; postcondition chup watermark khi verifier bat dau sau effect va chi nhan
frame co sequence moi hon, khong dung cached frame truoc verifier. Baseline phai recheck
generation sau decode; generation advance tra `StaleGeneration`, stream dong tra
`StreamClosed`. ACK gesture/keys khong duoc doi thanh success neu evidence khong match.
Process proof phai khop exact bundle va pre-effect PID; proof sai la invalid/uncertain,
read-only same PID dung retry-safe schema `bundleId/pid/preEffectPid`, PID moi la
uncertain. Ca success va retry-safe process envelope da duoc test qua transition DB that.

Qualified read-back chi advertise tren session `FreshText` cua profile RT-MMO. WDA
chi POST singular `/element` voi exact `accessibility id|class name`, chap nhan W3C
hoac legacy element ID neu khong mau thuan, sau do GET exact `/element/{id}/text`.
Hai request dung chung mot absolute deadline va moi request tu tinh remaining timeout;
loi khi doc response body phai giu `UiErrorKind::Timeout`; khong boc request bang
`tokio::time::timeout`. Stock/ordinary session van false va `snapshotMaxDepth=1`
khong thay doi. Task 6 phai check live flag truoc dispatch
`accessibility.visible|accessibility.readText`, khong tao session implicit.

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

F1 Task 6 checkpoint ngay 31/07/2026: executor nam o
`crates/core/src/flow/executor.rs`, ownership wrapper nam o
`crates/core/src/flow/device_context.rs`. Plan co device resource chi acquire mot owner
`Script` va chi nang don dieu `Exclusive -> Session -> Streaming -> Closed`; khong
close/reacquire giua node. Plan compiler-valid `Start -> Wait -> End` la target-free,
khong acquire device/inspect target va persist typed target-free preflight + release
proof `hadSession=false/hadStream=false`. Launch dau tien di qua atomic
`foreground_target_app_and_start_interaction_session` dung mot lan; Pmd va Mock
override contract nay de backend chi foreground mot lan truoc khi tao ordinary/fresh
session. Control-plane hien can exact capacity reservation truoc moi UI session, ke ca
plan khong doc frame; MJPEG van chi start sau session va reservation duoc giu den khi
close.

Executor persist target-qualified preflight gom exact `DeviceCapabilitySnapshot`,
`AgentStatus` da dung de derive static capability IDs va chinh tap capability IDs.
No persist `IntentCommitted` + typed baseline truoc effect, doi chieu frame decode voi
`QualifiedGeometry.pixelWidth/pixelHeight`, roi check lai generation/geometry ngay
truoc coordinate dispatch. Deadline evidence 5 giay duoc reset sau khi bootstrap
session/stream xong. Config/action kind lech phai fail truoc device effect; ACK
gesture/text khong thanh success neu verifier khong match. Assert Visible dung fresh
read-back session nhung khong bat stream; request read-only fail thanh
`FailedVerified`, khong thanh `Uncertain`. Wait kiem cancellation moi toi da 250 ms.
Bridge-only Terminate dung process proof va khong doi geometry UI. Moi close thanh
cong phai tra `ContextReleaseProof` cua worker;
`clean_ticket` invalidate session sau khi dung stream de close dung mot lan. Executor
kiem cancellation/deadline ngay truoc va sau read-only process baseline; cancellation
trong sidecar read khong duoc persist `EffectDispatched` hay goi Terminate sau khi read
tra ve. Khong boc request dang chay bang `tokio::time::timeout`. Executor
recompute canonical execution SHA-256 va khop exact `flow_runs.plan_sha256` truoc khi
acquire device; cung flow ID/revision nhung canonical execution hash khac phai fail
`RunIdentityMismatch` khi device run van `Queued`.

Public `start_reserved_stream` van giu cleanup bat dong bo cua caller cu. Rieng Flow
dung recoverable upgrade va `StreamHandoffProof`: generation duoc ghi ngay truoc
session, nen ca direct start error/cancel va missing first frame deu stop exact
generation, close session va release capacity qua worker. Backend co the clear StreamHub
va tang generation truoc khi tra direct start error; nhanh dong bo nay phai lay lai
non-destructive handoff proof va cap nhat ticket bang exact post-cleanup generation.
Cancellation/partial-live van giu generation cu; chi stop proof sai moi quarantine.
Capacity reserve fail sau `Running` phai khoi phuc/close exclusive context
va persist terminal release proof. Neu setup session/stream cua Launch dau tien loi,
attempt giu nonterminal diagnostic o `EffectDispatched`, doc active app mot lan de
phan loai `Succeeded`, proved non-delivery `FailedVerified` + retry-safe, hoac
`Uncertain`; khong relaunch.

F1 Task 7 checkpoint ngay 31/07/2026: multi-device runtime nam o
`crates/core/src/flow/runtime.rs`. Runtime co lifecycle mot chieu
`Recovering -> Ready -> Stopping`; cung mot admission mutex bao ve startup recovery,
enqueue, retry va shutdown de khong spawn worker qua bien shutdown. Enqueue tao
`flow_run` + toan bo frozen `device_run` trong mot `IMMEDIATE` transaction roi moi
spawn. Moi device khoi tao toan bo attempt release-1 trong mot transaction; claim
`Queued -> IntentCommitted` kiem latest attempt, tat ca predecessor `Succeeded` va
khong co `Uncertain` tren cung device trong chinh transaction do.

Startup recovery, moi run va moi retry deu co top-level Tokio task duoc runtime track;
cac device future nam trong `join_all` cua task cha, khong detached spawn. Cancellation
danh thuc acquire/Wait nhung khong huy WDA request dang chay. `shutdown()` chuyen
`Stopping`, bat dau global deadline **truoc khi** doi admission, cancel, drain va join
tat ca handle trong 30 giay. Chi sau deadline tong moi abort owned task, await handle
da abort va persist `ShutdownDeadlineExceeded`; khong boc tung WDA request bang
`tokio::time::timeout`. Khong doi `join_all` thanh cac child `spawn` roi lam roi
`JoinHandle`: abort task cha hien tai cung drop toan bo device future do no so huu.

Startup doc full immutable run/device/attempt/artifact aggregate truoc khi nhan run
moi. `IntentCommitted` thanh `FailedBeforeDispatch`; read-only active attempt qua
`Interrupted -> Queued`; queued successor chi reclaim khi predecessor latest deu
Succeeded va device khong co Uncertain. Numeric stream generation khong co epoch nen
khong bao gio duoc tin qua desktop restart, ke ca khi so bi reuse: Tap/Swipe frame va
Type Text matched read-back van thanh `Uncertain`. Launch/Home chi doc active app va
khong foreground lai. Terminate chi doc PID: absent la success, cung pre-PID la
`FailedVerified` + proved retry-safe, PID khac la `Uncertain`; khong kill lai.
Screenshot chi adopt exact canonical artifact cua dung run/device/attempt sau khi
decode/format/hash pass; absent, nhieu file, symlink hoac invalid deu `Uncertain`.
Luc chay moi, Screenshot recheck exact stream generation ca sau atomic file rename va
ngay truoc DB publication; generation doi thi rollback ca staged/final file.
Type Text recovery doc exact locator bang fresh-text session sau khi da xac nhan
active app, khong foreground target.

Retry idempotent-set bat buoc doc live state lai ngay truoc khi tao attempt moi;
khong tin `retry_safe` cu. Proof chi duoc expose sau khi device da `Failed`, error
attributed dung latest attempt va transaction `retrySafetyProved` commit. Retry chi
reacquire mot device, skip predecessor da Succeeded va tiep tuc queued successor;
`Uncertain` Tap/Swipe/Type Text khong co duong retry. Recovery terminal release proof
OR resource da quan sat voi frozen `ContextPlan` de crash khong ha thap cleanup claim.
`FlowRunUpdated` chi mang run ID + event revision da commit va runtime khong emit lui
revision. Fresh retry proof chi gate viec tao attempt moi; attempt moi van capture
baseline/evidence cua chinh no sau khi reacquire, khong dung proof cu lam success.
Khi resume sau nhieu `Launch App`, phai xac nhan bundle cua Launch thanh cong gan nhat
truoc node tiep tuc, khong mac dinh quay ve `initial_bundle_id`.

Verification Task 7 hien PASS: runtime 35/35, executor 26/26, Flow DB 21/21,
artifact store 11/11, JobQueue 3/3, DeviceControl 36/36; full `riviu-core` PASS 254
unit (1 ignored fixture writer) + 15 real-frame tests va clippy all-targets
`-D warnings`. Day van la core runtime checkpoint, chua wire Tauri command/startup
composition root hay Flow React UI; F1 final gate duoc ghi ngay ben duoi.

F2 composition root bat buoc goi `FlowRuntime::recover_startup()` truoc generic
`FlowArtifactStore::reconcile()`: recovery phai co co hoi adopt exact artifact cua
attempt nonterminal truoc khi orphan scanner quarantine file do. Global reconcile sau
do phai nhan day du committed artifact rows, khong chi artifact cua run nonterminal.
Event runtime hien co dam bao monotonic/post-commit o admission va khi device/recovery
hoan tat; F2 khong duoc quang ba live per-node refresh cho toi khi co post-commit
invalidation callback hoac polling command duoc test.

Runtime F1 da dong ngay 31/07/2026. Implementation commit range la
`b5ad940534a2eb75f207659d4a703cca14a220c0` through
`60c7dfca2f0b0ba9ceb64e41485127cce29c6f90`; rollback commit van giu nguyen
pre-F0 `805056790d890046384ad7a578cc34a99088e799`. Final gate PASS:
`python -m unittest sidecars.pymobiledevice3.test_app_control -v` 10/10;
`cargo fmt --all -- --check`; `cargo test --workspace` voi core 254 pass + 1
ignored fixture writer, real-frame 15/15, iOS driver 131/131, desktop 28/28,
script-engine 26/26 va cac signing/reconstruction suite deu PASS;
`cargo clippy --workspace --all-targets -- -D warnings`; `git diff --check`.
Production IPA va canonical-LF manifest van dung SHA-256
`8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea` va
`e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a`.

Termination F1 chi dung exact typed DVT process contract: read positive PID, kill
dung PID mot lan, poll absent trong deadline; already-absent la success. Recovery va
retry chi dung read-only `ReadProcess`, khong kill lai; payload legacy/field thieu-thua
hoac dependency handshake sai deu fail closed. TikTok domain nodes van khong co trong
release-one catalog; `Comment`, `ReplyToComment`, Save, Repost va Direct Message van
bi khoa cho toi khi `InteractionCampaignEngine` + persistence + G0-G3 va identity
artifact gate ton tai. `RawHttp`, `RawWda` va `Shell` cung khong nam trong catalog.

Gate tiep theo la Desktop F2 tai
`docs/superpowers/plans/2026-07-30-riviu-flow-v2-desktop.md`. F2 phai wire Tauri
commands/startup, runtime-qualified protected snapshot + geometry, Flow React UI,
startup artifact order va live node invalidation; khong chuyen thang sang F3. Mac
van phai chay B0/Gate B/Gate C cua Riviu Agent candidate rieng truoc khi thay production
oracle; F1 core PASS khong thay cho live device gate.

Snapshot metadata hien tai cua `PmdIosDriver::inspect_device_for_target` chua co
protected-auth proof va `QualifiedGeometry`, nen UI Flow tren Pmd that co chu y fail
closed o preflight. Them nua, park MJPEG cua Pmd ha cached Agent state tu `Ready` ve
`Starting`; khong doc cache sau park roi tu nang no lai thanh ready. Khong
hard-code/fabricate cac proof nay va khong goi generic preflight tao session/stream.
Buoc noi desktop/live sau phai cap runtime-qualified, target-bound snapshot co auth +
geometry va dinh nghia readiness truoc/sau park ro rang truoc khi dispatch;
bridge-only Terminate van chay duoc voi snapshot metadata. Task 6 moi la core
executor/control-plane; Task 7 da co core startup recovery nhung chua wire desktop
command hay composition root.
Khong gan bundle gia cho plan target-free va khong dua direct driver handle ra ngoai
typed Flow ownership.

### 3.15 Main integration va trang thai san pham trung thuc (31/07/2026)

Checkpoint F0/F1 lich su da duoc fast-forward tai
`89f19beeb3a48fe2352abb123d03ef0947c13fb3`. Flow F2 va F3 fixture/rollback sau
do da duoc fast-forward vao `main` qua
`9f5d2774c390fbb5b41e613ef3a03808a934e243`; dung `git rev-parse origin/main`
de lay documentation commit moi hon neu co. Sau checkpoint F0/F1 da chay va PASS:
`cargo test --workspace`, Python app-control 10/10, frontend Vitest 11/11,
`npm run build`, `cargo fmt --all -- --check`, va
`cargo clippy --workspace --all-targets -- -D warnings`. `npm run lint` exit 0,
con ba warning Fast Refresh o `Icons.tsx`/`SelectionStrip.tsx`, khong phai loi build.

Khong duoc ket luan "tat ca tinh nang app da xong" tu checkpoint nay. Trang thai
release hien tai phai duoc mo ta dung nhu sau:

- Nuoi account va text comment qua production RT-MMO da co live proof tren iPhone 8
  iOS 16.7.15. Luong nay van phu thuoc exact production IPA + RT-MMO token; chua co
  live regression tren Mac cho toan bo `main` moi.
- Source Riviu Agent candidate da PASS source/contract/fixture tren Windows; B0,
  Gate B va Gate C tren Mac da `PASS`. Candidate da duoc noi vao desktop voi
  profile rieng va mac dinh chi advertise `stream/tap/swipe/clipboard`. Artifact
  `RiviuAgent-text.ipa`/Full app da co manifest `text` sau probe comment that,
  nhung build `2` chua duoc live re-probe vi iOS dang doi trust profile. Desktop
  macOS tao WebView sau asset setup de tranh trang trang va gioi han Keychain
  candidate de bootstrap khong mac neu keychain dang doi user interaction.
- Flow V2 da dong F0/F1/F2: Tauri composition/commands, startup recovery, React
  drag/drop editor, exact revision save, run monitor va invalidation da co. F3
  Rust fixture, Playwright va rollback proof da PASS voi nhan `FIXTURE_ONLY`; F3
  live van `PENDING_MAC_DEVICE` cho toi khi Phase 4A protected-auth/geometry PASS.
- TikTok Interaction Campaign hien moi co reviewed design, control-plane foundation
  va Gate 0 source/fixture. `InteractionCampaignEngine`, persistence, Tauri commands,
  menu/UI dan link, scheduler, G0.12 live Mac va G1-G3 van chua hoan tat. Chua duoc
  bao rang luong dan link -> chon tat ca/rieng le -> mo post -> tuong tac da ship.
- Multi-account moi chi co schema/planner extension point; production van mot
  `device:<udid>:default` tren moi may va khong co account switching.
- Proxy chi co CRUD/assign/endpoint check + `manual_required`; may hien tai
  unsupervised nen khong co system-wide apply/verify/rollback tu dong.
- MDM/supervision/AdminControl, remote fleet policy va cac muc deferred o section
  3.11 chua nam trong phase hien tai.

Production artifact van phai giu byte-exact; artifact text la promoted sidecar
rieng va khong ghi de oracle:
`sidecars/wda/RiviuAgent.ipa` SHA-256
`8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea` va
canonical-LF `agent-manifest.json` SHA-256
`e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a`.
Mac build candidate vao `target/riviu-agent/artifacts/0.2.0-text/`, chay lai B0/B/C
cho build `2` sau khi trust, sau do lap lai TikTok comment end-to-end. Chi artifact
text duoc cap nhat theo transaction co rollback; khong ghi de production artifact
chi vi source/build fixture PASS.

### 3.16 Active priority: Interaction Campaign + Flow V2 (31/07/2026)

User da chot uu tien hai nhom: (1) menu `Tương tác`, dan link, chon tat ca/tung may,
All/RoundRobin, Run Now/Once, scheduler + durable campaign engine; (2) Flow V2 F2/F3
voi editor keo tha va run monitor. Plan dieu phoi active nam o
`docs/superpowers/plans/2026-07-31-interaction-flow-delivery.md` va la noi giai quyet
xung dot giua cac detailed plan cu.

Trang thai thuc thi: Flow F2 va F3 mock/Playwright/rollback da dong tren Windows.
Mac van phai dong G0.12; G1 Campaign core, G2 verified actions va G3 operator UI chi
bat dau production path sau exact G0.12 live report PASS. Flow F3 live van cho
runtime-qualified protected auth + geometry tren Mac/iPhone.

Cac quyet dinh bat buoc cho lan trien khai nay:

  - Interaction dung migration version 4 tren `schema_migrations` chung da co version
  1/2; khong tao migration ledger thu hai. Flow F2 khong thay schema: archive tra
  projection ngay trong cung transaction, con `FlowMutationCoordinator` tuan tu hoa
  commit + emit va cap revision invalidation tang dan trong tung phien desktop.
  Revision nay chi la cache hint; sau restart UI refetch projection SQLite truoc khi
  nhan event moi. Khong them migration Flow chi de luu cache hint vi se pha rollback
  pre-F0 va chiem version 3 da khoa cho Interaction.
- Flow F2 pin mot bo jsdom/Testing Library/Playwright va mot test config; G3 tai su
  dung, khong tao happy-dom/Vitest/Playwright config canh tranh.
- Initial metadata device scan phai populate registry truoc startup recovery. Sau do
  recover Flow/Interaction truoc reconcile artifact root rieng voi **toan bo**
  committed rows, khong chi run nonterminal; Exit chan admission va join ca hai
  runtime truoc `DeviceControlPlane::shutdown_cleanup()`.
- F2 da them bounded polling 750 ms cho run nonterminal va consume
  `FlowRunUpdated`; document list/editor consume `FlowUpdated`. Khong bo cac duong
  invalidation nay vi event F1 chi o completion/recovery khong du cho live monitor.
- Admission gate luc Exit phai chan va drain **moi** mutating command, gom ca
  save/archive/start/schedule/cancel/retry/settings/credential/DB write/queue insert,
  khong chi command doi man hinh. Khi G1 duoc compose, test Exit phai co dispatcher +
  scheduler that; F2 chi chung minh duoc phan shutdown truoc Interaction.
- Exit phai reject command moi, signal Nurture/Flow/Job stop, roi moi cho admission
  drain. Doi wait va stop se deadlock khi retry dang giu admission va cho device cua
  Nurture. Worker Flow run/retry phai dang ky `JoinHandle` qua registration barrier;
  khi ket thuc phai retire task, cancellation va cache emitted revision truoc
  shutdown. Khong quay lai registry chi duoc drain luc Exit.
- Truoc F3 live phai implement va gate snapshot Pmd target-bound co protected auth,
  `QualifiedGeometry` va readiness ro rang truoc/sau park stream. Truoc G5, ca hai
  iPhone acceptance phai co G0/G2 live tuple proof; G5 khong duoc tu tao capability.
- Flow/Interaction command error phai map tu typed service error, khong parse chuoi
  `anyhow`; artifact tren monitor phai co backend command validate row/path/kind/
  size/hash, khong render link no-op hay nhan arbitrary path. F3 dung evidence theo
  tung action, khong doi frame proof cho Launch/Home/Terminate/TypeText/Screenshot.

Riviu Agent candidate thay RT-MMO, multi-account switching, proxy system apply,
MDM/supervision, G4 Save/Repost/Direct Message, Flow branch/loop/cross-device va
A-comment/B-reply van giu o deferred roadmap. Viec uu tien Interaction/Flow khong
duoc dung de xoa hoac tuyen bo hoan tat cac muc nay. Production IPA/manifest va
qualification registry van theo exact gate/rollback rules o cac section tren.

### 3.17 Flow V2 F2 va F3 fixture checkpoint (31/07/2026)

Desktop F2 da dong tren Windows qua ba implementation commit:

- `13409b04201cf657c2553747f91ce3a834c408f6`: compose `FlowRuntime`, artifact
  store, startup recovery, 15 typed Tauri commands, exact save/archive, command
  admission va exit ordering. Initial device scan fail thi startup fail ro rang;
  run/retry workers co registration barrier va tu retire bookkeeping.
- `60875e7cfd1a7a25543322955c5759db9de24cad`: Flow React workspace co palette,
  canvas keo tha, custom node, inspector, validation, undo/redo 50 entry,
  draft restore, import/export/JSON, One/Selected/AllEligible, coordinate picker,
  run monitor va Legacy tab. `FlowUpdated` refetch revision sach; draft dirty chi
  refresh list va khong bi ghi de.
- `57eb3737e9198918b81a63410da6ed3cc62652f2`: headless fixture hai device,
  Playwright workflow va visual snapshot 1440x900/900x700.

F2 khong them database migration. Ledger van exact version 1/2; Interaction giu
version 3. `FlowMutationCoordinator` revision la invalidation hint trong mot desktop
process; restart phai refetch SQLite. Cancel/retry missing ID tra typed
`FlowRunNotFound`/`FlowAttemptNotFound`, khong parse error string.

Final Windows gate PASS: `cargo test --workspace` (core 263 pass + 1 ignored
fixture, ios-driver 131, desktop 40 va cac integration/doc target), workspace
Clippy `-D warnings`, rustfmt, Vitest 71/71, frontend build, Oxlint exit 0 voi 7
Fast Refresh warning khong chan, Playwright 6/6, Python app-control 10/10 va
`git diff --check`. Mock harness PASS: plan SHA-256
`88333ddcbb7ae804825e1902ad5c0a3d04431def5a947f65aabf8dae724173c4`, hai
device, 16 attempt, 0 uncertain, 2 JPEG da verify, stream max 2/2 va cleanup
context/stream/quarantine deu 0.

Rollback proof PASS tren ban sao DB tam: release migrate v1 -> ledger exact 1/2;
pre-F0 `805056790d890046384ad7a578cc34a99088e799` core/parser, frontend, desktop build
va desktop boot 5 giay cung PASS, SQLite `integrity_check=ok`. Windows khong duoc
dung `APPDATA` de co lap proof vi `dirs::data_dir()` dung Known Folder API. Dung
`RIVIU_MOCK_DATA_DIR` chi khi `RIVIU_MOCK_DEVICES=1`, path absolute, va apply
`docs/fixtures/rollback-pre-f0-mock-data-dir.patch` vao detached pre-F0 worktree.

Bao cao checkpoint o `docs/re/flow-v2/release-1.md`. F3 live chua dong: real
coordinate picker va Pmd UI Flow van fail-closed cho toi Phase 4A tren Mac/iPhone;
khong tao `gate-f3.json/md`, khong ghi live PASS va khong doi production Agent.
Production `RiviuAgent.ipa` van SHA-256
`8a24847099495ff70b998522692c43f00dd16b90f698bda6953a73f5d33002ea`; manifest
canonical-LF van
`e98a549af4c061556effd36424e7732219e1a6d262bcf1f259279975024b6e1a`.

### 3.18 Desktop self-contained packaging va CI/CD (31/07/2026)

#### 3.18.1 Mac local bundle checkpoint (04/08/2026)

`apps/desktop/src-tauri/tauri.conf.json` phai dat
`bundle.macOS.signingIdentity` = `"-"` cho build local. Neu bo field nay, Tauri
chi de linker ad-hoc signature tren executable; app co `Contents/Resources` nhung
khong co resource seal, `codesign --verify --deep --strict` fail va Finder khong
mo duoc. Sau moi build Mac phai kiem tra ca `.app` va DMG da mount, khong chi xem
file ton tai. Build da verify:

```text
target/debug/bundle/macos/Riviumanagersphone.app
target/debug/bundle/dmg/Riviumanagersphone_0.1.0_aarch64.dmg
```

Ca hai deu `codesign --verify --deep --strict` PASS; `open -W` giu process song.
Day van la ad-hoc artifact (`TeamIdentifier` khong co), nen `spctl`/Gatekeeper
van co the can phep lan dau; chua goi day la Developer ID/notarized release.
Ngay 04/08, `/Applications/Riviumanagersphone.app` van la ban cu voi resource
seal hong du du target bundle moi da dung; da thay bang bundle moi va verify lai
ngay tai duong dan Finder. Khi user bao app van loi, kiem tra ca hai duong dan
nay truoc khi rebuild lan nua. Ban cu duoc giu tai `/tmp` de rollback.

Desktop khong duoc panic khi `RIVIU_RTMMO_TOKEN` thieu. Setup luu loi vao
`StartupState`, frontend hien trang thai cau hinh va khoa thao tac thiet bi; no
khong tu fallback sang stock/mock va cung khong sinh token. Token hop le van phai
duoc dat qua environment mot lan de migrate vao native Keychain account
`agent-auth-token`, sau do mo lai app.

Tile background stream bat dau tinh deadline tu luc producer that su chay
(`mark_running` sau frame dau), khong tinh tu luc reserve; vi vay bootstrap
agent/MJPEG cham khong lam tile roi som. Khi frame dau da co, registry phai dat
`wdaReady=true` va frontend doc lai `latest_frame` de khong mat frame den truoc
luc WebView subscribe. Core fixture van co budget mac dinh 1 de giu gate
foreground; desktop Full tao `StreamBudgetManager::new(2)` va giu hai producer
MJPEG cho hai UDID cung luc. Khi budget desktop da du hai slot, deadline 5 giay
chi dung de phat hien socket dung va recycle producer; stream khoe tiep tuc
`Live`, khong park theo chu ky. Neu foreground preempt hoac budget mot slot thi
sampler moi park producer va giu anh cu voi nhan `Parked`; `Sampling` phai hien
thi dang mo stream thay vi `No stream`.
Sau khi park/preempt, agent co the bao `Starting` trong luc session cu vua dong;
sampler chi duoc quay lai neu tile da co `wdaReady=true` va khong
`Busy/Preparing`, con bootstrap lan dau (chua co frame) van phai cho den luc
`Ready`.

Ngay 05/08/2026 da sua dung contract nay: `DeviceDriver::park_owned_stream`
cho phep `DeviceControlPlane` dung producer nen ma khong go `StreamHub.latest`;
`StreamHub` van tang generation de frame buffered cua producer cu bi bo qua.
`PmdIosDriver` stop child MJPEG, park generation, roi sampler co the quay lai
producer moi. Moi ban Full phai duoc compile voi `RIVIU_DEFAULT_AGENT_MODE=full`
va build script phai khai bao `rerun-if-env-changed`, neu khong binary co the
dong goi candidate mode cu va bi chan token/stream ngay luc startup.
Sampler liveness dung `StreamHub.latest_frame_sequence` thay vi digest JPEG:
man hinh dung yen van co the phat nhieu frame byte-giong-nhau, va chi socket
khong phat frame moi trong tron luot 5 giay moi duoc danh dau `Stale`. Bo dem
sequence rieng theo UDID van tang qua moi generation; clear/park chi doi
generation va cache frame, khong cho frame dau cua producer moi lap lai so cu
va bi danh dau stale oan.
`DeviceOwned.stream_generation` phai khop generation hien tai moi duoc reuse
child; child con song nhung thuoc generation da park phai bi stop va spawn lai,
neu khong reader se publish vao generation cu va tile se giu cung sequence.

Release desktop khong duoc phu thuoc Python/pip/tidevice cua may nguoi dung.
`scripts/build_desktop_sidecar.py` dung PyInstaller onedir de dong goi Python,
`pymobiledevice3==10.1.0` va `tidevice==0.12.11` thanh
`sidecars/pymobiledevice3/runtime/riviu-pmd(.exe)` trong bundle. Driver uu tien
runtime nay; chi source/dev moi fallback sang `python3`/`python` + `riviu_pmd.py`.
Signer cung re-enter exact runtime qua allowlist `__script`; khong dua lai hard-code
`python3` vao packaged path. Moi child console Windows van phai giu
`CREATE_NO_WINDOW`.

PyInstaller loai IPython bang `pyinstaller_runtime_hook.py`: interactive shell cua
pymobiledevice3 khong nam trong product. Khong loai them module theo cam tinh. Toan
bo transitive closure bi khoa trong `requirements-lock.txt`; builder bo qua
distribution local khong lien quan nhung fail neu bat ky dependency active trong
lock bi thieu/sai version, va ghi exact active closure vao manifest. Collector phai
doi chieu tap closure bang tuyet doi voi lock, khong chi ba top-level package. Moi
thay doi dependency/hook phai PASS frozen `ping` co `verifiedProcessControl`, embedded
tidevice, signer, signing-resource self-test va Windows structured-error JSON, sau
do recompute runtime tree gom ca node type, POSIX mode va symlink target.
Tren macOS, runtime PyInstaller bat buoc map bang `bundle.macOS.files` vao
`Contents/Resources/sidecars/pymobiledevice3/runtime`, khong map qua generic
`bundle.resources`: Tauri resource walker bo directory symlink va dereference file
symlink, con macOS directory copier giu nguyen ca hai. Collector khoa exact overlay
va so cay trong DMG de thay doi nay khong bi vo tinh quay lui.
Lan do local Windows bang Python 3.14 giam tu 162,296,882 byte/6,650 file xuong
58,956,091 byte/734 file; `ping` khoang 0.43 giay. Khong bat Python `-OO` vi
`pymobiledevice3`/`tidevice` co assert tham gia vao hanh vi runtime, khong chi debug.
CI release pin exact Python 3.12.10, Node 24.15.0 va Rust 1.95.0; artifact manifest
moi la so do chinh thuc theo tung OS/architecture.

Root release profile dung `opt-level=3`, thin LTO, mot codegen unit, abort panic va
strip symbols de can bang runtime performance, kich thuoc va thoi gian build. Windows
NSIS la current-user install; WebView2 dung `downloadBootstrapper` va chi tai khi
thieu. Khong tu bundle/cai ngam Apple USB driver: Windows van can Apple Devices hoac
Apple Mobile Device Support de co usbmux. Mac run binh thuong khong can Python;
Xcode/Apple certificate chi la prerequisite khi rebuild/re-sign agent iPhone.

Legacy re-sign source trong desktop la stock WDA 16.0.0 o
`sidecars/wda/WebDriverAgent`, khoa boi `legacy-wda-source-lock.json` cung hash logo
va iconset. Digest source canonical hoa CRLF thanh LF, bind file type/content va
canonical mode 0644/0755; `executablePaths` phai khop mode that tren POSIX. Khong tao
lock tu Windows working-tree CRLF hoac bo executable mode. Packaged flow khong duoc
clone upstream HEAD va khong duoc build/ghi vao
signed `.app`: `build_and_install.py` verify resource roi copy source sang
`~/Library/Caches/com.riviu.managersphone/signing`, tach workspace bang hash UDID,
truoc khi sua/build. Day la legacy rollback path, khong thay doi Project 2 candidate
hoac production RT-MMO Agent.

`.github/workflows/desktop-ci-cd.yml` chay quality gate roi build ba artifact native:
Windows x64, macOS arm64 va macOS x64. Moi push `main` upload artifact 30 ngay; tag
`v*` chi publish khi exact `v<tauri/npm/cargo version>`. Toolchain va official action
deu pin exact; release tag la immutable, release da ton tai phai fail thay vi
`--clobber` binary cung version. CI phai administrative-
extract MSI, silent install/uninstall NSIS va mount read-only chinh DMG duoc upload;
sau do doi chieu full runtime/resource/production IPA va chay lai packaged smoke.
Windows con phai tim exact desktop EXE trong ca MSI/NSIS, parse PE machine x64 va
ghi rieng size/SHA-256 moi bundle. Khong ep hai EXE byte-equal: Tauri patch bundle
type metadata khac nhau truoc moi pack. Mac kiem architecture va
`codesign --verify --deep --strict`. DMG verifier phai attach vao exact mountpoint
tam do collector tao va bat dau `finally` ngay sau attach; ke ca plist/mount validation
loi van phai detach. Neu verify va detach cung loi thi giu verify error lam loi chinh
va chain detach error lam cause. `attach` timeout/nonzero van phai query
`hdiutil info -plist` theo exact mountpoint; neu da mount thi detach toi da ba lan,
lan cuoi co `-force`.
Khong ha gate
thanh chi tim thay installer hoac kiem sibling `.app` ngoai DMG.
Production IPA va canonical-LF manifest van byte-contract o section 3.15; pipeline
snapshot truoc build va khong duoc overwrite chung.

Windows local release gate sau CI-fix da PASS: MSI 51,757,692 byte, NSIS
40,995,508 byte;
administrative MSI extract, NSIS silent install/uninstall, full resource/source
tree, frozen ping, tidevice, signer, signing resource va UTF-8 error JSON deu PASS.
Mac CI hien ky ad-hoc (`-`) de co artifact test. Chua co Developer ID/notarization
thi khong goi DMG la ban phat hanh Gatekeeper hoan chinh; nguoi dung co the phai cho
phep lan dau trong Privacy & Security. Them Apple signing/notarization secrets la
phase distribution sau, khong hard-code secret vao workflow. Checkpoint nay chi
PASS source + Windows native package. Run GitHub dau tien cua commit `14fcc48` fail
dung o quality gate do quality chi cai runtime requirements (thieu module build
`packaging`) va WDA lock cu bam CRLF working tree; khong rerun commit do. Sau khi
doi quality sang `requirements-build.txt` va lock canonical LF/mode, run commit
`19dafec` da PASS Quality va build/attest sidecar Windows. Hai job Mac dung o exact
closure gate: pip tren ca arm64/x64 co them `apple-compress`, `loguru`, `pexpect`,
`ptyprocess`, `jinxed`, trong khi `av` va `lzfse` chi active tren Windows. Lock da
tach marker `darwin`/`win32` theo tap cai that nay; phai theo doi run sau ban sua
marker va chi tuyen bo CI xanh khi Windows MSI/NSIS cung hai Mac DMG deu PASS.
Run `9fd58e9` da PASS Quality va Mac arm64 build xong PyInstaller runtime, nhung
frozen `build_and_install.py --self-test` exit 1; wrapper cu che stdout JSON nen chua
co failure cu the. `run_checked()` phai giu toi da 2,000 ky tu cuoi cua stdout/stderr
trong CI error; khong quay lai `CalledProcessError` khong co diagnostic.
Run diagnostic `9a64a52` bi chan truoc Mac boi integration
`flow_release_one_fixture` tren Windows runner: local 2.55 giay nhung runner tai cao
mat 11.87 giay, trong khi moi SQLite read co the block toi busy-timeout 5 giay.
Deadline cua fixture nay la 30 giay; day chi la gioi han test, khong doi timeout
product/evidence va khong duoc ha lai 5 giay.
Run `55fd06e` da PASS Quality gate; Mac arm64 sau do lo dung loi integrity cua
legacy WDA source: `Path` native sort case-insensitive tren Windows nhung
case-sensitive tren POSIX, nen cung mot tree tao hai digest. Moi source attestation
phai sap xep theo relative POSIX path ma hoa UTF-8 byte, khong dung thu tu mac dinh
cua `Path`. Digest portable cua WDA source 16.0.0 la
`74acd24fdbde2fd5ad2b73d4956217900e23461b01cf8100b2ef8cccb37cc4a0`;
khong tai sinh lock bang native path ordering.
Run `c918d25` da PASS Quality va macOS arm64 native package gate; macOS x64 build
xong PyInstaller nhung ping tra `contracts=[]`. Log pip cho thay cryptography
49.0.0 khong co Intel macOS wheel, nen runner build sdist lien ket dong toi
`/usr/local/opt/openssl@3`; DVT import trong frozen runtime vi the khong san sang.
Trong khi con phat hanh macOS x64, lock cryptography o 48.0.0 (universal2 wheel),
CI phai chay trong venv rieng va dung `--only-binary=cryptography`; khong cho pip
am tham build crypto source theo thu vien cua host. Contract smoke bat diagnostic
chi qua `RIVIU_SIDECAR_CONTRACT_DIAGNOSTICS=1`, gioi han 1,000 ky tu moi exception
va 2,000 ky tu khi dua vao build error; ping product mac dinh khong lo diagnostic.
Moi native artifact manifest schema 2 phai ghi exact `sourceCommit` va release gate
bat buoc ba manifest cung khop `GITHUB_SHA`. Ngay truoc `gh release create`, workflow
phai peel lai remote tag (ho tro ca lightweight/annotated) va doi chieu voi checkout
commit; `--verify-tag` mot minh khong khoa artifact vao commit da build.

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
- `nurture/actions.rs`: lấy contact sheet ba frame và chạy grounded generate +
  independent verify trước khi mở UI; nếu semantic gate không đạt thì bỏ lượt,
  không dùng pool comment chung;
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

### 9. Context-grounded comment (04/08/2026)

- Comment chữ production phải lấy bằng chứng từ **ba frame MJPEG liên tiếp** của
  cùng màn hình Feed. Frame được ghép thành contact sheet portrait, kèm crop phóng
  vùng caption; không dùng `GET /screenshot` và không lấy caption từ OCR UI riêng.
- Mỗi lần comment chạy hai lượt AI: `grounded_generate` đọc caption/visual facts và
  tạo một câu; `grounded_verify` đọc lại frame độc lập để chấm relevance,
  evidenceSupport, instructionFit và genericity. Nội dung/caption luôn thắng
  direction giọng điệu; câu đạt phải ngắn, khẩu ngữ như phản ứng vừa xem xong,
  không mang giọng báo cáo/tóm tắt. Marker kiểu `được trình bày`, `mang đến`,
  `người xem`, `chất lượng` bị coi là formal-style và phải retry/skip.
- Chỉ nhận khi overall >= 80, instructionFit >= 70, genericity <= 30 và không có
  contradiction/unsupportedClaim/uiTextConfusion. Một lần retry chỉ dành cho lỗi
  điểm mềm; API lỗi, JSON sai, frame không phải Feed hoặc bằng chứng mơ hồ đều
  `ContextSkipped` và **không** dùng pool comment chung.
- Mỗi attempt grounded, kể cả lượt bị skip trước UI, được ghi vào
  `nurture_comment_attempts`; attempt qua gate có caption preview, frame SHA-256,
  điểm kiểm chứng, token/cost và outcome (`sent`, `text_not_armed`,
  `text_uncertain`, `context_skipped`, ...). Cost row chỉ được ghi sau xác nhận
  nút Gửi đã tắt; HTTP ACK không phải bằng chứng gửi thành công.
- `generate_comment_pool` và pool fixture chỉ còn để tương thích test cũ; không
  được gọi từ production `NurtureEngine`. Thay đổi schema phải kèm migration,
  rollback test và cập nhật command `nurture_list_comment_attempts` nếu UI cần
  hiển thị lịch sử.

### 10. Interaction Campaign implementation checkpoint (04/08/2026)

- `crates/core/src/interaction.rs` hiện có parser URL TikTok video/photo trực tiếp,
  reject typed cho host/scheme/path/short-link, planner root rotation theo
  `(target_index + ordinal) % actor_count`, chain parent và hash exact text trước
  UI. Short link vẫn phải resolve qua bước identity Copy Link trước khi được
  phép chạy; parser không tự coi URL rút gọn là target hợp lệ.
- Migration `interaction-comment-threads` là **version 4** trong ledger chung,
  không tạo ledger riêng. SQLite lưu campaign/actor/target/assignment, prepared
  text, effect intent, evidence, retry/cancel projection và artifact locator.
  `Database` có create/list/get/request/prepare/state/artifact APIs; test rollback
  migration và test persistence đều phải giữ.
- Tauri đã đăng ký `interaction_parse_links`, preview/start/list/get/cancel/retry
  và `interaction_open_on_device`. React có nút `Tương tác` cạnh `Nuôi TT`, panel
  Setup/Monitor, multiline direct link, actor 2-6, message 2-6, instruction và
  max words. Run Now persist trước khi spawn worker; không có scheduler phase này.
- Worker dùng `DeviceWorkOwner::Interaction` và thứ tự session -> MJPEG -> open URL;
  từng target chuẩn bị toàn bộ text qua grounded AI rồi persist hash trước send.
  Root sender xác nhận drawer/type/Send armed/Send cleared bằng frame. Sau root,
  Vision OCR revision 3 phải thấy author + exact normalized text trên hai frame;
  reply chỉ tap nút `Reply` khi locator khớp hai frame, nếu không assignment là
  `skippedParent`/partial. Sau effect intent, lỗi send là `uncertain` và retry bị
  chặn; không báo thành công theo HTTP ACK.
- Gate live Mac 04/08: candidate mới build/sign được với
  `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`, nhưng DVT launch
  `com.riviu.managersphone.agent.xctrunner` trả deviceprocesscontrol code 2 trên
  iPhone 8 iOS 16.7.16; Gate B/C = `FAIL`, report tại
  `docs/re/riviu-agent/interaction-gate-live-e561.json`. Supplemental reuse cũng
  chỉ là `SUPPLEMENTAL_ONLY`. Vì vậy production
  `sidecars/wda/interaction-capabilities.json` vẫn giữ `qualifications: []`,
  không promote candidate và không gọi Interaction Ready trên desktop.
- Hash production text/manifest/capability sau vòng test vẫn phải khớp lần lượt
  `45b98dda18ad403b2fdeb547e239a3594506944e1235d8e99345cd7450158389`,
  `562578b1740a1e4ae13c863b28e6f72c448c3be80bfb3906b9d8342595850e73` và
  `f2e75b2c71dda557de6ec21f64f49b7ab0c8bb3bfe0bbccb5e64ab59be2c9709`.
- Verification roles, scoped diff và rollback artifact nằm ở
  `docs/verification/interaction-thread-20260804/`; lỗi trước khi ghi effect
  intent là `Failed` và có thể retry, còn lỗi sau `Sending` là `Uncertain` và
  không dispatch lại.

### 11. API comment preview (05/08/2026)

- Popup `Nuôi TT` có nút `Test API` trong `Cấu hình AI`. Nút lưu và validate đúng
  cấu hình hiện tại, lấy tối đa ba frame MJPEG của máy đã chọn, rồi gọi
  `nurture_test_api` dùng chính `prepare_grounded_comment` của production khi
  provider nhận ảnh. DeepSeek V4 public endpoint là text-only nên nhánh này OCR
  caption cục bộ và gọi `prepare_caption_comment` hai lượt text JSON; kết quả ghi
  rõ `OCR caption + text`. Lệnh chỉ trả preview comment, caption, điểm bằng
  chứng, token, cost, model/host và SHA-256 evidence; không mở composer, không tap
  Send, không ghi comment lên TikTok.
- Test phải chạy khi máy đang ở một video/photo TikTok có frame stream mới. Home,
  profile, LIVE hoặc frame không đủ bằng chứng sẽ trả lỗi/context rejection thay
  vì tạo comment chung chung. Không coi HTTP/API thành công là comment đã gửi.
- Model và Base URL vẫn là cấu hình người dùng; không hiển thị dòng gợi ý model
  cố định trong popup. Cấu hình hiện tại của DB dùng endpoint DeepSeek và model
  DeepSeek đã lưu, không được ghi đè khi build/cài lại app.
- **Đã đo 09/08/2026** — `GET api.deepseek.com/models` trả `deepseek-v4-flash` và
  `deepseek-v4-pro`; gửi content part `image_url` tới **cả hai** đều trả 400
  `unknown variant "image_url", expected "text"`. Serde chỉ liệt kê đúng một
  biến thể, tức content-part enum của endpoint **không có** case ảnh: giới hạn
  nằm ở request schema của endpoint chứ không phải khả năng của model, nên không
  model string nào đi vòng qua được. `provider_supports_vision` khoá theo host là
  vì thế. Đo lại trước khi tin — DeepSeek thêm image part thì cờ này sai âm thầm.
- Nurture (`nurture_test_api`) và chiến dịch chuỗi bình luận dùng **chung**
  `prepare_comment_for_frames`. Trước đây mỗi bên tự viết nhánh và bên chuỗi
  không viết: nó gọi `prepare_grounded_comment` vô điều kiện nên provider
  text-only làm hỏng cả campaign. Provider text-only là đường bằng chứng yếu hơn
  (OCR caption + gate `accepts_caption`), **không phải** lý do từ chối — đừng
  thêm lại cổng chặn ở `interaction_start_thread`.
- Gate text-only dùng context OCR >= 60 và relevance/evidenceSupport >= 80,
  kèm các cờ contradiction/unsupportedClaim/uiTextConfusion và formal-style như
  gate vision. Không hạ gate vision; hai mode phải hiển thị rõ nguồn evidence.
- Artifact và lệnh kiểm chứng/rollback của preview nằm ở
  `docs/verification/api-test-20260805/`.

### 12. Stream preview scaling (05/08/2026)

- `StreamHub` giữ một fleet feed cho desktop scheduler và channel riêng theo
  UDID cho `FrameSource`/popup watcher. Không quay lại kiểu mỗi watcher đọc
  broadcast toàn fleet: 100 máy sẽ tạo fan-out O(n²) và làm stream bị giật dù
  từng MJPEG vẫn còn sống.
- Desktop preview giữ latest frame theo UDID, round-robin và mã hoá base64 tối
  đa 240 frame/giây toàn fleet. Hai máy vẫn nhận tối đa 24 FPS/máy; khi tăng
  fleet, tốc độ preview tự chia đều (20 máy = 12 FPS/máy, 100 máy = 2 FPS/máy)
  và producer đã dừng sẽ rời ngân sách sau 10 giây. Đây chỉ là ngân sách UI;
  stream raw vẫn là nguồn bằng chứng cho watcher và nurture.
- `RIVIU_STREAM_CAPACITY` là cấu hình desktop cho 1..100 producer, mặc định 2
  để giữ hành vi live hiện tại. Giá trị ngoài khoảng bị bỏ qua và ghi cảnh báo,
  không được tự nâng capacity trong code gọi lệnh.
- **Đã sửa 09/08/2026**: `StreamBudgetManager` chặn cứng ở 2, nên
  `RIVIU_STREAM_CAPACITY=3` từng làm app **panic lúc khởi động** qua `.expect()`
  — trái đúng hợp đồng "giá trị ngoài khoảng bị bỏ qua" ở ngay trên. Giờ hạ về
  mặc định kèm cảnh báo (`state.rs::desktop_stream_budget`).
- Dải local WDA-control có 128 slot (`18100..18227`) để không đụng port khi
  fleet xoay vòng tới 100 UDID. Mọi relay vẫn phải nằm trong supervisor lock và
  registry fingerprint; không tạo relay thứ hai cho cùng UDID.
- Khi thêm virtualized grid hoặc focus priority, chỉ thay scheduler preview;
  không nới `snapshotMaxDepth`, không bật `autoDismissAlerts`, không đổi thứ tự
  session-trước-stream và không dùng preview event để chứng minh gesture/comment.

### 13. Standalone Riviu Agent full interaction install (05/08/2026)

- `sidecars/wda/RiviuAgent-text.ipa` build `2` là artifact candidate độc lập cho
  scope tương tác hiện tại: `stream`, `tap`, `swipe`, `clipboard`, `text`.
  Text gate TikTok đã có frame `armed`/`sent` thật và manifest SHA-256 là
  `45b98dda18ad403b2fdeb547e239a3594506944e1235d8e99345cd7450158389`.
- Build `2` đã được upgrade trên cả hai iPhone test; `is-installed` xác nhận
  `com.riviu.managersphone.agent.xctrunner`, version `1.0`, build `2`, đúng
  Apple Development signer. Desktop Full được build với
  `RIVIU_DEFAULT_AGENT_MODE=full` và khởi động không cần biến môi trường.
- Protected runtime hiện dùng `backend=riviu-agent` trên cả hai máy. Sáu mẫu
  `/status` liên tiếp trả `state=ready`, protocol `2`, có `text`; mỗi máy đã
  trả một JPEG MJPEG HTTP `200` hợp lệ qua header `X-Riviu-Token`. Không có
  process/port RT-MMO (`8906`/`9093`) trong phiên này. `RiviuAgent.ipa` và
  `agent-manifest.json` production vẫn chỉ là rollback oracle, không phải
  dependency runtime của Full.
- Bản desktop đã được đóng gói self-contained bằng PyInstaller onedir Python
  3.12.13 với closure khóa `pymobiledevice3==10.1.0` và `tidevice==0.12.11`;
  frozen `ping`, embedded tidevice, signer và signing-resource self-test đều
  PASS. Process thực tế chạy từ
  `Contents/Resources/sidecars/pymobiledevice3/runtime/riviu-pmd`, không cần
  Python/pip/tidevice trên máy người dùng. Executable hash là
  `46b711e1ddf7e133cca945a28dc9a50e4a400214e527e966b7c65ec87f901946`, tree
  hash `56774fce35dc0a20f29e052c86b5cfeda342e274e827b0a978a70a1aea15e0cf`.
- CI release phải truyền `RIVIU_DEFAULT_AGENT_MODE=full` và merge
  `apps/desktop/src-tauri/tauri.full.conf.json` trước
  `target/tauri-sidecar.conf.json`; nếu chỉ dùng config mặc định thì artifact
  sẽ trở về tên desktop cũ và mode legacy. Thư mục `target/` không commit: push
  `main` tạo artifact Windows/MSI/NSIS trong Actions, còn tag `v*` tạo Release.
- Verification record, desktop preview capture, IPA rollback và desktop
  pre-sidecar rollback nằm trong
  `docs/verification/standalone-agent-full-20260805/`.
- `RiviuAgent-text.ipa` hiện đã deep-verify chữ ký và embedded profile có đúng
  hai UDID test, `CreationDate=2026-08-03` và `ExpirationDate=2026-08-10`;
  đây là Xcode-managed/free provisioning 7 ngày. Windows desktop installer
  không có hạn này, nhưng Agent trên iPhone sẽ cần IPA ký lại sau ngày hết hạn
  hoặc khi đổi UDID. Không gọi IPA này là universal artifact cho thiết bị mới.
- Candidate v2 chưa quảng bá `pushMedia`; capability này chỉ được thêm sau khi
  có route contract và read-back test riêng theo source-reconstruction design.
  Không gọi bản candidate hiện tại là parity đầy đủ với oracle RT-MMO cho tới
  khi gate đó hoàn tất. Verification và rollback của lần cài này nằm ở
  `docs/verification/standalone-agent-full-20260805/`.

### 14. Photo carousel publish campaign (05/08/2026)

- Input publish là một thư mục một cấp: mỗi thư mục con là một carousel image,
  ảnh phải có tiền tố số liên tiếp bắt đầu từ `01`, có đúng một `caption*.txt`;
  `partners*.xlsx`, file ẩn và file không nhận diện bị bỏ qua có notice. Parser
  chỉ đọc PNG/JPG/JPEG (HEIC chưa được decoder hỗ trợ), giữ caption UTF-8 sau
  chuẩn hoá newline và tính SHA-256 từng ảnh/caption. Không tự sửa caption bị
  cắt hoặc tự thêm hashtag.
- `crates/core/src/publish.rs` tạo manifest side-effect-free; copy sang
  `artifacts/publish/<request-id>/<bundle-id>` được verify lại hash trước khi
  ghi DB. Mapping là một-một theo thứ tự bundle đã chọn và UDID đã chọn, cấm
  trùng/thiếu. Visibility hiện cố định `Public`, âm thanh TikTok mặc định,
  cleanup chỉ được phép sau bằng chứng post thành công.
- Migration 5 (`publish-campaigns`) lưu request, manifest bundle, assignment,
  dispatch lease và event revision. Tauri commands mới là
  `publish_scan_folder`, `publish_create_campaign`, `publish_list`,
  `publish_get`, `publish_prepare`, `publish_transfer`, `publish_cancel`;
  `publish_prepare` chỉ chuyển sang `ready`, không giả nhận đã đăng.
- `publish_transfer` và `push_material` không được gọi `install_app` cho media.
  Chúng giữ device lease rồi gọi sidecar `media-stage`, đẩy ảnh/caption qua
  HouseArrest/AFC vào `Documents/Riviu/Publish/<campaign-id>`, ghi manifest và
  đọc lại size + SHA-256. Candidate media route sau đó gọi protected native
  `prepare` rồi `import`: Photos tạo album `Riviu-<import-id>` theo đúng thứ tự
  ảnh và lưu asset IDs để cleanup idempotent. Lỗi stage/native import phải ghi
  `uncertain`, không để assignment kẹt ở `transferring` và không tự đăng lại.
- `sidecars/wda/riviu-agent/Contracts/media-v1.json` nay là candidate-route cho
  native `pushMedia`: patch 0006 thêm protected `POST/GET
  /riviu/media/v1/prepare`, kiểm tra campaign/schema, path containment, size và
  SHA-256 readback. `build_candidate.py --media-capable` và probe truyền cờ
  runtime một cách opt-in; production/default candidate vẫn không advertise
  feature này cho tới khi gate TikTok import, post-frame evidence và cleanup
  verification hoàn tất.
- UI Publish hiện cho chọn thư mục, subset bundle, subset phone, hiển thị mapping
  tuần tự/caption, chạy ngay hoặc lịch một lần. Assignment `imported` có nút
  `Post`; `publish_post` mở fresh TikTok session, stream MJPEG, chọn album, chọn
  đủ ảnh, nhập caption Unicode, xác nhận modal Public và chỉ ghi `succeeded`
  khi frame sau đăng thay đổi. Scheduler đến hạn chạy transfer rồi post tự động;
  lỗi sau effect intent là `uncertain`. Test đã pass: core parser/DB campaign,
  TypeScript/Vite build, Python media manifest, candidate contract.
- Bản Full arm64 đã build/cài tại `/Applications/Riviumanagersphone Full.app`,
  `codesign --verify --deep --strict` PASS. Candidate `0.5.2-media-text` (build
  `8`, source SHA
  `6055167f6cc2bab55147839bb21d028328554660568c7884d68fc93154443e03`) quảng bá
  đúng `stream/tap/swipe/clipboard/text/pushMedia`; resource sidecar frozen có
  `pymobiledevice3 10.1.0`. Live e561 đã PASS stage + native import (8 ảnh,
  1 caption) và đã chạy tới TikTok composer/post flow. iOS yêu cầu xác nhận khi
  xoá album Photos; patch 0007 chuyển cleanup sang `performChanges` async, bơm
  run loop và tự bấm nút `Xóa/Delete`, còn desktop cleanup chạy trước khi đóng
  stream và có một lần retry. Build/install đã PASS; Gate post+cleanup cuối vẫn
  chờ e561 được mở khóa lại sau reboot để chạy lại live. Production/default IPA
  vẫn giữ nguyên; không promote candidate trước khi record mới có frame post và
  cleanup `state=cleaned`.

#### 14.1 Live checkpoint 06/08/2026

- Native media permission đã có retry bằng XCTest pointer event. Patch `0011`
  fallback `wdFrame` và patch `0012` ưu tiên `UIScreen.mainScreen.bounds`, sau
  đó dùng fixture `375x667` nếu UIKit chưa trả frame. Cleanup giữ retry native
  bốn lần và fallback frame từ patch `0010`.
- Baseline lock hiện có 12 patch; output source SHA-256 là
  `f219ee8e356dc68119ee763059803934f80caaa275eda07ba8f42ea7bdb4f9a9`.
  Candidate build `0.5.7-media-text`/build `13`, IPA SHA-256
  `feeaa11cc68d9ab040e3a4326c5d4a52d0de037fb820c7406a28fa65f712708d`,
  source/contract/objective-C unit tests đều PASS và feature list gồm
  `stream/tap/swipe/clipboard/text/pushMedia`.
- Full app được build từ `apps/desktop/src-tauri` với cả hai config full và
  sidecar overlay; executable SHA-256 hiện tại là
  `663d03a2a48363115e65f345fafc2e4eea4785428ee79d9facb4059d36cd5a53` và
  `codesign --verify --deep --strict` PASS. Production
  `sidecars/wda/RiviuAgent.ipa` không bị thay thế.
- Live campaign `49496e40-9642-42fa-a44b-949edb5ecc24` và
  `723cc89d-36f4-4b72-8b33-d686ef296d3e` đã xác nhận stage/readback, nhưng
  import e561 timeout ở popup Photos nên state là `uncertain`; không gọi đây là
  `imported`. Cần trust lại IPA build 13 trên thiết bị trước khi chạy lại
  transfer/post/cleanup và ghi frame evidence.
- Test xác nhận: `cargo test -p riviu-core --lib publish` 9/9,
  `cargo check -p riviu-managers-phone` PASS (chỉ dead-code warning), Python
  `unittest discover sidecars/wda/riviu-agent/Tests` 125/125. Hai assertion
  patch-count đã đổi sang đọc số patch từ `baseline-lock.json` để không vỡ khi
  thêm patch native.

#### 14.2 Live verifier checkpoint 06/08/2026

- Candidate 0.5.7/build 13 đã chạy thật trên e561. Photos permission không tự
  đóng trước deadline; manual native tap `(187,407)` đóng được popup, sau đó
  phải bỏ qua alert `iPhone chưa được Kích hoạt`. Campaign
  `521e1510-ba54-4bdf-9e57-73384cbe2468` giữ `uncertain/media_transfer_native_failed`.
- Với quyền Photos đã được cấp, campaign
  `94389eb4-68a5-416c-816c-e47e2e0ee3b0` đạt `imported` (8 ảnh), Post flow rời
  composer và cleanup trả `state=cleaned` cho 8 asset. Frame sau Post lại hiện
  popup `Trạng thái tài khoản / Tài khoản của bạn đã bị khóa.`; record đã được
  sửa transactionally thành `uncertain/post_account_locked`, assignment có
  `effectIntent=post_carousel`, frame `/tmp/e561-post-success.png`, event
  `verification_failed` revision 7. Không gọi đây là post thành công.
- `publish_commands.rs` nay chạy Vision OCR ở frame sau Post và frame chờ tiếp
  theo, chặn cả chuỗi tiếng Việt/không dấu và English `account locked`. Desktop
  crate test 47/47, core publish 9/9, Python candidate 125/125, fmt/check PASS.
- Baseline lock có 13 patch; patch mới
  `0013-media-permission-logical-tap-fallback.patch` SHA-256
  `31567ca568c71550b130bb8054e647c83fe9453ea7a154f43c1561ea45bd1831` kéo dài
  16 lần tap native và dùng logical `(187.5,407)` nếu UIKit báo bounds vật lý
  2x. Source SHA-256 mới là
  `4c7465251a31469c5b90edfb56defa988f7f80f69b1278c3027366722304d915`.
- Candidate `0.5.8-media-text`/build `14`, IPA SHA-256
  `e86e77abe14d7190090b19e8e88c2a9b14417caac5ec18c604ab4ebb9a2e7d51`, features
  `stream/tap/swipe/clipboard/text/pushMedia`, Objective-C unit tests PASS. Build
  dùng a99 vì e561 đã rớt khỏi danh sách Xcode; gate live vẫn `PENDING_MAC_DEVICE`.
- Full app mới tại `/Applications/Riviumanagersphone Full.app`, executable SHA-256
  `d4a033b259a43debd4dd1fb02ca2b778822509834afe1184c73102958f42ba1b`,
  `codesign --verify --deep --strict` PASS. Production
  `sidecars/wda/RiviuAgent.ipa`/manifest không bị thay thế. Candidate 14 cần một
  vòng cài/trust e561 mới để xác nhận automatic Photos permission; không ghi
  PASS trước vòng đó.

#### 14.3 Gate B/C a99 checkpoint 06/08/2026

- Candidate media-only `0.5.8-media`/build `15` được build trên a99 với patch
  0013; feature set đúng contract gate là
  `stream/tap/swipe/clipboard/pushMedia`, IPA SHA-256
  `5f085ee785b77c7bd3050592212c38a5dcc438a77930dde34c0203b0ec8d3420`, manifest
  SHA-256 `083263e4101b986d23d40790fd6816deca17d877fc0acfc2e542ff01926b25bf`,
  source SHA-256 vẫn `4c7465251a31469c5b90edfb56defa988f7f80f69b1278c3027366722304d915`.
- Probe fresh report `docs/re/riviu-agent/candidate-probes-a99-20260806-media-fresh2.json`
  xác nhận `candidateFreshInstalled=true`, identity và cleanup đều pass, nhưng
  cold launch bị iOS từ chối với `Security ... profile has not been explicitly
  trusted by the user`; Gate B/C là `FAIL`. Đây là trust của profile sau
  uninstall/fresh-install, không phải HTTP/auth/manifest failure. Settings trên
  a99 đang mở popup `Nhà phát triển Không đáng tin cậy` để user xác nhận profile.
- Sau reboot, `tidevice developer -r` đã mount Developer Support. Runner text cũ
  chỉ launch được khi đã trusted; media candidate cũng báo `Test runner ready`
  khi được upgrade từ bản trusted. Không gọi supplemental reuse là Gate PASS;
  live Gate B/C chính thức vẫn chờ user trust candidate mới rồi chạy lại fresh
  probe với ngưỡng cố định.
- Các report supplemental/fresh fail đều được giữ lại và qua
  `rtmmo-re verify-redaction`; production IPA/manifest và app Full không bị
  thay đổi bởi gate probe.

#### 14.4 Human-like nurture checkpoint 06/08/2026

- Guard nhịp cũ đã được gỡ khỏi `NurtureSettings`, Tauri validation và popup.
  UI không còn mục `Nhịp an toàn`; cấu hình người dùng chỉ giữ xác suất và
  thời lượng xem. Không thêm lại các trường `risk_*`/`RiskGuard`.
- `crates/core/src/human_behavior.rs::HumanSessionPolicy` là policy nội bộ,
  luôn bật: cap rolling ngẫu nhiên theo giờ (tim/bình luận/follow), khoảng
  cách 12..35 giây, tối đa 2 bài đã tương tác trong 5 bài gần nhất, micro-rest
  7..13 video, block 20..45 phút, nghỉ Home 60..240 giây, Home ngẫu nhiên và
  cold restart rất hiếm (tối đa một lần mỗi phiên). Attempt được ghi trước
  gesture; counter thành công chỉ tăng khi frame sau xác nhận.
- Engine lấy action rail mới trên từng frame, không dùng rail cũ. `FeedCardKind`
  phân biệt video, `PhotoCarousel` (vuốt ngang 1..3 ảnh), `LivePreview` (vào
  phòng theo xác suất, dwell rồi thoát hoặc vuốt qua) và transition. Watcher
  tạm nhường `LiveRoom` khi engine đang sở hữu phòng để không tự đóng nhầm.
- Production DeepSeek text-only đi qua `FrameTextSource` của desktop, OCR
  caption rồi `prepare_caption_comment`; provider vision vẫn dùng 3-frame
  grounded path. Default mới là `https://api.deepseek.com`/
  `deepseek-v4-flash`. Windows adapter hiện báo thiếu Vision OCR thay vì giả
  nhận diện.
- Harness headless gọi preflight install/auth bằng context `Repair`, thả
  context trước khi chạy nurture, và dùng token env trực tiếp để tránh Keychain
  prompt. Trình tự live xác nhận: relay/auth -> session -> stream -> foreground.
- Verification: `cargo test -q -p riviu-core --lib` 299 pass/1 ignored,
  `cargo test -q -p riviu-core --test real_frames` 15 pass,
  `cargo test -q -p riviu-managers-phone` 49 pass, frontend `npm run build`
  PASS, `codesign --verify --deep --strict` PASS. Full executable hiện tại có
  SHA-256 `e4da1fb730ad7fcb4cf82b750c85ed05f5b3bcf743f6ab4a427c4d81ec9e53e2`.
- Installed app là `/Applications/Riviumanagersphone Full.app`; rollback copy
  được giữ tại `/Applications/Riviumanagersphone Full.app.rollback-20260806-human-v2`
  với hash baseline `335c35fcb79af920e0714b2f96d20ffeb250100ef361628f8ff798252d1ef68a`.
  Không overwrite production IPA/manifest trong `sidecars/wda/`.
- Live smoke pass trên a99 (1 phút): session create/prime pass, stream có frame,
  6 video, popup đóng 1 lần, nhận diện LIVE preview và bài ảnh, 0 recovery nặng.
  Một lượt sau gặp màn không phải FYP và kết thúc `0 video`; giữ cả hai log,
  không chuyển lượt fail thành pass. Chi tiết nằm ở
  `docs/verification/nurture-human-v2-20260806/`.
- Review default 06/08/2026: `HumanSessionPolicy` giữ một ngưỡng nghỉ cố định
  7..13 video rồi mới bốc ngưỡng tiếp theo; trước đây nó bốc lại ở từng video
  nên cadence không ổn định. `frenzy_prob` giờ được nối vào các swipe feed
  bình thường (retry sau swipe kẹt vẫn dùng tốc độ thường) và có ô chỉnh trong
  popup. Default fresh install là like `35%`, comment `0%` (comment chỉ bật sau
  khi có API key), follow `3%`, vuốt nhanh `6%`, xem `3..18s`; lịch vẫn tắt,
  nếu bật dùng chu kỳ `240 phút`/block `150 phút`. Setting đã lưu không bị
  migrate/ghi đè.
- Validation mới chặn `num_videos` > 10000, `num_rounds` > 100, watch > 120s,
  lịch ngoài `15..1440` phút hoặc block ngoài `15..360` phút; engine dùng
  `saturating_mul` cho legacy fixture. Tests sau review: core `299 pass/1
  ignored`, Tauri `49 pass`, frontend `73 pass`; Full app rebuild hash
  `e4da1fb730ad7fcb4cf82b750c85ed05f5b3bcf743f6ab4a427c4d81ec9e53e2`, harness
  hash `681ffe53517fb1244791778c177091ff8baf0d33389c9167bec309e29f6246df`,
  codesign strict PASS. Live smoke cũ vẫn là bằng chứng hành vi thiết bị; chưa
  gán nó thành pass mới cho thay đổi default.
- Touch/speed review 06/08/2026: `crates/core/src/nurture/touch.rs` giữ lịch sử
  tọa độ theo UDID và session, lượng tử hóa về lưới logical nguyên, không trả
  lại điểm đã dùng và tránh điểm gần nhau trong 96 lần gần nhất. Planner được
  dùng cho rail, LIVE, comment drawer/composer, emoji, thread reply và send;
  watcher popup vẫn giữ điểm đóng cố định để không miss hộp thoại hệ thống.
  Swipe feed dùng mixture nhanh hiếm `190..280ms`, bình thường `300..520ms`,
  chậm `520..820ms`; cờ frenzy dùng `150..240ms`, retry swipe kẹt không frenzy.
  Carousel dùng `280..420ms` nhanh hoặc `420..760ms` thường. Không gọi đây là
  bất biến vô hạn: vùng hitbox hữu hạn; planner có fallback mở rộng và fail
  closed khi toàn bộ logical screen đã cạn điểm.
- Final closure 06/08/2026: legacy nurture settings được migrate một lần với
  marker `nurture.settings.migration.v2`, DB backup và `rollback-db.sh`; candidate
  Riviu Agent mở URL bằng `/url` khi capability report không có route riêng,
  desktop inject OCR caption thật và text-only comment retry sau verifier. Live
  target-photo run `live-comment-target-open-url-v6.jsonl` PASS: 3 video, 2
  comment có frame xác nhận, 0 recovery. Không quảng bá comment khi evidence gate
  fail; stock/RT-MMO vẫn giữ fail-closed contract.

## 9. Fleet Android (09/08/2026)

Ổ cắm cho việc này được chừa từ ngày đầu — bản thiết kế gốc
(`docs/superpowers/specs/2026-07-25-riviu-managers-phone-design.md:7`) viết
*"…multiple iPhones, with Android deferred behind a `DeviceDriver` trait"*.
`crates/android-driver` lấp chỗ đó và **không phải sửa `DeviceDriver`/`UiSession`**.

Số đo đầy đủ ở `docs/ANDROID_PROBE_REPORT_2026-08-09.md`. Những điều không được
đoán lại:

- **Hai tầng, chia theo số đo, không theo thẩm mỹ.** `adb.rs` chỉ dùng cho vòng
  đời (cài/mở/dừng/khởi động lại): mỗi lệnh tốn 1–2 giây trên fleet Galaxy S8+.
  `agent.rs` nói HTTP với `appium-uiautomator2-server` thường trú qua
  `adb forward`: click 130–280 ms, tìm element 609 ms, đọc thuộc tính 241 ms.
- **Đừng dump cả cây accessibility trong vòng điều khiển.** Agent **không** làm
  nó rẻ hơn (3403 ms so với 2693–4239 ms qua CLI) vì chi phí nằm ở duyệt và
  serialize cây, không ở khởi động công cụ. Hãy truy vấn đúng element cần — đó
  đã là hình dạng của `find_and_tap`/`assert_visible`/`read_text`.
- **Locator ưu tiên `content-desc`, KHÔNG phải `resource-id`.** `resource-id`
  của TikTok bị R8 obfuscate (`a1p`, `ty9`, `ebz`) và đổi theo bản build;
  `content-desc` thì ngữ nghĩa và **mã hoá cả trạng thái**:
  `Like` ⇄ `Video liked`, `Read or add comments. 15 comments`.

  > **SỬA 10/08/2026 — `content-desc` KHÔNG phải tiếng Anh bất kể ngôn ngữ UI.**
  > Câu đó đo trên bản TikTok *global* (`com.zhiliaoapp.musically`). Trên bản SEA
  > `com.ss.android.ugc.trill` với UI tiếng Việt (Redmi Note 12, Android 15),
  > dump hierarchy thật cho thấy nhãn **bị dịch**:
  >
  > | AGENTS.md ghi | Thực tế trên `trill` + tiếng Việt |
  > |---|---|
  > | `Like` | `Thích` và `Thích video. 1Tr lượt thích` |
  > | `Video liked` | *(chưa đo trạng thái đã-thích)* |
  > | `Read or add comments. N comments` | `Đọc hoặc viết bình luận. 21,1K bình luận` |
  > | `For You` | `Đề xuất` |
  > | `Tap to watch LIVE` | *(không có nhãn nào chứa)* |
  > | `Follow <name>` | `Follow Hoàng Sơn` — **chỉ cái này giữ tiếng Anh** |
  >
  > Nhãn khác đo được: `Chia sẻ video. 310,6K lượt chia sẻ`, `Thêm hoặc xóa video
  > này khỏi mục Yêu thích.`, `Chú thích`, `Hồ sơ <tên>`, `Bạn bè`, `Đã follow`,
  > `Tìm kiếm`, `Trang chủ`, `Cửa hàng`, `Quay`, `Hộp thư`, `Hồ sơ`.
  >
  > **Hệ quả:** mọi `Locator::Description` tiếng Anh trong nurture/interaction sẽ
  > **absent** trên máy UI tiếng Việt — G1 probe đo được `find("Like")` absent,
  > `find("Video liked")` absent, `assert_visible("For You")` fail, dù rail vẫn
  > hiển thị đầy đủ trên màn hình. Cách mã hoá trạng thái trong nhãn vẫn đúng,
  > chỉ là **theo locale**. Muốn chạy fleet trộn locale thì locator phải tra theo
  > (TikTok package × ngôn ngữ UI), không hard-code một chuỗi. `package.json` của
  > GenFarmer có cột `apps.locale_input` — họ gặp đúng vấn đề này.
  >
  > **Đã có catalog: `riviu_core::tiktok_labels`.** Nhãn là **dữ liệu đo**, khoá theo
  > `(package × ngôn ngữ UI)`, và `labels_for()` trả `None` cho cặp chưa đo — `None`
  > **phải nghĩa là từ chối**, đúng như `CALIBRATED_LAYOUTS` (§10). Đừng thay bằng
  > nhãn của ngôn ngữ khác: locator đó khớp rỗng và đọc thành "chưa thích", tức một
  > câu trả lời **sai**, không phải câu trả lời thiếu.
  > - Mỗi nhãn mang `Exact` hoặc `Contains` như đo được. `Contains` không phải cho
  >   tiện: nhãn bình luận nhúng số đếm — đo được `21,1K bình luận` rồi `697 bình
  >   luận` trên hai video khác nhau, nên exact **không bao giờ** khớp.
  > - Cái gì chưa đo thì để `None`, **không đoán**. Hiện `Liked` và `LiveRoom` của
  >   bản `vi` là `None`: probe thấy `Thích` present và `Đã thích` absent trên video
  >   *chưa* thích — điều đó vừa hợp với nhãn đúng vừa hợp với nhãn sai, nên chưa
  >   chứng minh được gì. Muốn chốt phải đo trên video **đã** thích.
  > - **Đọc ngôn ngữ UI từ `persist.sys.locale`, KHÔNG phải `ro.product.locale`.**
  >   Đo trên Redmi Note 12: `persist.sys.locale=vi-VN` còn
  >   `ro.product.locale=en-GB` (mặc định xuất xưởng). Đọc sai cái là chọn nhãn tiếng
  >   Anh cho máy UI tiếng Việt — **bẫy dòng-đầu thứ ba** cùng loại với `wm size` và
  >   `mCurrentFocus`. Đi qua `AndroidUiSession::ui_locale()` / `adb::parse_locale`.
  > - Verify trên máy thật: catalog tự chọn `vi`, rồi `Đề xuất`/`Thích`/`bình
  >   luận`/`Chia sẻ video`/`Yêu thích` đều PRESENT trong 135–185 ms.
  Vì thế `supports_accessibility_readback` ở đây là `true` — backend đầu tiên
  của dự án nói được câu đó, do iOS buộc giữ `snapshotMaxDepth = 1` (§2.3).
- **Không port `screen.rs` sang Android.** Nó là cách lách một API hỏng; Android
  không có lỗi đó.
- **Không cần IME riêng.** `ACTION_SET_TEXT` gõ tiếng Việt đủ dấu vào ô bình
  luận TikTok (xác minh bằng ảnh chụp), sau đó TikTok tự chuyển nút gửi sang đỏ.
  `adb shell input text` thì **giết tiến trình** khi gặp dấu.
- **`wm size` trả HAI dòng**; phải đọc **Override** (`1080x2220`), không phải
  Physical (`1440x2960`). Đọc nhầm là lệch 33% mọi toạ độ.
- **Bẫy hai `EditText`**: khay bình luận mở có một ô ẩn `focused=false` và một ô
  thật `focused=true`. Selector theo `class name` lấy nhầm cái ẩn, set text
  "thành công" ở tầng API mà màn hình trống. Dùng `Locator::focused()`.
- **Bài LIVE trong feed không có rail** like/comment. Vòng nurture phải nhận ra
  và vuốt qua; nhận ra được từ hierarchy.
- `find_and_tap` lấy bounds rồi **chạm thật** trong vùng đó, **không** dùng
  `ACTION_CLICK` của accessibility — click accessibility phân biệt được với
  người và đi vòng qua lớp cử chỉ mà chống phát hiện đang dựa vào.
- `AdbProgram::run` giải mã stdout thành text nên **phá dữ liệu nhị phân**; mọi
  thứ nhị phân (`exec-out screencap -p`) phải qua `run_bytes`/`device_bytes`, và
  cả hai đường screenshot đều kiểm magic PNG.
- Dấu phân cách trong lệnh shell gộp **chỉ được dùng chữ và gạch dưới**: một
  `--8<--` từng bị shell trên máy hiểu `<` là chuyển hướng input.
- `pidof` thoát khác 0 khi tiến trình vắng mặt. Đó là **câu trả lời**, không
  phải lỗi — đi qua `AndroidDriver::pid_of`.
- **Clipboard Android bị platform gác — ĐỪNG implement qua uiautomator2-server.**
  Đo trên Android 15: `POST /session/{id}/appium/device/set_clipboard` **có tồn
  tại** và trả 200 `value:null`, nhưng `get_clipboard` trả **rỗng**. Từ Android 10
  chỉ app đang giữ focus/quyền mới đọc được clipboard, và MIUI log rõ
  `ClipboardServiceI: checkProviderWakePathForClipboard: <pkg> is not a
  activePermissionOwner`. Server appium không có UI nên không bao giờ là chủ quyền
  đó. **Implement `get_clipboard` trên đường này sẽ tạo ra capability báo thành
  công mà trả về không gì** — đúng loại "HTTP 200 không phải bằng chứng" mà §3.9
  cấm. Vì vậy `AndroidUiSession` **cố ý** không implement clipboard, để rơi về
  `unsupported("getClipboard")` của trait.

  Ba đường khả dĩ, chưa chọn: (1) IME — GenFarmer dùng `AdbKeyboard` của openatx
  với broadcast `ADB_KEYBOARD_GET_CLIPBOARD`, vì **IME được phép đọc clipboard**;
  đổi default IME là xâm lấn và để lại dấu. (2) `io.appium.settings` — app helper
  mà Appium Node driver thực sự dùng cho clipboard, cần cài thêm một APK. (3) Với
  §3.12 Copy Link, đọc URL từ UI thay vì clipboard. Đây là **chặn thật cho
  Interaction trên Android**, không phải việc còn thiếu vài dòng code.
- **Pha 5 đã có câu trả lời đo được: minicap bản Java, KHÔNG phải scrcpy.**
  `noarch/minicap.apk` của `@devicefarmer/minicap-prebuilt` chạy qua
  `CLASSPATH=<apk> app_process / io.devicefarmer.minicap.Main`, **không cần cài** nên
  không vướng gate MIUI, và phát **JPEG** đúng contract `Frame` hiện có (không cần
  decoder H.264). Đã implement ở `crates/android-driver/src/frames.rs`; G1 probe đo
  qua chính code đó trên Redmi Note 12/Android 15 khi TikTok phát video,
  `-P 1080x2400@540x1200/0 -Q 70`: **155 frame trong 6,00 s = 25,8 FPS, 43,2
  KB/frame**, `forward tcp:0` đọc lại được port adb cấp (50784), banner
  `real=1080x2400 virtual=540x1200 quirks=2`. Một reader viết bằng PowerShell trên
  cùng socket chỉ đạt 11 FPS — **chênh lệch là do reader, không phải máy**; đừng lấy
  harness chậm làm giới hạn thiết bị.

  **Đã nối vào fleet, không còn là producer rời.** `riviu_core::FrameSink` là seam
  phía publish: `StreamHub` implement nó, `AndroidDriver::set_frame_sink` nhận nó ở
  composition root (`state.rs`), nên frame Android vào **cùng một hub** với iOS và
  giữ nguyên generation/sequence — không dựng hub thứ hai. `ensure_stream` trả
  `auto-stream://{udid}` như iOS. Kết quả trên máy thật: tile Android `● Live`,
  `Tổng quan 2/2`, badge "2 sẵn sàng".
  - Producer publish qua `publish_if_current`; **`false` là tín hiệu dừng, không
    phải lỗi** — stream mới đã sở hữu máy đó, byte còn trong buffer của reader cũ
    không được phép thành bằng chứng cho stream mới.
  - `ensure_stream` gọi lại thì **reuse** feed còn sống cùng generation, không
    restart.
  - **`adb forward tcp:0` cấp port MỚI mỗi lần gọi.** Retry cả forward lẫn connect
    trong một vòng lặp làm leak một port mỗi lần thử — đo được 4 forward mắc cạn
    sau một lần chạy. Forward đúng **một lần**, chỉ retry connect. Và vì teardown
    không bao giờ chắc chắn chạy, `frames::forward` **prune mọi forward cũ tới đúng
    socket của máy đó** trước khi tạo mới; socket mang tên serial nên cái gì còn
    bám vào đó đều là rác của mình. Đã verify: vào với 1 forward cũ, ra với đúng 1.
  minicap **native** thì đã chết trên Android nay (`.so` prebuilt chỉ tới android-30;
  trên SDK 35 lỗi `cannot locate symbol _ZN7android2ui4Size7INVALIDE`).
  minicap chỉ phát khi display đổi — đó là **ưu điểm**, khớp §3.4. `screencap -p` là
  512 ms/frame và `screencap` raw là 990 ms + 10 MB/frame: cả hai **không** làm được
  stream. Chi tiết và cách đo: `docs/re/genfarmer/README.md` §7.
- **`dumpsys window windows` không còn mang `mCurrentFocus` trên Android 15.** Đo
  được rỗng trên Redmi Note 12/HyperOS, trong khi đó lại là lệnh chạy được trên
  fleet S8+ Android 9 — nên `active_app_bundle` hỏi **cả hai** nguồn
  (`dumpsys window windows`, rồi `dumpsys window displays`) và coi việc grep thoát
  khác 0 vì không khớp là **câu trả lời**, không phải lỗi. Chính exit code đó làm
  G1 probe fail với thông báo rỗng. Đừng thay hẳn sang một lệnh.
- **`probe.rs` nhận `RIVIU_TIKTOK_PACKAGE`.** Package TikTok theo vùng: global là
  `com.zhiliaoapp.musically`, SEA là `com.ss.android.ugc.trill`. Máy mang bản kia
  làm probe fail ngay ở `launch_app` (`monkey -p … failed`) nên **không** đo được
  gì phía sau. Mặc định giữ nguyên giá trị cũ.
- **G1 probe đo lại trên Android 15 (10/08/2026, Redmi Note 12):** `list_devices`
  500 ms, `launch_app` 596 ms, `open_session` 2.160 ms, `window_size` 0 ms
  (1080x2400 Override), `active_app_bundle` 297 ms, `screenshot_png` 844 ms
  (571.271 byte), `inspect_app_process` 98 ms. Agent là
  `appium-uiautomator2-server` **10.4.0**, `/status` trả `ready:true`.
- **MIUI/HyperOS chặn cài app qua adb, và không có đường lách từ host.** Trên
  Redmi Note 12 (`ro.miui.ui.version.name=V816`, `OS2.0.207.0.VMTMIXM`,
  SDK 35) cả ba đường đều trả `INSTALL_FAILED_USER_RESTRICTED: Install canceled
  by user`: `adb install`, push rồi `pm install` trong shell máy, và session
  `pm install-create`/`install-write`/`install-commit` (create + stream **thành
  công**, chỉ **commit** bị chặn). Đừng thử lại ba đường này — nó là policy của
  PackageManager áp cho shell UID, không phải lỗi APK. Mở khoá bằng
  **Tuỳ chọn nhà phát triển → "Cài đặt qua USB"** (và trên MIUI thường cần thêm
  **"Gỡ lỗi USB (Cài đặt bảo mật)"**, đòi đăng nhập Mi account). Vì `ensure_agent`
  **không tự cài** agent, máy nào chưa bật cờ này thì Android control dừng ở đúng
  câu lỗi "the agent is not installed on {serial}".
- **Retry adb là opt-in, không phải mặc định.** `run_bytes` vẫn một phát;
  `run_bytes_idempotent` mới được retry. Lý do: `pm install`, `am start`,
  `am force-stop` và `input` không idempotent, nên retry sau một lần đã landing
  thật sẽ cài hai lần / mở hai lần mà **cả hai lần đều "thành công"** — lỗi vô
  hình. Đừng gộp retry vào `run_bytes` cho gọn.
- **`classify_fault` coi lỗi lạ là terminal, không phải transient.** Máy chưa bấm
  Allow (`Unauthorized`) fail y như vậy mãi mãi; retry nó chỉ làm chậm đúng thông
  báo operator cần. Chỉ `Transient` và `Timeout` được retry.
- **"adb còn sống" = danh sách device ổn định qua hai lần đọc**, không phải một
  lệnh trả về (`devices_stable`). Quan trọng vì `DeviceRegistry::upsert_many`
  thay cả vector: một snapshot xấu không chỉ sai, nó **xoá máy khỏi fleet**.
  `list_devices` hiện chỉ log khi không ổn định — registry giữ lại vector cũ khi
  lần đọc không đáng tin là việc **chưa làm**.
- **`adb kill-server` không bao giờ được gọi tự động.** Nó là hành động toàn cục:
  mọi tool khác trên máy mất kết nối adb và **mọi `adb forward` chết theo**, phải
  forward lại từng máy. Chỉ chạy khi người vận hành yêu cầu, và log lại.
  Nền tảng cho mấy điều trên: `docs/re/genfarmer/README.md`.

`MultiplexDriver` (`crates/core/src/driver_multiplex.rs`) gộp hai backend vào
**một** `DeviceControlPlane`. Không tách hai plane: `DeviceRegistry::upsert_many`
thay cả vector và phát `DevicesUpdated` mang toàn fleet, nên hai plane poll độc
lập sẽ luân phiên xoá máy của nhau; `DeviceExclusiveContext` còn mang `plane_id`
mà mọi transition đều kiểm. **Bảng route chỉ dựng từ `list_devices`** — không
bao giờ đoán nền tảng từ chuỗi udid; việc repo không validate định dạng udid là
tài sản, giữ nguyên. Một backend hỏng **không** che backend còn lại.

`supports_text_comments`, `supports_verified_app_termination`,
`requires_fresh_text_session`, `supports_push_media` giờ **nhận udid**: trên
fleet trộn, câu trả lời fleet-wide là lời nói dối về máy của nền tảng kia.

Backend Android chỉ tham gia khi `adb` thực sự dùng được (`detect_driver` chạy
`adb version` lúc khởi động), và lý do vắng mặt nằm ở
`android_unavailable_reason` **riêng** với `driver_degraded_reason` — "máy này
không có adb" và "sidecar iOS chết" là hai sự việc khác nhau.

`StreamHub` **dùng chung** cho cả hai backend qua `riviu_core::FrameSink` (xem ở
trên); `ensure_stream` của Android trả `auto-stream://<serial>` vì frame được
producer minicap publish thẳng vào hub, không có URL riêng cho session.

### 9.1 Nurture Android: cùng policy, khác cách nhìn (10/08/2026)

`crates/core/src/nurture/hierarchy.rs`. Đây chính là điều "**không port
`screen.rs` sang Android**" hàm ý, không phải một ngoại lệ của nó.

- **Chia đúng một chỗ: quan sát.** Vòng lặp Android **dùng lại nguyên** tầng
  policy của engine iOS — `HumanBehavior` (dwell, swipe duration, fatigue),
  `HumanSessionPolicy` (cap mỗi post/phiên, action gap, rest), `MoodCycle`,
  `roll_feed_action_in_mood`, `TouchPointPlanner`. Nhân bản policy sẽ để hai
  backend trôi thành hai người dùng khác nhau; đó là lỗi, không phải tự do.
- **Seam là `UiSession::locate_description(value, exact)` → `ElementBox`.** Trả
  *hình chữ nhật*, không phải text — `read_text` không đủ. iOS mặc định refuse vì
  `snapshotMaxDepth` bị ghim ở 1 (§2.3), nên `supports_element_bounds()` là cửa
  phân luồng: `false` → engine pixel, `true` → hierarchy. `run_session` thử
  hierarchy **trước** cửa `calibrated_layout`, nên iOS đi qua không đổi một byte.
- **Tính chất rơi ra từ việc định vị trước khi tap: mọi tap đều sinh từ một
  rectangle máy trả về.** Thẻ nào vòng lặp không hiểu thì **không có tap nào**.
  Không tồn tại lỗi "tap vào vị trí rail hoá ra không có" mà engine pixel phải
  chống bằng `FeedCardKind`.
- **Chứng minh tim mà không cần nhãn `Liked`**: nhãn *chưa-tim* là exact match,
  nên khi state đổi thì đúng chuỗi đó biến mất. Bắt buộc nút bình luận **vẫn còn**
  cùng lúc mới loại được lý do khác (thẻ đã chuyển). Đó là bằng chứng thật.
  Nhãn `Liked` giờ **đã đo** (`Đã thích video`) nên đường chính dùng nó.
- **Chứng minh vuốt bằng nhãn, không bằng pixel**: nhãn bình luận và chia sẻ mang
  số đếm riêng của từng post (`… 697 bình luận` → `… 1.665 bình luận`), nên cặp
  đó đổi theo thẻ. Fingerprint rỗng ở cả hai đầu **không** tính là video.
- **Đừng đọc fingerprint bằng một lần đọc sau sleep cố định.** Đo thật: một phiên
  300 s với 1 lần đọc ở 900 ms báo 6/34 lượt vuốt là "chưa chứng minh", và **mỗi
  lần đều kéo theo một thẻ trông như không có rail** — cùng một frame giữa
  transition bị đếm hai lần. Phải **chờ rail quay lại** (`RAIL_RETURN_WINDOW`
  2,6 s) rồi mới fingerprint.
- **Thẻ LIVE nhận qua việc thiếu rail, không qua nhãn.** Bản `vi` chưa đo được
  `LiveRoom` (chưa gặp post LIVE nào), nên `Comments` vắng mặt là dấu hiệu dùng
  thật. Đo nhãn khi nào gặp; đừng đoán.
- **Drawer bình luận đã đo xong (10/08/2026), bằng `probe --measure-comment`.**
  Ba thứ, đo từng cái, không đoán:
  - **Ô nhập** là `android.widget.EditText`, định vị bằng **class**:
    `content-desc` rỗng và `text` là placeholder (`Thêm bình luận...`) nên không
    có nhãn nào bám được. Mở drawer **không** focus nó — phải tap; đo được ô nằm
    ở `[199,2127]` khi chưa focus và nhảy lên `[199,1175]` khi bàn phím lên.
  - **Nút Gửi** là `android.widget.Button` với `content-desc="@2131823284"` —
    **một resource id chưa resolve, không phải chữ**. Hệ quả kép: không phụ thuộc
    ngôn ngữ, nhưng **vỡ khi TikTok cập nhật** vì resource id bị gán lại. Đừng
    coi nó cùng loại với các nhãn khác. **Và điều đó đã xảy ra thật — xem
    §9.13.**
  - **Bằng chứng "armed"**: đúng nút đó có `enabled="false"` khi ô rỗng và
    `enabled="true"` ngay khi có chữ. Đây là câu trả lời của hierarchy cho
    `CommentDrawer::SendArmed` mà engine pixel phải dò bằng màu. Vì vậy
    `ElementBox` mang thêm `enabled`, và mặc định khi **không đọc được** là
    `true` — mặc định `false` sẽ báo nút đang sống là chưa armed và âm thầm bỏ
    mọi bình luận.
  - **Đã chạy được trên máy thật tới bước armed**; **bước bấm Gửi chưa chạy** vì
    nó đăng công khai dưới tên tài khoản thật — cần người vận hành đồng ý. Chạy
    `--example nurture -- <serial> --comment "<chữ>"` để thực hiện.
- **`UiSession::back()` là phím mới, không phải tiện tay thêm.** Đóng drawer bằng
  `home()` sẽ **thoát hẳn TikTok**. iOS mặc định refuse `back()` vì không có phím
  back hệ thống; Android dùng `KEYCODE_BACK`.
- **Chữ bình luận đi qua `CommentTextSource`, và app cắm đúng generator của
  iOS** (`prepare_hierarchy_comment`) nên hai nền tảng nói cùng một giọng và ghi
  cùng một bảng audit. Nhưng **không dùng `collect_comment_frames`**: hàm đó chặn
  bằng `screen::feed_ready` — detector hiệu chỉnh cho iPhone 8 — nên trên frame
  Android nó loại sạch và mọi bình luận sẽ ra "context unavailable". Dùng
  `collect_grounding_frames` (không có cửa pixel): vòng lặp đã chứng minh tab feed
  và thanh hành động có trên màn qua hierarchy, mạnh hơn heuristic pixel.
- **`NurtureSettings.bundle_id` mặc định là bundle iOS** (`com.ss.iphone.ugc.Ame`)
  — không máy Android nào mở được. `ensure_tiktok_foreground` đọc foreground
  *trước*, và nếu phải tự mở mà package cấu hình không nằm trong catalog thì
  refuse kèm danh sách package đúng: đây là lỗi cấu hình, retry không chữa được.
- **Gate G2 là `cargo run -p riviu-android-driver --example nurture`**, gọi thẳng
  `riviu_core::nurture::run_hierarchy_session` — không control plane, không DB,
  không stream. Số đo 10/08/2026 trên Redmi Note 12: **28/34 video, tim 10/10 đều
  được nhãn xác nhận, outcome `done`, 363 s**; sau khi sửa lỗi settle: 5/6 và 2/2.
  Lưu ý `--videos` bị **bỏ qua** khi có `--seconds`: phiên có thời hạn thì
  `total_videos` thành vô hạn (đúng quy tắc engine iOS).
- **`probe --measure-liked` là cách duy nhất đọc nhãn đã-tim**, và nó tự hoàn
  nguyên: tim → đọc → bỏ tim → kiểm tra nhãn cũ trở lại, và **hét lên** nếu
  không. Không đoán bản dịch: `Đã thích video` khác `Đã thích`, và trật tự từ
  ngược so với `Video liked` của bản tiếng Anh.

### 9.2 ĐÍNH CHÍNH (11/08/2026): clipboard KHÔNG chặn Interaction

Một phiên bản trước của mục này viết *"Clipboard Android vẫn bị chặn nên
Interaction campaign chưa chạy"*. **Sai, và sai theo hướng đắt**: nó chỉ người
đọc đi cài IME hoặc `io.appium.settings` để mở một cái không hề chặn.

Đo lại bằng cách đọc mã, không phải suy từ tài liệu:

- `set_streaming_clipboard` / `get_streaming_clipboard` / `guarded_clipboard_transition`
  **không có caller nào ngoài test** (`device_control.rs:4770`, `:4803`, `:4836`).
  `crates/core/src/interaction.rs` không hề nhắc clipboard.
- Interaction giao URL bằng `session.open_url` (`interaction_commands.rs:967-970`),
  mà Android **đã có** (`session.rs:233-246`, `am start -a …VIEW -d '<url>'`).
- Phần **đo** ở §9 về clipboard vẫn đúng nguyên (uiautomator2 `get_clipboard` trả
  rỗng; MIUI `activePermissionOwner`). Chỉ **kết luận về Interaction** là sai.
- §3.12 Copy Link là **đặc tả chưa ship**: `interaction_commands.rs` đi thẳng từ
  `plan_threads` sang DB sang worker; không có sentinel, không có
  `identity_copy_intent`. Identity được chứng minh bằng `open_url` + frame-SHA
  đổi + `locate_action_rail` + OCR handle tác giả. Đọc mã làm hợp đồng, không đọc
  đặc tả đó.

**Chỗ tắc thật là 5 method control-plane mà `AndroidDriver` chưa implement**:
`confirm_interaction_stream_stopped`, `start_interaction_session`,
`start_stream_after_session`, `stop_owned_stream`, `park_owned_stream`. Chúng rơi
về `unsupported(...)` của trait, và hệ quả rộng hơn Interaction:

- **Nurture Android chưa từng chạy qua app.** `open_ui_context`
  (`nurture/mod.rs:215-231`) → `try_start_interaction_session`
  (`device_control.rs:718-767`) → `confirm_interaction_stream_stopped` (`:729`) →
  refuse. Số đo §9.1 (28/34 video) đến từ `examples/nurture.rs`, mà module doc của
  nó nói thẳng là **bỏ qua** control plane. Example đó vẫn là gate đúng cho vòng
  lặp thuần — đừng "sửa" nó để đi qua plane.
- **Tile Android hỏng ở bước park.** `StreamSampler` (`state.rs:298`) →
  `stop_background_stream` → `clean_background` → `park_owned_stream`
  (`device_control.rs:3005`) → refuse. Mỗi tile lên `● Live` rồi kết thúc lượt ở
  `TileStreamState::Error`, sample bị giữ mãi. Bug người dùng thấy được, độc lập
  với nurture.
- **`reserve_ui_capacity` chết còn sớm hơn cả bước 1.**
  `preview_foreground_victim` (`stream_budget.rs:354-390`) có thể trả về **chính
  máy đích** làm victim khi nó đang giữ producer background, và `reserve_context`
  gọi `stop_owned_stream` với `quarantine: true` (`device_control.rs:2812-2820`).
- **`stop_minicap` (`driver.rs:247`) không có caller nào** — dead code. Đó là lý do
  không ai phát hiện teardown Android chưa từng chạy.

Nên **năm method phải cùng lên**; thiếu `stop_owned_stream` thì vẫn fail ở reserve.

**Đã làm, đo được (11/08/2026, Redmi Note 12).** Gate là
`cargo run -p riviu-android-driver --example control_plane -- <serial>` (G3), lái
`DeviceControlPlane` thật:

```
start_background_stream -> auto-stream://10969614
acquire_exclusive ok / reserve_ui_capacity ok
start_interaction_session ok (foreground proven)
start_reserved_stream ok      sink: generation=2 published=1 cleared=2 parked=0
session: screen (1080,2400) foreground com.ss.android.ugc.trill
close_ui_context -> stopped_generation: 2, next_generation: 3
cleanup_quarantine_count = 0      forwards after: none
```

- **`cleanup_quarantine_count() == 0` là gate thật**, không phải chuỗi log ở trên.
  Một `old_generation` sai hay `child_stopped: false` chỉ hiện ra dưới dạng ticket
  bị quarantine; mọi thứ khác trong lần chạy vẫn trông sạch.
- **G3 phải mở tile bằng `reserve_background_stream` + `start_background_stream`,
  KHÔNG phải `driver.ensure_stream` trực tiếp.** Gọi driver thẳng thì
  `StreamBudgetManager` không biết có producer nên `reserve_ui_capacity` không tìm
  ra victim để thu hồi, trong khi driver vẫn giữ minicap — và handoff từ chối
  đúng, nhưng gate không chứng minh được gì về đường app đi. Đã mắc lỗi này một
  lần; comment trong example ghi lại để không mắc lại.
- **`child_stopped: true` khi không có gì để stop.** `confirms_stop` đòi
  `child_stopped && new > old`, và `clean_ticket` quarantine cả lease nếu không
  thoả — trả `false` cho "không có producer" sẽ quarantine mọi teardown sau một lần
  start stream thất bại. iOS trả lời y hệt.
- **`first_frame_observed` là JPEG *decode được*, và timeout là lỗi chứ không phải
  `false`.** `riviu_core::frame_source::decodes_as_jpeg` nằm ở core để hai backend
  không thể bất đồng, và để không backend nào phải tự mang decoder. Magic byte một
  mình không đủ: một blob đúng kích thước đúng prefix vẫn không phải ảnh.
- **Khoá**: `starting: Mutex<HashSet<String>>` (claim, nhả trong `Drop` nên future
  bị cancel không treo serial) thay cho việc giữ `streams` — mutex **toàn fleet** —
  suốt `ensure_apk` (timeout 120 s) + spawn + forward + 10 s retry connect. Trước
  đó một máy mở stream chặn mọi máy khác cả start lẫn stop.

### 9.3 Hai bẫy môi trường đo được (11/08/2026) — cả hai từng trông như lỗi locator

**1. `mWakefulness` KHÔNG thấy được máy đang khoá.** Máy khoá màn hình báo
`mWakefulness=Awake`, `mAwake=true mScreenOnEarly=true mScreenOnFully=true` — màn
hình **thật sự đang bật** — trong khi `mCurrentFocus=NotificationShade` và mọi lệnh
đưa app lên trước im lặng không làm gì. `monkey` vẫn exit 0. Ba key nói đúng sự
thật: `isKeyguardShowing=true`, `mKeyguardShowing=true`, `mDreamingLockscreen=true`
(`adb::parse_keyguard_locked` nhận cả ba vì không key nào chắc chắn có trên cả dải
Android 9 → 15). Bất kỳ true nào cũng tính là khoá: từ chối oan một máy tốt cho ra
thông báo rõ, còn nói sai là "đã mở" thì driver đi tap lên lock screen.

**2. Session uiautomator2 SỐNG LÂU bị thoái hoá, làm mọi query rơi vào chế độ
timeout 10 s.** Triệu chứng: element query từ ~150 ms lên **10 000+ ms rồi trả
`absent`**, kèm lỗi `Timed out … waiting for the root AccessibilityNodeInfo in the
active window`. `am force-stop io.appium.uiautomator2.server{,.test}` khôi phục
**118–425 ms ngay lập tức**.

> **ĐÍNH CHÍNH nguyên nhân (cùng ngày).** Bản đầu của đoạn này ghi nguyên nhân là
> **session tích tụ** — sai, và tôi tự viết nó. `GET /sessions` trên chính máy đang
> thoái hoá trả về **đúng một** session, nên không có gì tích tụ; `POST /session`
> thay thế chứ không cộng dồn.
>
> Phép thử phân định: trên session cũ, một `find` mất **10 116 ms** và lỗi. `DELETE`
> session đó rồi `POST` session mới — **không restart agent** — thì cùng câu query
> mất **7 ms**. Vậy thứ mục là **một session dùng lâu**, không phải một đống session.
>
> Điều đó làm bản sửa đầu của tôi **sai hướng**: cache-và-dùng-lại-mãi khiến app
> desktop chạy hàng giờ ôm đúng một session đang mục dần. Bản đúng là **tự tái
> tạo**: `AgentClient::send` bắt đúng chuỗi lỗi của server
> (`waiting for the root AccessibilityNodeInfo`), `DELETE` + `POST` một session mới,
> rồi thử lại **một lần**. `session_id` là `Arc<Mutex<String>>` nên việc tái tạo
> chữa cho **mọi** clone, kể cả `AndroidUiSession` đã trao cho một vòng lặp đang
> chạy. Chỉ retry trên **đúng** chuỗi lỗi đó — retry vô điều kiện sẽ phát lại tap và
> các lệnh không idempotent.
>
> Cache session (`AndroidDriver::agents`) vẫn giữ, nhưng lý do đổi: nó tiết kiệm
> ~2 s `POST /session` mỗi lần mở, **không** phải để tránh tích tụ.
>
> **Chưa quan sát được đường tự tái tạo thực sự nổ.** Bằng chứng về *nguyên nhân* thì
> chắc (phép thử DELETE/POST bằng tay), nhưng lần chạy sau khi sửa lại lấy một session
> mới vì tôi đã xoá session cũ trong lúc thí nghiệm — nên nó không đi qua nhánh
> recycle. Muốn xác nhận: chạy nhiều lượt tới khi query lên 10 s, rồi chạy tiếp và
> xem query có tự về ~150 ms không. `tracing::warn!("recycled a degraded agent
> session")` là dấu, nhưng **example chưa nối subscriber** nên sẽ không thấy dòng đó —
> đo bằng thời gian, hoặc nối subscriber trước.

### 9.11 Bottom tab bar (11/08/2026)

`probe --measure-tab-bar`, read-only. Trên `1080x2400`, năm tab đều `216x135` tại
y=2135:

| x | `content-desc` | class |
|---|---|---|
| 0 | `Trang chủ` | `FrameLayout` |
| 216 | `Cửa hàng` | `FrameLayout` |
| **432** | **`Quay`** | **`Button`** |
| 648 | `Hộp thư` | `FrameLayout` |
| 864 | `Hồ sơ` | `FrameLayout` |

Nút mở composer mang `content-desc="Quay"` — tức "quay phim", **không** phải tên của
việc mà đường publish dùng nó (đi tới picker thư viện). Đó là chuỗi **đo được**;
đừng "sửa cho dễ đọc". Đã vào catalog là `TikTokControl::ComposerOpen`.

Mỗi tab còn có một `TextView` nhãn riêng ở y=2219 với `content-desc` **rỗng** và chữ
nằm ở `text` — cùng dạng như nút Reply, và là lý do nữa để `LabelAttribute` tồn tại.

**Mọi thứ bên trong composer vẫn chưa đo** (mục thư viện, album picker, grid ảnh,
Next, ô caption, Đăng, xác nhận công khai) nên `composer_open` là nhãn publish
**duy nhất** có trong catalog, và nhánh publish hierarchy phải **từ chối** cho tới
khi đo xong. Đo tiếp bằng cách bấm `Quay` rồi dump — nó mở camera, nên đó là một
bước xâm lấn hơn read-only và nên chạy khi có người xem.

> **Điều này làm lung lay một kết luận cũ của mục này.** §9 ghi chế độ 10 s là
> thuộc tính của **feed đang phát** trên fleet S8+ (p50 10531 ms, 20/20 query, và
> `waitForIdleTimeout: 0` "đã xác minh áp dụng và không đổi gì"). Phép đo hôm nay
> trên Redmi cho thấy nguyên nhân đủ để gây ra đúng triệu chứng đó là **session
> tích tụ**, và feed vẫn đang phát khi query trở lại 118 ms. Chưa đo lại được trên
> S8+ nên **không kết luận** số cũ sai — nhưng ai đo lại phải khởi động agent sạch
> trước, nếu không rất dễ đo lại chính hiện tượng này và gán cho feed.

**Triệu chứng của cả hai bẫy đều là "không thấy nhãn"**, tức trông y như catalog
nhãn sai. Trước khi nghi nhãn, kiểm hai điều này.

### 9.5 Hàng comment trong drawer (11/08/2026) — nhãn nằm ở `text`, không phải `content-desc`

Đo trên `com.ss.android.ugc.trill` **46.3.3** (Redmi Note 12). Một hàng comment:

| thành phần | class | thuộc tính mang nhãn | x | vị trí |
|---|---|---|---|---|
| tên tác giả | `Button`, clickable | `text` | 174 | **trên** body |
| nội dung | `TextView`, không clickable | `text` | 174 | — |
| **nút Reply** | `Button`, clickable | **`text="Trả lời"`**, `content-desc` **rỗng** | 307 | **dưới** body, **phải** của body |
| `Xem N câu trả lời` | `Button`, không clickable | `text` | 237 | dưới nút Reply |

Bước dòng ~300 px, và dải dưới một body **với tới nút Reply của hàng sau** — đó là
lý do "gần nhất phía dưới" là load-bearing, không phải "cái tìm thấy đầu tiên".

- **`content-desc` không đủ.** Nút Reply có `content-desc` rỗng. Vì vậy
  `LabelMatch` có thêm `Text`/`TextContains` và `ElementQuery` có thêm
  `Text { value, exact }`; `LabelMatch::to_query()` là **một** chỗ dịch duy nhất,
  vì bản copy nào quên một variant sẽ fail bằng cách *không tìm thấy gì*, không
  phân biệt được với "control không có trên màn".
- **Một nhãn ↔ nhiều phần tử.** `locate_all` (qua `POST /elements`) là bắt buộc:
  đo được **4** nút Reply cùng lúc. Chọn cái nào là câu hỏi **hình học**, không phải
  câu hỏi matching — chọn sai là đăng reply dưới comment người lạ, và điều đó vô
  hình trong log.
- **`locate_all` cố ý bỏ đọc `content-desc`/`enabled`.** Đó là 2 round trip **mỗi
  phần tử**, và nó chạy trên cả danh sách. Đo: 4 phần tử 684 ms, 13 phần tử
  1172 ms (~90–170 ms/phần tử chỉ với `rect`). Nên `description` trả `None` ở
  đường này; ai cần nhãn thì gọi `locate` cho phần tử cụ thể.
- **Reply lồng nhau thụt vào.** Đo: `reply[2]` ở x=**374** trong khi ba cái còn lại
  ở x=307. Đây là dữ liệu thật cho luật "tên tác giả phải có lề trái tương đương
  body" — một nhãn thụt vào thuộc về hàng khác.
- **`Trả lời` là bản dịch, không phải resource id** — trái ngược `comment_send`
  (`@2131823284`). Nên nó **phụ thuộc ngôn ngữ nhưng bền qua update**. Hai kiểu dễ
  vỡ khác nhau, đừng đối xử giống nhau. Chuyện "vỡ âm thầm khi TikTok cập nhật"
  **đã xảy ra và đã đo được** — xem §9.13.
- **Luật định vị được *port*, không viết lại**
  (`crates/core/src/interaction_hierarchy.rs`): body phải xuất hiện **đúng một
  lần**, tác giả là nhãn gần nhất **phía trên** có lề trái tương đương, nút Reply là
  cái **gần nhất phía dưới** và **bên phải** body. Cả bốn test đối kháng của đường
  OCR được port sang, cộng test mới cho "body chứa chuỗi cần tìm nhưng dài hơn".
  Verify trên máy: `RESOLVED author="Ghét tháng 9." reply at 307,1149`.
- **Ưu thế so với đường OCR**: chuỗi đem đi so là chuỗi **chính project gõ ra** qua
  `ACTION_SET_TEXT`, nên không mất mát phiên âm và toàn bộ bộ gấp dấu
  (`normalize_locator_text`, `LATIN_FOLD`) **không cần** ở đây.
### 9.6 Package TikTok theo từng máy (11/08/2026)

`TIKTOK_BUNDLE_ID` từng là **hằng module ở ba file** desktop, cả ba ghi bundle
**iOS**. Nó được truyền vào `start_interaction_session` rồi so với
`active_app_bundle()` — trên Android không bao giờ khớp, nên vòng chờ chỉ thoát
bằng timeout. Cùng defect ở `commands.rs`, tức manual control và Open-on-Device
trên máy Android cũng chết y hệt.

Giờ có `DeviceDriver::resolve_tiktok_package(udid)` (default = bundle iOS: với
backend một app id cố định thì đó là **sự thật**, không phải phỏng đoán) và
`crates/core/src/tiktok_target.rs`:

- **Đọc package đã cài, không đọc foreground.** Lúc caller cần giá trị này thì chưa
  có session và máy có thể đang ở launcher. Foreground là cách đúng để **phá thế
  ngang bằng** khi có hai build cài cùng lúc, và là cách sai để giải từ đầu — đúng
  chỗ `hierarchy.rs::ensure_tiktok_foreground` đọc foreground trước, vì ở đó nurture
  đã chạy *trên feed*.
- **`pm list packages com.foo` khớp theo substring**, nên `com.foo.bar` cũng trả về.
  So cả payload sau `package:`, không dùng "contains" — nếu không thì bản Lite
  (`com.zhiliaoapp.musically.go`) sẽ được nhận là bản đã đo nhãn. Có test.
- **Hai build cùng cài là nhập nhằng**, không phải "lấy cái đầu": refuse trừ khi
  foreground phá được thế. Đọc foreground bằng adb trực tiếp — giải một package
  **không được** có tác dụng phụ là mở session.
- **Danh sách build hợp lệ suy ra từ `TIKTOK_LABEL_SETS`**, không viết lại: build
  không đọc được nhãn thì không lái được, nên hai danh sách không được phép lệch.
  Có test khẳng định điều đó.
- Memoise theo serial (mỗi candidate là 1–2 s adb), xoá khi `refresh_device` —
  refresh là lúc operator nói "xem lại", cũng là lúc một build có thể vừa được cài
  hoặc xoá.
- `reports_element_bounds(udid)` là **dự đoán pre-flight** trả lời được không cần
  session, để gate picker và chọn chiến lược trước khi chạm máy;
  `UiSession::supports_element_bounds` vẫn là thẩm quyền runtime. Lệch nhau thì theo
  session và ghi lại, đừng âm thầm chọn đường kia. Đặt tên theo **tính chất code
  thật sự phụ thuộc**, không theo nền tảng: "android nghĩa là hierarchy" là suy diễn
  có thể sai.
- Verify: G3 giờ **giải** package thay vì đọc `RIVIU_TIKTOK_PACKAGE` →
  `resolved target com.ss.android.ugc.trill`, chuỗi handoff đủ, quarantine 0.

### 9.7 Composer reply (11/08/2026) — bốn câu hỏi, bốn câu trả lời đo được

`probe --measure-reply`, trên `com.ss.android.ugc.trill` 46.3.3. Kết quả sạch hơn
dự đoán, và cả bốn đều là điều **không suy ra được**:

| Câu hỏi | Đo được | Hệ quả thiết kế |
|---|---|---|
| Có `EditText` thứ ba không? | **Không** — vẫn đúng **1**; composer **thay** ô chứ không xếp lên. `.focused(true)` tìm được. | Bẫy hai-`EditText` không bật ở đây. `type_text` dùng nguyên. |
| `@nickname` có prefill? | **Không có `@` nào.** `text` là **placeholder** `"Trả lời Ghét tháng 9."`, không phải nội dung. | `set_text` an toàn — không có mention phải giữ. Đây là ẩn số hệ quả nhất và câu trả lời là câu đơn giản. |
| Nút Send có đổi? | **Cùng `@2131823284`**, `enabled` **cùng false→true**. | `crate::tiktok_drawer` dùng lại **không sửa gì** cho reply. Không cần `CommentReplySend`. |
| Back từ composer đi đâu? | **Về danh sách comment**, drawer **vẫn mở** (feed tab chưa hiện). | Đúng thứ Interaction cần: đọc lại reply vừa đăng từ danh sách còn mở. |

**Phát hiện thêm, đáng xây trên đó**: placeholder `"Trả lời <tên tác giả>"` **nêu
tên người đang được reply**. Đó là một bằng chứng **độc lập** rằng đã bấm đúng nút
Reply — mạnh hơn chỉ tin vào hình học. `send_reply` nên đọc nó và kiểm **trước khi
gõ**; lệch thì từ chối, vì tới lúc đó chưa có gì được đăng.

Cách kiểm đúng là **`text` của ô có chứa `author_label`** đã lưu trong
`CommentLocatorIdentity`, chứ **không** phải so tiền tố: `"Trả lời Ghét tháng 9."`
chứa `"Ghét tháng 9."`. Nhờ vậy phép kiểm **không phụ thuộc ngôn ngữ** và không cần
thêm entry catalog nào — tiền tố `"Trả lời "` là bản dịch, nên nếu đem nó vào thì
lại tự tạo ra một nhãn nữa phải đo cho mỗi ngôn ngữ, để đổi lấy đúng số 0.

**Chưa đo**: trang bài mở từ link (`--measure-target-open`, cần một link TikTok
thật). Gate Threaded cần **hai** máy Android có nhãn đã đo; hiện chỉ một máy cắm.

### 9.8 `tiktok_drawer` — tách dùng chung, không nhân bản

`crates/core/src/tiktok_drawer.rs`. Nurture và Interaction dùng **một** bản, vì
drawer là thứ đo đắt nhất trong repo và hai bản sao của "cờ `enabled` của nút Gửi
là bằng chứng armed" sẽ trôi — trôi ở đó nghĩa là hoặc bình luận bị bỏ âm thầm,
hoặc **cùng một bình luận đăng hai lần**.

- **Các bước để rời, và `leave` là quyết định của caller.** Nurture muốn đóng drawer
  về feed; Interaction cần **để mở** sau khi gửi, vì evidence của nó đọc lại bình
  luận từ danh sách và luồng reply làm việc trong cùng drawer đó. Một
  `post_comment` luôn đóng drawer không phục vụ được cả hai.
- **`TapPlanner` là generic, không phải `&mut dyn FnMut`.** Trait object buộc phải
  gọi tên mọi auto trait mà future cần (thử `+ Send` rồi vẫn đòi `Sync`); generic
  để closure của caller tự mang. Nhờ vậy nurture vẫn giữ lịch sử jitter của
  `TouchPointPlanner`, còn probe truyền tâm phần tử là xong.
- **Bẫy khi viết test cho state machine này**: fake trả lời theo hàng đợi thì khi
  hết đáp án nó báo phần tử **absent**, mà "nút Gửi biến mất" thì flow **đúng** khi
  coi là đã gửi (drawer đóng). Nên tình huống *mơ hồ* thật — nút còn đó và vẫn
  armed — cần fake trả lời **bền**. Test đầu tiên tôi viết fail vì lý do này, và
  đó là fixture sai chứ không phải code sai.

### 9.9 Gate actor Interaction: theo *tính chất*, không theo nền tảng

`require_parent_locator` thay `require_vietnamese_reader`. Lý do cũ nói yêu cầu OCR
là thuộc tính của **máy tính chạy app** — sai: nó là thuộc tính của **actor**. Máy
đọc được hierarchy không gọi `interaction_ocr` lần nào, nên ngôn ngữ OCR của host
không liên quan tới nó.

Và một ràng buộc **mã cũ không có khái niệm**, vì tình huống chưa thể xảy ra:
**campaign Threaded trộn hai loại máy phải bị từ chối** (`MixedPlatformThread`).
Chuỗi là tuyến tính và mỗi message gửi từ actor **khác**, nên message N phải tìm
được bình luận của N−1. Actor hierarchy lưu `author_label` đọc từ `text` node;
actor pixel sau đó phải tìm lại hàng đó bằng OCR và **khớp author label**. Phần
body sẽ khớp — cả hai so với chuỗi chính project gõ ra — nhưng author label có thể
không: badge, truncation, khác biệt rendered-vs-attribute. Từ chối bây giờ không
mất gì vì chưa ai chạy campaign trộn, và rẻ hơn nhiều một mắt xích đứt giữa chừng
không rõ lý do. `Standalone` không bị ảnh hưởng — nó không có cha để tìm.

Gate UI actor picker **vẫn lọc về iPhone** cho tới khi nhánh `TargetDriver` có
thật: đường gửi hierarchy chưa nối vào `execute_thread_campaign`, nên nới picker ra
bây giờ chỉ cho operator chọn máy rồi fail.

### 9.10 MediaStore (11/08/2026) — `adb push` là đủ, không cần scan

`probe`/`media_probe` trên Redmi Note 12, Android 15. Trước phép đo này repo **không
có một dòng nào** về MediaStore, nên toàn bộ đường publish Android là suy đoán. Kết
quả nhẹ hơn nhiều so với dự kiến:

| Câu hỏi | Đo được |
|---|---|
| `adb push` có đủ để MediaStore thấy file? | **Có, và không cần scan.** Cả ba thư mục thử đều thấy trong ~1,5 s: `/sdcard/DCIM/Camera`, `/sdcard/Pictures`, `/sdcard/Movies`. |
| Có cần `MEDIA_SCANNER_SCAN_FILE`? | **Không.** Nhờ vậy tránh hẳn bẫy `result=0` mà §"ĐÍNH CHÍNH" đã ghi cho `ADB_INPUT_TEXT`. |
| `_id` đọc được để xoá đúng row? | Có: `1000011139`, `1000011140`, `1000011141`. |
| Thứ tự có giữ? | **Có.** `content query` trả đúng thứ tự push (`date_added` cách nhau ~1 s/file). |
| Cleanup có idempotent? | **Có.** `content delete --where "_data LIKE '%riviu-media-probe%'"` lần 1 → 0 row, lần 2 → 0 row, không lỗi. 5582 ms cả chu trình. |

**Hệ quả thiết kế**: staging + import Android **không cần** helper trên máy, không
cần MediaStore insert API, không cần root. Chỉ `adb push` vào một thư mục công khai
với tên có tiền tố riviu — cùng convention `frames.rs` đã đặt cho `minicap.apk` để
bản của một farm tool khác không bị nhận lẫn. Cleanup chạy hai lần vẫn `cleaned` —
đúng thứ `publish_commands.rs` assert. (Phép đo dùng `--where "_data LIKE …"` cho
tiện; **code thì không được** — xem luật `_id` ở dưới.)

**Cách kiểm là `content query`, không phải exit code của push hay của broadcast.**
Probe cố ý viết theo hướng đó, vì "file có trên đĩa" không phải "app khác thấy
được file".

**Tách được `stage` khỏi `import` bằng thư mục có dấu chấm đầu — đo được:**

| đường dẫn | file trên đĩa | MediaStore |
|---|---|---|
| `/sdcard/Pictures/.riviu-stage-test/dot.png` | **có** (1 104 834 bytes) | **No result found** |
| `/sdcard/Pictures/riviu-plain-test/plain.png` | có | **thấy** |
| sau `mv` dot-dir → `/sdcard/Pictures/riviu-import-test/` | có | **thấy** (`_id=1000011143`) |

Nên hợp đồng publish map sạch sang Android, và **giữ đúng ngữ nghĩa hai bước** của
iOS (stage vào sandbox không thấy được, import mới hiện ra):

- **stage** → `adb push` vào một thư mục **có dấu chấm đầu**: file ở trên máy, và
  MediaStore (do đó cả picker TikTok) **không thấy**.
- **import** → `mv` sang thư mục thường. MediaStore thấy ngay, **không cần**
  broadcast scan. `mv` trong cùng volume là rename, nên rẻ và nguyên tử.
- **cleanup** → xoá row theo `_id` rồi `rm` file.

**Xoá theo `_id`, đừng xoá theo `_data LIKE '%riviu%'`.** Trên máy này có sẵn
`/storage/emulated/0/riviufarm-shot.png` — của **GenFarmer**, không phải của mình.
Một mẫu `LIKE '%riviu%'` sẽ xoá luôn nó. Tiền tố phải hẹp và cleanup phải nhắm `_id`
đã biết.

#### MediaStore thấy là **cần và chưa đủ** — `is_pending` mới là điều kiện đủ

Đây là phép đo đắt nhất của mục này, và nó phủ định kết luận ở trên nếu chỉ đọc tới
đó. **Picker của TikTok không liệt kê một row nào có `is_pending=1`.** Row do
`adb push` sinh ra **luôn** có `is_pending=1` — cờ scoped storage nghĩa "một app còn
đang ghi file này" — và row pending thì **mọi app khác đều không thấy**, kể cả khi
file đã nằm trên đĩa và đã có row trong MediaStore.

So hai row cạnh nhau trên cùng máy:

| | ảnh do camera chụp | row của `adb push` |
|---|---|---|
| `_size` | 2117779 | **NULL** |
| `width`/`height` | 3072 / 3072 | **NULL** / **NULL** |
| `is_pending` | **0** | **1** |
| `owner_package_name` | `com.android.camera` | `com.android.shell` |

`content update --uri content://media/external/images/media/<id> --bind is_pending:i:0`
— **chỉ một lệnh đó, không gì khác** — làm ảnh import hiện ra ở **ô đầu tiên** của
picker (đối chiếu từng dòng với ảnh gốc: `Wi-Fi … Riviu 4 Zbtlink 2.4G`,
`Bluetooth`, `Mạng di động`). Hết pending thì MediaProvider **tự** scan file và tự
điền `_size=160276`, `width=1080`, `height=2400`, `date_modified`.

Bốn nghi phạm đã bị loại bằng phép đo, đừng đi lại:

- **Không phải thư mục.** Vẫn vắng khi row ở `DCIM/Camera`.
- **Không phải cache của TikTok.** Vẫn vắng sau khi tắt hẳn và mở lại TikTok.
- **Không phải timestamp.** Row mà picker **đã nhận** vẫn có `datetaken=NULL`. Đoạn
  code stamp `datetaken`/`date_added` bằng tay trước đó là cargo-cult, đã bỏ.
- **Không phải `owner_package_name`.** Không đổi được nó và cũng không cần.

Nên `crate::publish::import` làm đúng một update đó cho từng row rồi **đọc cờ lại**
bằng `--projection _id:_data:is_pending` (`parse_pending_rows`), vì exit code 0 của
`content update` không phải bằng chứng — cùng lý do `result=0` của `am broadcast`
không phải bằng chứng. Và nó **sort row theo `_data`** trước khi update: thứ tự
carousel là một phần của bài đăng, `content query` không hứa thứ tự nào.

#### Mục thư viện trong composer: ô **không có nhãn** ở góc dưới-trái

Nó **không có `content-desc` lẫn `text`**, nên không đọc được từ tree. Ba `View`
clickable 204x204 xếp hàng ở y=1780 bên phải nút chụp trông giống ứng viên nhất, và
bấm thử hai cái (x=642, x=846) thì **cả hai mở bảng hiệu ứng** (`Mọi hiệu ứng`) —
sai cả hai.

Thứ tìm ra nó không phải là đọc tree kỹ hơn mà là **xem ảnh chụp màn hình**: thư
viện là `FrameLayout` **không nhãn** ở `y=2077 x=0`, `220x165` — góc dưới-trái, dưới
tab chế độ. Ghi lại đây vì bài học lặp lại: khi tree không có nhãn cho một control,
nhìn pixel trước khi bấm thử, chứ bấm thử trên máy thật thì mỗi lần sai là một trạng
thái phải dọn.

Nhãn composer đã đo được: `Lật`, `Thêm âm thanh`, `Flash`, `Hẹn giờ`, `Bố cục`,
`Mic`, `Tỷ lệ`, `Menu thả xuống`, nút chụp `@2131823324`, tab chế độ
(`10 phút`/`60s`/`15s`/`ẢNH`/`VĂN BẢN`), và `ĐĂNG`/`TẠO` ở y=2112.

#### Hai máy Android hành xử KHÁC NHAU — import phải hỏi, không được suy ra

Đo cùng ngày trên máy thứ hai: **SM-N950F, Android 8.0 (API 26)**, locale `vi-VN`,
TikTok `com.ss.android.ugc.trill` **46.4.3** (Redmi là 46.3.3), `wm size` Override
`1080x2220`.

| | Redmi Note 12 / API 35 | SM-N950F / API 26 |
|---|---|---|
| `adb push` đủ để MediaStore thấy | **có** | **không** |
| `mv` vào thư mục thường đủ | **có**, ~1,5 s | **không bao giờ** |
| `MEDIA_SCANNER_SCAN_FILE` | không cần | **cần, và có tác dụng** |
| Cột `is_pending` | có, và bắt đầu ở 1 | **không có cột này** |

Nên kết luận "`adb push` là đủ, không cần scan" ở đầu mục này **chỉ đúng cho API 35**.
`crate::publish::import` xử theo *hành vi máy trả lời*, không theo API level: poll →
nếu rỗng thì broadcast scan từng file → poll lại → chỉ clear `is_pending` trên máy
**báo có cột đó**. Evidence JSON trả `scanBroadcast` và `pendingModel` để nhánh đã đi
là thứ đọc được, không phải thứ suy ra. Gate `media_probe --contract` pass trên cả
hai, và in đúng hai nhánh khác nhau:

```
API 26: import: files=2 scan=broadcast   pending=absent
API 35: import: files=2 scan=not-needed  pending=cleared
```

**BẪY: `content` báo lỗi ra stderr và vẫn exit 0.** Trên API 26,
`content update --bind is_pending:i:0` vào một MediaStore không có cột đó in cả
`SQLiteException` kèm stack trace và trả `rc=0`. `AdbProgram::shell` trả **stdout** và
coi exit 0 là thành công, nên lỗi đó về tới code dưới dạng chuỗi rỗng + `Ok` — và
lượt chạy đầu của tôi **báo `pendingModel: "cleared"` cho một cột máy không có**.
Cùng họ với bẫy `am broadcast … result=0`. Cách sửa: mọi lệnh `content` trong module
thêm `2>&1` **phía máy** (device-side `sh`, portable) và kiểm bằng `content_error()`;
`content query` lỗi cũng **không được** đọc thành "0 row", vì mọi caller đọc danh sách
rỗng như một sự thật về thư viện.

#### `importId` là **khoá chọn ảnh trong picker của TikTok**, không chỉ là handle cleanup

Đây là phát hiện có giá trị nhất cho đường publish. Sau import, mở dropdown album
(`Gần đây` + `Xuống`) thì TikTok liệt kê **đúng chuỗi `importId`** làm một album:

```
[273,504][1034,554]  TextView  text='riviu-picker-check-one-8e69493351ef'
[273,565][289,607]   TextView  text='1'        <- số file
```

Nên `HierarchyPublishDriver` chọn album của campaign bằng **một chuỗi do chính code
này viết ra** — không OCR, không phiên âm, không nhập nhằng — rồi grid chỉ còn ảnh
của campaign đó. Thay được hẳn giả định "mấy ảnh mới nhất trong `Gần đây` là của
mình", vốn là rủi ro đúng-sai lớn nhất còn lại của đường publish Android.

**Chưa đo:** id dài có bị cắt bằng ellipsis trong dropdown đó không. Cái đã đo dài 36
ký tự và hiện đủ. `wda.rs:1493` vốn đã chặn `import_id.len() <= 65`, nên giữ id ngắn
là điều kiện đã có sẵn lý do — giờ có lý do thứ hai.

#### Picker: cấu trúc đọc được, và `Tiếp` là cờ armed

Đo trên SM-N950F (1080x2220), TikTok 46.4.3:

- Tab picker **có `content-desc`**: `Tất cả` (đang chọn), `Video`, `Ảnh`, `Thư viện AI`
  — định vị được bằng `Description`, không cần toạ độ.
- Grid **3 cột**, ô `FrameLayout` clickable **không nhãn**, 354x357, x = 5 / 364 / 722,
  y = 312 / 674 / 1036 / 1398 / 1760. Gần trùng số đo Redmi (352x357, x = 6/364/722).
  Ô không có nhãn nên chọn theo **hình học trong container**, nhưng bounds là số máy
  trả về chứ không phải point iPhone hard-code.
- `Chọn nhiều` (TextView, wrapper clickable `[32,1894][326,2094]`) và `Tiếp`
  (`Button [550,1936][1048,2052]`).
- **`Tiếp` có `clickable=false` khi chưa chọn ảnh nào** — cùng loại bằng chứng với cờ
  `enabled` của nút Gửi trong drawer comment. Nên "đã chọn đủ ảnh" **kiểm được**, không
  phải chờ theo thời gian.

**Và một bẫy nữa:** dump lúc picker đang mở **vẫn chứa node của màn camera phía dưới**
(`Lật`, `Flash`, `Hẹn giờ`, `Bố cục`, `Tỷ lệ`, `Làm đẹp`, `ĐĂNG`/`TẠO`/`LIVE`). Nên
**thấy `Lật` không chứng minh đang ở màn camera**. Muốn biết đang ở picker thì tìm
thứ chỉ picker có (`Thư viện AI`, `Chọn nhiều`, `Tiếp`).

#### Overlay của app khác che được control không nhãn

Trên SM-N950F có bong bóng chat Messenger nổi ở `[53,1952][158,2057]`, **nằm đúng
trong** ô thư viện `FrameLayout [0,1920][210,2070]`. Tap vào **tâm** ô thư viện là tap
vào bong bóng. Tôi phải tap `(188, 1936)` — vẫn trong ô, ngoài overlay — mới mở được
picker.

Vì ô thư viện **không có nhãn**, driver không có cách nào biết mình vừa bấm vào app
khác. Nên đường publish hierarchy phải **xác minh sau khi tap** bằng một node chỉ
picker có (xem trên) và bằng `active_app_bundle()`, chứ không coi tap là xong.

### 9.12 BẪY MÔI TRƯỜNG: Git Bash mangle đường dẫn `adb push`

Nó làm tôi kết luận sai **ba lần liên tiếp** trong một buổi, nên ghi lại.

Trong Git Bash (MSYS2), `adb push <local> /sdcard/x.png` bị dịch đích thành
`C:/Program Files/Git/sdcard/x.png`. `adb` tạo đường dẫn vô nghĩa đó **trên máy** và
báo `1 file pushed, 0 skipped. 28.0 MB/s` — thành công hoàn toàn thuyết phục. Rồi
`adb shell ls /sdcard/x.png` báo không có file, và người đọc kết luận "push nói dối"
hoặc "scoped storage chặn" hoặc "second space lệch user" — tôi đã đi qua đủ cả ba.

Dấu vết duy nhất chỉ đúng chỗ là `adb pull`:
`failed to stat remote object 'C:/Program Files/Git/sdcard/...'`.

- `MSYS_NO_PATHCONV=1` sửa được đích nhưng **làm hỏng path local** (`/c/Users/...`
  không phải path Windows).
- **Cách đúng: chạy `adb push` từ PowerShell, hoặc từ Rust** (`AdbProgram::device`,
  không qua shell). `media_probe` chạy đúng ngay từ đầu chính vì nó là Rust.
- `mkdir -p /sdcard/...` **bên trong** `adb shell '...'` thì an toàn — chuỗi được
  quote nên MSYS không chạm.

Cùng họ với các bẫy shell đã ghi: `>` của PowerShell làm hỏng stdout nhị phân, và
`--8<--` bị device shell hiểu là redirection.

### 9.13 Resource id nút Gửi ĐÃ đổi giữa hai phiên bản app (11/08/2026)

Phần trên viết rằng `comment_send = @2131823284` "có thể vỡ khi TikTok cập nhật".
Máy Android thứ hai chứng minh điều đó **đã xảy ra**, và nó cho thấy khoá catalog
`(package, language)` là **sai** cho loại nhãn này.

| máy | Android | app version | nút Gửi trong drawer |
|---|---|---|---|
| Redmi Note 12 | 15 (API 35) | 46.3.3 | `@2131823284` |
| SM-N950F | 8.0 (API 26) | **46.4.3** | **`@2131823293`** |

Cùng package `com.ss.android.ugc.trill`, **cùng UI tiếng Việt**. Trên 46.4.3,
`@2131823284` **không xuất hiện ở bất kỳ node nào** — nên tra theo ngôn ngữ sẽ
**từ chối một máy đang chạy tốt**. Và nếu resource id được gán lại cho một nút
*khác*, nó sẽ bấm sai nút.

Hợp đồng armed thì **y nguyên** (đo từng bước trên 46.4.3): drawer mở, ô rỗng →
`enabled=false`; tap focus, vẫn rỗng → `enabled=false`; gõ `riviu` →
`enabled=true`. Vậy chỉ có *id* dịch chỗ, `crate::tiktok_drawer` không phải sửa gì.

**Giải: tách catalog theo *kiểu dễ vỡ*, không theo tiện tay.**

- `TIKTOK_LABEL_SETS` — khoá `(package, language)`, chứa **bản dịch**: phụ thuộc
  ngôn ngữ, bền qua update.
- `TIKTOK_RESOURCE_SETS` — khoá `(package, app_version)`, chứa **resource id**:
  không phụ thuộc ngôn ngữ, gán lại mỗi lần build app.
- `controls_for(package, language, app_version)` là **cửa duy nhất**. Không có
  `label()` trên riêng bảng nào, nên **không caller nào đọc lẫn** một id keyed
  theo version ra khỏi bảng keyed theo ngôn ngữ.

**Version chưa đo thì chỉ từ chối resource id, không từ chối cả set.** Đây là khác
biệt giữa backend suy giảm và backend chết: TikTok update xong thì like/đọc vẫn
chạy (bản dịch), chỉ nút Gửi từ chối cho tới khi có người đo lại. Kiểm bằng
`an_unmeasured_app_version_refuses_only_the_resource_ids`.

Version đọc bằng `UiSession::app_version(bundle)` → `dumpsys package <pkg>` →
`parse_version_name`, **một lần mỗi session** (`dumpsys` 1–2 s trên fleet này),
không phải mỗi `locate`. Verify trên máy thật, cả hai đều tự lấy đúng bảng của mình:

```
ce06…646f0d7e  app version = "46.4.3"  resource id đo trên 46.4.3 (SM-N950F)
10969614       app version = "46.3.3"  resource id đo trên 46.3.3 (Redmi Note 12)
```

**Bài học chung, không chỉ cho nhãn này:** khi hai thứ trong cùng một bảng có
*chiều dễ vỡ khác nhau*, một khoá không thể đúng cho cả hai. Chỗ khác trong repo có
cùng hình dạng — `screen::CALIBRATED_LAYOUTS` keyed theo lớp máy — nên nếu thêm nhãn
mới, hỏi trước: **cái này vỡ khi đổi ngôn ngữ, hay khi đổi bản app?**

### 9.14 `TargetDriver`: một refactor và một lỗi nó tự gây ra (11/08/2026)

Interaction Android chạy qua app bằng cách đưa **ba** bước phụ thuộc thiết bị vào
`TargetDriver` (`apps/desktop/src-tauri/src/interaction_target.rs`):
`open_target`, `send_root`, `send_reply`. `PixelTargetDriver` bọc code iOS **nguyên
văn**; `HierarchyTargetDriver` gọi `crate::interaction_hierarchy`. Chọn **một lần mỗi
assignment** theo `session.supports_element_bounds()`.

Vì sao là trait chứ không phải `if` trong hai hàm send: nhánh **không phải hai hàm mà
là sáu**, và `open_target_confirmed` một mình đã giết lượt chạy Android trước khi tới
hàm send nào.

**LỖI TÔI TỰ GÂY RA, và phải hiểu nó trước khi sửa file này lần nữa.**

Bản refactor đầu tiên dời cả cụm "đi tìm comment cha" (mở drawer, cuộn, khớp hàng)
từ **trên** dòng `effect_intent = true` xuống **trong** `driver.send_reply`, tức là
**dưới** nó. Mọi bước đó có thể fail mà **chưa gõ chữ nào và chưa bấm Gửi** — thường
gặp nhất là cha không có trong danh sách, vì mỗi reply gửi từ máy khác và TikTok xếp
lại hạng giữa các lần. Hậu quả:

`effect_intent == true` → nhánh lỗi ghi `Uncertain` → `retryable_assignments`
**loại `Uncertain`** → `interaction_retry` trả `RetryNotAllowed`. **Một message chưa
bao giờ được đăng thành không thể gửi lại, vĩnh viễn.** Trước refactor nó là `Failed`
và retry được.

Tệ hơn ở nhánh hierarchy: `ReplyRefusal` có doc ghi rõ *"Every variant means nothing
was typed"* và có test cho từng variant, nhưng call site **gộp hết** vào cùng một
`anyhow::bail!` với `NotConfirmed` — đúng cái verdict duy nhất **không được** retry.

Và comment tôi viết trong cùng commit đó nói **ngược lại** với code
(*"everything above this point failed with nothing posted, and must stay
retryable"*) — đúng loại lỗi repo này gọi là "compiling is not evidence".

**Cách sửa (giữ nguyên hình dạng này):** trait trả `Result<SendOutcome, SendFailure>`
với `SendFailure::{BeforeEffect, AfterEffect}`. Chỉ **driver** biết nó đã làm gì, nên
chính nó phân loại; caller đặt `effect_intent = failure.effect_may_have_gone_out()`
**sau** khi gọi, không phải trước.

- `PixelTargetDriver`: ba bước định vị → `BeforeEffect`; `send_prepared_thread_*` →
  `AfterEffect` (giữ **đúng** hành vi iOS trước đây, vì hàm đó cũng có thể fail trước
  khi bấm Gửi và đổi hướng đó là đổi hành vi iOS).
- `HierarchyTargetDriver`: phân loại theo `CommentVerdict` — **chỉ `NotConfirmed` là
  `AfterEffect`**, vì hợp đồng của enum đó ghi mọi variant khác nghĩa là chưa đăng gì.
  Mọi `ReplyRefusal` → `BeforeEffect`.

**Luật rút ra, rộng hơn cái bug:** `effect_intent` không phải một cờ tiện tay, nó là
**ranh giới** giữa "retry được" và "retry sẽ đăng hai lần". Dời code qua ranh giới đó
là đổi ngữ nghĩa dữ liệu, kể cả khi diff trông như chỉ di chuyển hàm. Ai chạm
`execute_thread_campaign` phải hỏi: **bước này có thể fail mà chưa có tác dụng gì
không?** Nếu có, nó phải nằm ở phía `BeforeEffect`.

**Ba lỗi khác cùng lượt review đó, đã sửa:**

- **`await_composer` chờ sai điều kiện.** Nó chờ "có `EditText` nào text không rỗng"
  — nhưng ô nhập **đã** mang placeholder `Thêm bình luận...` **trước** khi bấm Trả
  lời, nên điều kiện đó đúng ngay từ đầu: hàm trả về **tức thì** với hint của drawer
  gốc, rồi phép kiểm tên tác giả so với chuỗi sai. Sửa: đọc placeholder **trước** khi
  tap, rồi chờ nó **đổi**. Có regression test.
- **`starts_with` không có dấu `/`.** `riviu-req1-<sha>` là tiền tố của
  `riviu-req1x-<sha>`, nên cleanup campaign thứ nhất sẽ tìm thấy — và xoá — row của
  campaign thứ hai.
- **`cleanup` nhận id nào cũng chạy.** "Shell-safe" không phải "của mình":
  `Camera`, `Screenshots`, `DCIM` đều shell-safe, và `cleanup` `rm -rf` thư mục mà id
  đó trỏ tới. Giờ đòi tiền tố `riviu-`.

Cộng hai chỗ comment nói quá đã sửa cho khớp code: `rows.sort_by(_data)` **không**
quyết thứ tự carousel (picker xếp mới-nhất-trước, thứ tự thật do thứ tự *tap* quyết,
và đường post chưa có), và `choose_target_driver` **không** cross-check
`reports_element_bounds` với session.

### 9.15 `open_url` mở link TikTok vào HỘP THOẠI CHỌN APP, không vào TikTok (11/08/2026)

Đo bằng `cmd package resolve-activity` trên Redmi Note 12. Đây là intent mà
`UiSession::open_url` dựng **trước khi sửa**:

```
resolve-activity -a VIEW -d 'https://www.tiktok.com/@x/video/123'
  -> com.android.intentresolver.ResolverActivity        ← hộp thoại chọn app
resolve-activity -a VIEW -c BROWSABLE -d '<url>' com.ss.android.ugc.trill
  -> com.ss.android.ugc.aweme.deeplink.AppLinkHandlerV2  ← đúng chỗ
```

Vì sao: `pm get-app-links` cho thấy `www.tiktok.com` **đã verified cho TikTok**, nhưng
**Chrome cũng nhận** cùng URL. Hai app cùng khớp → Android trả `ResolverActivity`. Nên
link **không bao giờ tới bài viết**; nó tới một dialog. Đây chính là thất bại
`ArrivalRefusal::WrongApp` được viết ra để bắt — nhưng không để nó xảy ra thì tốt hơn
là phát hiện sau.

**Sửa:** thêm `UiSession::open_url_in_app(url, bundle_id)`, default delegate sang
`open_url` (đúng cho backend chỉ có một handler). Android override bằng
`am start -a VIEW -c android.intent.category.BROWSABLE -d <url> -p <package>`.
`BROWSABLE` cần vì đó là category mà intent filter của app link khai báo. Package
được `validate_package_name` như mọi chuỗi khác vào `adb shell`.

Verify trên máy sau khi sửa: logcat ghi
`START ... cmp=com.ss.android.ugc.trill/...deeplink.AppLinkHandlerV2` — không còn
chooser.

**Bẫy thứ hai, tốn của tôi bốn lượt đo:** một link **bài không truy cập được** trông
y hệt một link hỏng ở đường code. TikTok nhận intent, resolve bài trên server, thất
bại, rồi **âm thầm rơi về feed**. Trên màn hình đó vẫn có `Đọc hoặc viết bình luận`,
nên `Comments` một mình sẽ coi là "đã tới bài" và campaign sẽ bình luận vào **video
nào đang phát trên feed**.

Ba dấu hiệu phân biệt được, đo được:
- `Đề xuất` **vẫn có mặt** → predicate `Comments && !FeedTab` từ chối. **Đây là lúc nó
  cứu.**
- Số bình luận **đổi giữa các lần chạy** (`7.613` → `9` → `4.324`) — feed đang chạy,
  không phải một trang đứng yên.
- `Follow <tên>` là tên **người khác** với handle trong URL.

Kiểm link còn sống từ host, đừng đoán trên máy: `curl -L <url>` rồi tìm
`Video currently unavailable` và sự **vắng mặt** của `"uniqueId"`/`"nickname"` trong
HTML. Link đã đo (`@user52722048530408/photo/7668965946924666113`) trả HTTP **200** kèm
`<title>TikTok - Make Your Day</title>` — nên **status code không phải bằng chứng**,
cùng họ với `result=0` và `content ... exit 0`.

#### ĐÃ ĐO trên link còn sống: `!FeedTab` là mệnh đề SAI, đã bỏ

Ba link thật (`@mongquynh.dalat`, `@n.sp.i.hoang`, `@huongthao.dalat`), Redmi Note 12,
11/08/2026. **Bài mở từ link không phải một trang riêng** — TikTok render nó thành
**card hiện tại của feed pager**: hàng tab trên (`Đề xuất` vẫn sáng) và tab bar dưới
đều còn trên màn. Nên **không có khác biệt cấu trúc nào** giữa "bài đích" và "video
đang phát".

Bản đầu của `open_target_by_hierarchy` đòi `Comments && !FeedTab`. Nó sẽ **từ chối mọi
arrival thật**. Đo được `feed tab present` = `false` ở một lần và `true` ở hai lần khác
trên **cùng** đường đi, nên nó cũng không dùng được theo chiều nào.

**Thay bằng: rail có, và bài trên màn ĐÃ ĐỔI.** Đọc nhãn tác giả *trước* khi mở link
rồi đòi nó khác. Đây là bản hierarchy của phép so frame-SHA bên pixel, và nó bắt đúng
ca link chết đã đo (tác giả giữ nguyên `Follow Bích Vân` qua bốn lần thử).

Kèm một mức mạnh hơn khi may: nickname có tiết lộ handle không. Đo trên bốn account —
**hai khớp, hai không**:

| handle | nickname | khớp |
|---|---|---|
| `mongquynh.dalat` | `Mộng Quỳnh` | **có** |
| `huongthao.dalat` | `Hương Thảo` | **có** |
| `n.sp.i.hoang` | `Ăn Sập Đi Hoang` | không |
| `nguyenvantoan8584` | `Lúc này lúc kia` | không |

Hàng thứ ba đáng nhớ: handle là **bộ xương phụ âm** của nickname (`Ăn Sập Đi` →
`n.sp.i`), gấp dấu không phục hồi được. Nên nó **nâng** mức bằng chứng khi hit, **không
bao giờ** làm điều kiện. Và phải so theo **cụm từ liên tiếp**, không phải cả chuỗi:
nhãn là `Follow <nickname>` nên squash cả chuỗi ra `followmongquynh`, không nằm trong
gì cả.

Độ trễ rail đo được: **1272 / 2178 / 2460 / 2653 ms** khi app đã warm trên feed.
`ARRIVAL_WINDOW = 14 s` thừa sức.

#### GATE H4 PASSED (11/08/2026) — Standalone Interaction chạy thật trên Android

Qua đúng hàm shipped (`probe --gate-standalone`), không phải bản chép lại:

```
arrival: Identified (Follow Mộng Quỳnh) in 2761 ms
verdict = Sent (đã gửi) in 13376 ms
read back from the open list: author="Mítt zới còiii"
                              text="lịch trình chi tiết quá, lưu lại"
                              locator=android-hierarchy-v1
```

Xác nhận **bằng mắt** trên máy: bình luận nằm đúng bài, `1 giây` trước, drawer từ
`3 bình luận` lên `4`, và **drawer để mở** — đúng thứ `publish_evidence_frame` cần.

Nên chuỗi arrival → armed → disarm → đọc lại đã chạy trọn trên Android.

### 9.16 GATE H5 PASSED — reply gắn đúng cha, hai máy thật (11/08/2026)

`threaded_gate` (example mới): máy A đăng comment gốc, máy B **mở link độc lập**, định
vị comment của A bằng chuỗi A tự gõ, bấm `Trả lời` của **chính hàng đó**, gửi. Redmi
Note 12 = A, SM-N950F = B.

```
A: arrival Structural 6836 ms → verdict Sent
   parent identity: author="Mítt zới còiii" text="set này gọn mà đủ ghê"
B: arrival Structural 5765 ms → verdict Sent
   read back: author="Hoàng Hồng Nam" text="đúng ý mình luôn, lưu lại"
```

**Xác nhận bằng mắt** (bắt buộc — reply gắn sai cha cho ra log y hệt): drawer mở, hàng
của B **thụt vào** so với hàng của A (avatar/text/`Trả lời` đều lệch phải), header đọc
`2 bình luận`. Đây là phép thử duy nhất chứng minh nesting.

Gate này tìm ra **hai lỗi thật**, cả hai đã sửa:

**1. Nhãn `Follow` khớp cả tab "Following".** Catalog ghi `Contains("Follow")`, và
`descriptionContains` của uiautomator **không phân biệt hoa thường** — nên nó khớp
`content-desc="Đã follow"`, tức **tab feed**. Hậu quả kép: đọc tên tác giả ra
`Đã follow` (làm B từ chối một arrival thật), và **hành động follow của nurture sẽ bấm
vào tab** rồi đổi feed thay vì follow ai. Sửa: `Contains("Follow ")` — **dấu cách cuối
là load-bearing**. Mọi giá trị đo được đều là `Follow <tên>`. Có test khẳng định nhãn
không khớp `Đã follow`/`Following` mà vẫn khớp cả ba tên tác giả đã đo.

**2. Sau khi Send, `Back` đóng CẢ DRAWER, không chỉ composer.** §9.7 đo "Back từ
composer → về danh sách, drawer vẫn mở" — nhưng đó là đo **trước** Send. Sau Send,
composer đã tự thu lại, nên cú Back thừa thoát luôn drawer: đọc lại reply thất bại, và
drawer-để-mở mà `publish_evidence_frame` cần thì mất. Sửa: **hỏi trước rồi mới Back** —
placeholder chính là trạng thái (trong composer nó nêu tên cha, ở danh sách nó là hint
chung). Chỉ composer mới được Back.

**Bài học chung:** một phép đo lấy ở trạng thái A không dùng được cho trạng thái B, kể
cả khi cùng một control. §9.7 hoàn toàn đúng cho *trước* Send. Ai dựa vào một dòng
trong tài liệu này phải hỏi: **đo ở thời điểm nào?**

### 9.17 Link nào mở được: chỉ máy trả lời được, host thì không (11/08/2026)

`target_check` (example mới, chỉ-đọc) chạy `open_target_by_hierarchy` trên một danh
sách link. Trên 12 link thật: **4 mở được, 8 không.**

| verdict | số | nghĩa |
|---|---|---|
| `ARRIVED` (Structural) | 4 | campaign dùng được |
| `target_open_screen_unchanged` | 7 | bài xoá/riêng tư/chặn vùng — TikTok nhận intent rồi để nguyên feed |
| `target_open_no_post_page` | 1 | `http://tiktok.com/...` (không `https`, không `www`) — **fail sau 684 ms** |

Ca cuối đáng ghi: intent filter của TikTok chỉ nhận `https` + đúng danh sách host
(`pm get-app-links`), nên `http://tiktok.com/...` không ai nhận, `am start -p` không
resolve được, và nó **fail nhanh và khác** với ca bài chết. Hai mã lỗi khác nhau là
đúng: một cái sửa được bằng chuẩn hoá URL, cái kia thì không.

**Đừng kiểm link từ host.** `curl -L` trả `200` + `<title>TikTok - Make Your Day</title>`
cho **cả** bài sống và bài chết, phục vụ captcha shell thay vì dữ liệu bài, và báo
`Video currently unavailable` cho một bài **mở hoàn hảo trên máy**. Máy là thẩm quyền
duy nhất, và `target_check` là cách hỏi nó.

### 9.4 `platform` + `os_version` (11/08/2026) — và ba chỗ cố ý KHÔNG đổi

`DeviceInfo` giờ có `platform: DevicePlatform` (`Ios` | `Android`) và
`os_version`. `DevicePlatform` **cố ý không có `Default`** và không
`#[serde(default)]`: default hợp lý duy nhất là `Ios`, đúng cái bug field này sinh
ra để diệt. Backend không trả lời được thì phải fail compile; payload thiếu khoá
thì phải fail decode (có test cho cả hai).

**Từng driver tự đóng dấu, không phải `MultiplexDriver`.** `backend_name` có sẵn
nhưng thứ duy nhất để đóng dấu *từ* đó là `backend.name` — một `String` mà doc của
nó ghi rõ là "human-readable name for the operator". Biến chuỗi hiển thị thành
load-bearing, và nhánh `_ =>` không có câu trả lời toàn phần. Thêm nữa
`refresh_device` cũng trả `DeviceInfo`, và `examples/` + `bin/` dựng driver trực
tiếp không qua multiplexer. Rule ở `driver_multiplex.rs` (*route chỉ từ
`list_devices`, không bao giờ đoán nền tảng từ udid*) giữ nguyên.

**Ba chỗ cố ý không đổi — đừng "dọn" chúng:**

1. **`flow/model.rs` — khoá literal `"iosVersion"` trong `json!` dựng hash.** Đó là
   hash material, không phải tên field. SHA-256 đó là
   `ImageCoordinateTarget::profile_id`, **persist** trong `flow_revisions` và
   `flow_node_attempts.canonical_input_json`, và `flow::executor` từ chối chạy node
   khi profile id lệch preflight. Đổi khoá = mọi flow đã lưu có image-coordinate tap
   bắt đầu fail. Golden hash trong test `crate::flow` là chốt: **nó dịch nghĩa là
   khối đó đã bị sửa.**
2. **`DeviceCapabilitySnapshot`: field Rust là `os_version`, khoá serde đóng băng ở
   `iosVersion`.** Nó persist trong `flow_device_runs.capability_snapshot_json` dưới
   `deny_unknown_fields` → đổi khoá làm mọi row cũ fail decode, lỗi cứng chứ không
   phải default. Migration để dịch khoá còn phải tính lại mọi `profile_id` đã lưu
   trong cùng transaction — hiểm hoạ toàn vẹn dữ liệu để lấy 0. **Migration count
   giữ ở 6**, và có test khẳng định điều đó.
3. **Wire sidecar và `iosMin/MaxInclusive`.** `riviu_pmd.py` là iOS-only —
   `lockdown.product_version` **thật là** phiên bản iOS, tên đúng chứ không phải
   misnomer — và `InteractionInspection` là `deny_unknown_fields` với sidecar
   resolve lúc runtime, nên binary mới có thể gặp bản cũ. `iosMinInclusive` nằm
   trong `required` **và** `pattern` của JSON Schema với producer Python. Ranh giới
   rename là **đúng một dòng**: `os_version: response.ios_version` ở `pmd.rs`.

**Mirror TS là viết tay, không có codegen** (không ts-rs/specta). Nên rename thuần
Rust compile sạch cả hai phía và render ra `undefined`. Hai fixture `tsc` **không**
thấy: `apps/desktop/e2e/` nằm ngoài `include: ["src"]` của `tsconfig.app.json`, và
`InteractionPopup.test.tsx` có `as never[]` vô hiệu hoá kiểm assignability. Cả hai
đã sửa tay. `types.ts` giữ `DeviceCapabilitySnapshot.iosVersion` **có chủ ý** —
mirror phản chiếu *wire*, không phải tên field Rust.

**Gate UI theo platform.** Backup/Restore: `disabled` + tooltip **và** early-return
trong handler (`disabled` là affordance, không phải guard); không ẩn, vì hai nút
nằm trong hàng `<nav>` 5 icon và ẩn đi làm hàng co lại theo từng máy. Interaction
actor picker: **lọc**, không disable tại chỗ — cùng danh sách nuôi checkbox,
auto-select `slice(0,6)` và số đếm "N thiết bị", nên để máy không đủ điều kiện nằm
lại sẽ báo quá fleet dùng được và bắn "Chọn từ 2 đến 6" khi đang thấy sáu máy;
backend **không** lọc actor theo platform nên gate UI là gate duy nhất, có test
regression. Publish: **từ chối trước khi dispatch**, không âm thầm bỏ máy — mapping
bundle→máy theo **vị trí** (`targets[index]`) nên bỏ một máy sẽ lệch index và đăng
sai caption cho sai tài khoản.

**Hoãn có lý do: `wda_ready`, `wda_expires_at`.** `wda_ready` đúng là misnomer
(Android nạp từ `agent_ready()`), nhưng **không consumer nào render chữ "WDA"** —
tất cả đọc nó như boolean — nên khác `iOS {version}`, nó không sinh chuỗi sai cho
người dùng. Rename sẽ chạm lại đúng 21 chỗ dựng `DeviceInfo` mà thay đổi này đã
chạm, làm diff to gấp ba và không revert độc lập được. Để commit riêng; tên gợi ý
`agent_ready`. `wda_expires_at` Android giữ `None` và vòng cảnh báo guard bằng
`if let Some` → không có defect sống.

**Còn nợ, đừng nhầm là đã xong**: Chưa máy nào trong fleet có root (`su -c id` không
trả `uid=0`, không có Magisk/SuperSU/KernelSU), nên toàn bộ tính năng cần root
vẫn chưa đo được. `backup_device`/`restore_device` trên Android **không phải việc
hoãn mà là không có đường**: `adb backup` đã bị bỏ trên Android hiện đại và không
có tương đương Mobilebackup2 — việc đúng là gate UI, đừng đi tìm lại.

## 10. Mở đường cho thiết bị mới (09/08/2026)

Mục tiêu là **kiến trúc nhận thêm được lớp máy mới**, không phải hiệu chỉnh
thêm máy. Hiện vẫn **chỉ iPhone 8 được hiệu chỉnh**, và điều đó phải hiển nhiên
trong code chứ không nằm rải rác thành hằng số.

**Lỗ hổng đã bịt, đừng để mở lại.** `nurture` **chưa bao giờ** kiểm hình học.
Registry qualification chỉ gác đường Flow/Interaction (`device_control.rs`
`negotiate`); còn `nurture::run_session` đi thẳng từ `window_size()` sang nhân
phân số iPhone 8 — nên **một máy kích thước khác sẽ bị chạm bằng toạ độ iPhone
8**, đúng thứ §691-692 cấm. Tệ hơn, khi không đọc được kích thước nó rơi về
`unwrap_or((375.0, 667.0))`, tức **bịa ra một màn hình**. Giờ cả hai đều là từ
chối có lý do.

- `screen::CALIBRATED_LAYOUTS` là danh sách **lớp màn hình đã đo thật**, hiện
  đúng một entry `iphone8-portrait-v1`. `screen::calibrated_layout()` trả `None`
  cho mọi thứ khác, và `None` **phải** nghĩa là từ chối.
- **Đây là danh sách khác với registry qualification.** Registry (rỗng, ở
  `sidecars/wda/interaction-capabilities.json`) nói máy nào được thương lượng
  năng lực; `CALIBRATED_LAYOUTS` nói bộ dò đã được đo trên màn hình nào. Cái này
  không suy ra cái kia.
- `QualifiedGeometry::validate()` **không còn** so bằng đúng `375.0/667.0`. Nó
  chỉ kiểm nhất quán nội tại (hữu hạn, dương, `logical*scale == pixel`,
  portrait). Bỏ được vì đó là kiểm **entry tĩnh của registry**, còn máy sống vào
  được là nhờ `matches()` so **bằng đúng** với một entry — registry mới là
  allowlist. Trước đây nó khiến registry **không thể chứa nổi lớp máy thứ hai**.

**Vì sao hằng số `screen.rs` không suy ra bằng công thức được.** Chúng là phân
số neo vào khoảng cách điểm **cố định tính từ mép màn**, không phải tỉ lệ của
toàn màn. `COMMENT_INPUT.1 = 640/667` là 27pt từ đáy trên iPhone 8, nhưng nhân
lên màn 844pt thành 35pt từ đáy — sai trước cả khi tính 34pt home indicator mà
iPhone 8 không có. **Thêm một lớp máy = đo lại vật lý theo mục 6, không phải
chia lại.** Và hiện `QualifiedGeometry` **không có trường safe-area nào cả**.

**Trỏ tới `adb`:** `RIVIU_ADB_PATH` chỉ thẳng vào file thực thi, ưu tiên trước
`ANDROID_SDK_ROOT`/`ANDROID_HOME` rồi mới tới `PATH`. Cần thiết vì một máy có
thể có platform-tools giải nén rời, không nằm trong layout SDK — khi đó không có
cách nào khai báo vị trí. Backend Android chỉ tham gia fleet khi `adb version`
chạy được (`detect_driver`); nếu không, lý do nằm ở `android_unavailable_reason`,
**tách riêng** với `driver_degraded_reason` của sidecar iOS.

**Kênh gõ chữ iOS không còn phụ thuộc IPA bên thứ ba.** Xem mục 13:
`RiviuAgent-text.ipa` là bản tự build từ WebDriverAgent, đã có `text` với bằng
chứng frame thật; `RiviuAgent.ipa` (RT-MMO `com.mrph.svc`) chỉ còn là rollback
oracle. Ràng buộc thật là **free provisioning 7 ngày** và profile nhúng chỉ có
hai UDID test — fleet 20 máy cần tài khoản Apple Developer trả phí.

### 9.18 Nurture Android chạy được qua app — và hai lỗi chỉ có máy thật mới thấy (12/08/2026)

**minicap: APK ở đâu ra, và số đo trên cả hai máy.** Đường stream Android là
minicap **noarch APK** chạy bằng `CLASSPATH=<apk> app_process`, không phải
scrcpy — quyết định đó cùng lý do đã ghi ở mục 9. APK lấy từ npm package
`@devicefarmer/minicap-prebuilt` (`npm pack`, v2.7.3), file
`prebuilt/noarch/minicap.apk`, 4.209.669 byte, magic `50 4B 03 04`. ~~Đặt ngoài
repo tại `~/.riviu/minicap/noarch/minicap.apk` và trỏ bằng
`RIVIU_MINICAP_APK` (biến User, không phải per-shell).~~ **Câu vừa gạch đã bị
đảo — xem 9.27 ngay dưới.** Không có APK thì `ensure_stream` từ chối và **mọi
thứ phía sau nó sụp theo** — kể cả nurture, vì `open_ui_context` đòi stream.

| Máy | Android | Banner | Số đo |
|---|---|---|---|
| Redmi Note 12 (`23021RAAEG`) | 15 | `real=1080x2400 virtual=540x1200 quirks=2` | **172 frame / 6,00 s = 28,7 FPS**, 70,8 KB/frame |
| SM-N950F Note 8 (`ce0617…`) | 8.0 | `real=1080x2220 virtual=540x1110 quirks=2` | **145 frame / 6,01 s = 24,1 FPS**, 67,8 KB/frame |

Note 8 **lần đầu được đo** ở đây; con số 25,8 FPS trong tài liệu cũ là của
Redmi. Cả hai ≥ `STREAM_FPS = 24`, Note 8 vừa đúng ngưỡng.

**Tile Android lạnh thì KHÔNG ready, và đó là đúng.** `agent_ready` trả `false`
khi `self.forwarded` chưa chứa serial — một `HashSet` **trong process**. App
vừa mở thì set rỗng, nên tile Android là `Parked / No stream` và fleet đếm
`1/3`. Bấm **Start** trên tile là đủ; sau đó forward tồn tại trong adb server
nên lần mở app kế tiếp cả hai tile tự lên `● Live` ngay. Đừng đi tìm lỗi ở
minicap khi thấy `Parked` trên một app vừa khởi động.

**H5 đạt qua app:** chọn Note 8 → `Nuôi TT` → `Bắt đầu`, tile giữ `● Live`
suốt phiên, kết thúc `stopped — 61/68 video, 8 tim, 0 bình luận, 1 follow,
907s (hierarchy)` và tile về `● Live`, fleet vẫn `2/2`. Đây là lần đầu nurture
Android chạy trọn vòng **qua app** thay vì qua example.

#### Lỗi 1 — vòng hierarchy chưa từng đọc lại settings; và đọc lại row cũng chưa đủ

Panel ghi được vào DB (kiểm trực tiếp: `likeProb 100`, `fatigue false`,
`frenzyProb 0`, `followEnabled false`) mà phiên đang chạy **không đổi hành vi**:
16 post liên tiếp sau khi lưu vẫn `lướt`, số tim đứng nguyên. Hai nguyên nhân
xếp lớp, và lớp thứ hai là lớp đắt:

1. `run_feed` nhận `settings: &NurtureSettings` **một lần** và giữ nguyên cả
   phiên. `absorb_live_settings` chỉ được gọi trong vòng pixel. Nên trên
   Android **không một công tắc nào trong panel có tác dụng**.
2. Nghiêm trọng hơn, **cả hai vòng** dựng `HumanBehavior` và
   `HumanSessionPolicy` *trước* vòng lặp. Hai struct đó giữ bản sao riêng:
   `fatigue`/`time_of_day`/`pause_swipe` chỉ đọc trong `HumanBehavior::new`, và
   `HumanSessionPolicy::new` biến xác suất thành **trần theo giờ** (`like_cap`
   8–16, `0` khi prob `0`) mà `can_attempt` đọc `0` là *không bao giờ*. Nên một
   tính năng **tắt lúc khởi động rồi bật giữa phiên là không bao giờ chạy được**,
   im lặng, dù UI hiển thị đã bật.

Điểm cần nhớ: **test unit của `absorb_live_changes` pass hoàn hảo trên cả hai
lỗi này.** Nó kiểm việc sao chép trường, không kiểm việc giá trị có tới chỗ ra
quyết định. Đó chính là "compile được không phải bằng chứng" ở dạng test.

Sửa: `crates/core/src/nurture/live.rs` — trait `LiveSettings` + một hàm
`apply_live_settings` làm cả ba bước (refresh row → `HumanBehavior::retune` →
`HumanSessionPolicy::retune`), **cả hai vòng gọi cùng hàm đó**. `retune` của
`HumanBehavior` là phép gán chứ không dựng lại: `session_start` là mốc đo
fatigue, dựng lại sẽ cho một phiên chạy hai giờ hoá thành vừa nghỉ xong.
`retune` của policy **chỉ tác động lên chuyển trạng thái** — đóng trần khi prob
về 0, mở trần mới khi prob rời 0, còn trần đang mở thì giữ nguyên số, vì
re-roll mỗi post sẽ làm "tối đa 8–16 mỗi giờ" thành vô nghĩa.

`CommentTextSource::comment_for_post` giờ **nhận** `&NurtureSettings` thay vì
implementor giữ một borrow từ lúc khởi phiên — nếu không thì vòng lặp giữ bản
live còn nguồn chữ giữ bản cũ, tức hai câu trả lời cho cùng một câu hỏi.

**Nghiệm thu trên máy thật (Note 8), chọn phép thử không thể hiểu nhầm:** khởi
động với **Thích tắt**, bình luận 0, follow tắt, vuốt nhanh 0 → 9 post đầu
`0/0♥`, không một tương tác nào. Bật **Thích** giữa phiên rồi `Lưu`
(DB xác nhận `likeEnabled True`) → `2/2♥` ở video 27, kết `stopped — 27/36
video, 2 tim, 344s`. Trước bản sửa, cùng thao tác đó cho `0/0♥` mãi mãi.

Test bắt được lỗi: 5 test trong `nurture::live`. Đã kiểm chúng **fail** khi bỏ
hai lời gọi `retune` (3/5 fail, 2 test bất biến vẫn pass đúng như phải vậy) —
một test không fail trước khi sửa thì không phải test.

#### Lỗi 2 — TikTok khởi động lạnh không đọc được foreground trong 5 s

`start_interaction_session` chờ `active_app_bundle()` khớp trong **5 s / 250 ms**.
Với TikTok đã chạy sẵn thì tức thì. Với TikTok **lạnh** trên Note 8 (Android 8),
start thất bại:

```
startInteractionSession failed … com.ss.android.ugc.trill did not reach the
foreground … within 5s; the phone is showing <unreadable: could not read the
foreground package. Tried: `dumpsys window windows | grep mCurrentFocus` had no
mCurrentFocus line …
```

Đo lại ngay sau đó bằng tay: lệnh đó **chạy tốt** và trả
`mCurrentFocus=Window{… com.ss.android.ugc.trill/…splash.SplashActivity}`. Nên
lệnh không sai — cửa sổ 5 s quá ngắn cho một lần mở lạnh, và trong lúc chuyển
cảnh launcher→app `dumpsys window windows` **có lúc không có dòng
`mCurrentFocus` nào cả**. Chưa sửa; ghi lại để không ai đi thay lệnh dumpsys.
Thêm nữa, ngay sau khi mở lạnh, TikTok có thể ở thẻ không có action rail: một
lần chạy kết `partial — 0/0 video, 14s` vì `OFF_FEED_LIMIT` = 6 lượt liên tiếp
không thấy tab feed. Lần chạy lại sau khi app đã vào feed thì bình thường.

#### `GIỚI HẠN VIDEO` và `VÒNG` không có tác dụng trên đường app

`NurturePopup.tsx:295` gọi `nurtureStart(targets)` **không truyền duration**, và
`nurture_commands.rs:284-288` khi đó gán một horizon **2–3 giờ ngẫu nhiên** (có
chủ ý: để các phiên không cùng kết ở một số video). `max_duration.is_some()` làm
`total_videos = u32::MAX`, nên hai ô đó chỉ còn tác dụng cho fixture/test. Đo
được: đặt `GIỚI HẠN VIDEO = 15` rồi chạy, phiên đi tới video **68** và **36**
trong hai lần. Hai ô này đang hiển thị kèm badge `cần chạy lại` như thể chúng
chặn phiên. Chưa sửa — đổi cái gì bị chặn phiên là quyết định về hành vi, không
phải sửa lỗi.

### 9.19 Ba chỗ ở §9.18 đã sửa, kèm số đo (12/08/2026)

**1. Cửa sổ chờ foreground: 5 s → 40 s.** Đo trên SM-N950F, ba lần từ
`am force-stop com.ss.android.ugc.trill`: TikTok lên foreground sau
**15,86 / 19,71 / 19,42 s**, một lần thứ tư với chu kỳ poll thưa hơn cho **26,9 s**.
Nên 5 s sai gấp bốn lần, và hệ quả người vận hành thấy là
`did not reach the foreground … <unreadable>` cho một máy đang mở TikTok bình thường.
40 s = lần chậm nhất đo được cộng biên, trên máy cũ nhất trong fleet. Vẫn là
**deadline chứ không phải fallback**: màn hình khoá làm `monkey` báo thành công mà
không có gì chuyển động, và cái đó phải kết thúc bằng từ chối.

**2. `active_app_bundle`: hỏi dạng lệnh chạy được trên cả hai máy trước.** Đo cả ba dạng,
và chúng chia đôi hoàn toàn:

| Lệnh | Note 8 / Android 8 | Redmi Note 12 / Android 15 |
|---|---|---|
| `dumpsys window windows \| grep mCurrentFocus` | chạy, 88–148 ms | **luôn rỗng** |
| `dumpsys window displays \| grep mCurrentFocus` | **luôn rỗng** | chạy, 105–107 ms |
| `dumpsys window \| grep mCurrentFocus` | chạy, 84–97 ms | chạy, 129–172 ms |

Dạng không có subcommand là dạng duy nhất trả lời được cả hai, giá tương đương (grep
chạy trên máy nên chỉ một dòng về host). Đưa nó lên đầu; hai dạng kia giữ làm fallback.
Trước đó `windows` đứng đầu nên **mọi** lần gọi trên máy Android 15 tốn một round trip
vô ích 122–167 ms.

Kèm một phát hiện quan trọng hơn: ngay sau `launch_app`, có những giây mà **cả ba** lệnh
đều không có dòng `mCurrentFocus` — probe bắt được đúng lúc đó và in
`Tried: … had no mCurrentFocus line; … had no mCurrentFocus line; …`. Nên
`active_app_bundle` trả lỗi trong lúc chuyển launcher→app là **trạng thái tạm bình
thường**, không phải lệnh sai. Vòng poll của `start_interaction_session` phải chịu được
nó, và đó là lý do thứ hai để cửa sổ đủ dài.

**3. Chờ feed lên trước khi vào vòng lặp.** Đo: sau `am force-stop`, tab feed `Đề xuất`
đọc được **23,8 s** sau intent, còn package lên foreground ở 16–27 s — hai mốc cách nhau
vài giây, khoảng giữa là màn splash. Hệ quả cũ: một phiên khởi động ngay sau lần mở lạnh
báo `partial — 0/0 video, 14s`, vì nhánh off-feed đốt hết `OFF_FEED_LIMIT` = 6 cú vuốt
vào màn splash trong 14 s. Nhánh off-feed đúng cho thẻ quảng cáo / LIVE / đang chuyển; nó
sai cho một app chưa khởi động xong. `await_feed` chờ tối đa 30 s rồi **từ chối có lý
do** — một máy đang ở trang chọn chủ đề thì không bao giờ có feed, và nói ra hơn là vuốt
mù sáu lần.

**4. `GIỚI HẠN VIDEO` / `VÒNG` giờ chặn thật.** `video_target` trong
`nurture/live.rs` là nguồn duy nhất cho cả hai vòng, và không còn nhánh
`if max_duration.is_some() { u32::MAX }`. Cả hai giới hạn cùng áp: **cái nào tới trước thì
dừng**. Thời lượng vẫn giữ đúng việc của nó — trần chặn một phiên bị bỏ quên, và giá trị
mặc định ngẫu nhiên 2–3 giờ vẫn khiến hai máy bấm cùng lúc không dừng cùng lúc.
Nghiệm thu qua app: đặt 5, chạy → `done — 3/5 video, 0 tim, 36s (hierarchy)`. Cùng cấu
hình đó trước bản sửa cho 68 và 36 video.

**BẪY: `adb shell uiautomator dump` giết agent mà không giết process.** Trong lúc đo mốc
feed tôi poll `uiautomator dump` hơn hai chục lần. Cả nó và
`appium-uiautomator2-server` đều cần `UiAutomation`, mà chỉ một bên giữ được — server
**vẫn sống**, `/status` vẫn trả lời, nên `agent_ready` vẫn báo ready, nhưng **mọi truy vấn
element treo**. Biểu hiện: `await_feed` hết 30 s trong khi tile đang hiện feed rõ ràng.
Phục hồi: `am force-stop io.appium.uiautomator2.server{,.test}` rồi để `open_session`
instrument lại (đo được 4040 ms). **Không dùng `uiautomator dump` để đo trên máy mà app
đang điều khiển** — dùng chính agent qua probe.

### 9.20 Vuốt ngang bài ảnh trên Android — và ba kết luận sai phải sửa để làm được (12/08/2026)

Tính năng này đã hứa trong UI (`Vuốt ngang` + phần trăm) mà **trên Android chưa có một
dòng nào**. Lý do nó bị bỏ là một kết luận sai tôi tự ghi vào code:
`carousel_portion_percent` nói *"a dump of every TextView on a photo post contains no
`1 / 7` counter (measured 11/08/2026)"*. Bộ đếm **có**, nó chỉ **bị tách thành ba node
rời**: `"1"`, `" / "`, `"5"`. Tìm một node chứa dấu gạch thì không thấy gì.

**Đo lại, trên hai bài ảnh thật của người dùng, SM-N950F:**

| Bề mặt | Bộ đếm | Hành vi |
|---|---|---|
| Trang bài mở từ link | 3 node `TextView` bền vững | `1 / 5` → `2 / 5` → … → `5 / 5`, rồi **mất** sau ảnh cuối |
| Feed | node `" / "` **không phải TextView**, `content-desc` rỗng | chữ số chỉ **xuất hiện từ cú vuốt đầu**: `+ "2" + " / " + "7"` |

`ImageView` **không đổi** suốt quá trình (22 hình chữ nhật đứng yên), nhãn `Comments`
cũng không đổi — đó là cách biết vẫn còn ở cùng một bài. Nên hình học không phải tín
hiệu; bộ đếm mới là.

**Sai lần hai: lấy bộ đếm làm gate.** Hai lần chạy 30 video không duyệt được bài ảnh nào.
Bộ đếm ở góc trên phải là **overlay tạm thời** — đo được 3/14 thẻ có, rồi 0/14 thẻ có
trên cùng loại feed — nó mờ đi sau khi thẻ vừa tới, còn vòng lặp tới đó sau khi đã xem
1–2 s và tương tác xong. Gate đọc bộ đếm gần như không bao giờ nổ.

**Sai lần ba: nghĩ thẻ bài ảnh không có action rail.** Giả thuyết "vòng lặp coi nó là thẻ
LIVE rồi bỏ qua" — đo ra `Comments=CÓ` trên **14/14** thẻ, kể cả thẻ bài ảnh. Sai.

**Cái đúng: nhãn `Ảnh`.** Nó nằm cạnh caption và **bền**; thẻ 10 trong một lượt đo có
`Ảnh` mà không có bộ đếm. Nên nó vào catalog như mọi nhãn khác —
`TikTokControl::PhotoBadge`, `LabelMatch::Text("Ảnh")`, khớp trên `text` vì node này
**không có `content-desc`**. Là bản dịch nên fail-closed theo ngôn ngữ: build `en` để
`None` và **không vuốt ngang gì cả**. Đó là hướng an toàn đúng, vì **vuốt ngang trên thẻ
video là cử chỉ mở trang tác giả của TikTok** — gate sai một lần là phiên đi khỏi feed.

**Hình dạng cuối, theo đúng thứ tự hai bề mặt thật sự làm việc:**

1. Gate bằng nhãn `Ảnh` — bền, có xuất xứ, fail-closed.
2. Vuốt lần đầu **trước khi biết tổng số ảnh**, vì trên feed chữ số chỉ hiện khi bắt đầu
   lật. Tổng về cùng cú vuốt đầu, rồi phần trăm áp lên **tổng thật của bài**.
3. Mỗi lượt lật phải **chứng minh được** bằng bộ đếm nhích lên. Bộ đếm đọc được mà không
   nhích = hết bài, dừng.
4. Bộ đếm **không đọc được** thì *không phải* hết bài — coi vậy làm một bài 15 ảnh dừng ở
   `1/15`. Cho phép tối đa `CAROUSEL_UNPROVEN_LIMIT = 3` lượt liên tiếp không chứng minh
   được rồi dừng.
5. Dừng ở `current == total`, không vuốt tới khi bộ đếm mất: đo được là nó chỉ mất **sau**
   ảnh cuối, nên chờ điều đó là tốn thêm một cử chỉ quá cuối bài.

**Nghiệm thu qua `run_hierarchy_session` (code đã ship), 20 video:**

```
gặp bài ảnh — vuốt ngang        →  bài ảnh: đã xem 8/8 ảnh
gặp bài ảnh — vuốt ngang        →  bài ảnh 11 ảnh — xem 11 (100%)  →  đã xem 11/11 ảnh
gặp bài ảnh — vuốt ngang        →  đã xem 4 ảnh (bản build không hiện số ảnh)
```

**Hai nền tảng giờ khác nhau và phải nói thẳng:** trên Android `50%` là nửa **của bài
đó**; trên iOS là nửa **của trần**, vì engine pixel không có bộ đếm để đọc, nó chỉ biết
"frame có đổi không". `NurtureSettings::carousel_ceiling()` (trần thuần) và
`carousel_slide_budget()` (trần đã gấp phần trăm vào) là hai số cho hai đường — dùng lẫn
sẽ áp phần trăm **hai lần**, và tôi đã tự làm đúng lỗi đó trước khi tách ra.

**Sàn trên feed là 2 ảnh** khi tính năng đang bật, vì không đọc được tổng trước cú vuốt
đầu. Phần trăm 1% không giúp một phiên khỏi lật một lần.

### 9.21 Agent còn sống mà cây đã chết: `/status` không phải bằng chứng (12/08/2026)

`ensure_agent` tin `AgentClient::is_alive()`, mà nó gọi `window_size()`. **Đo được:
`window_size` trả `0 ms ok` trên một agent đã mất `UiAutomation`**, trong khi mọi truy vấn
element treo tới hết timeout. `/status` cũng trả lời. Nên `agent_ready` báo ready,
`ensure_agent` tái dùng session, và vòng nurture chờ feed 30 s trong khi feed đang hiện rõ
trên tile.

Ba chỗ sửa:

* `is_alive()` giờ **truy vấn một element** (`FrameLayout`, có trên mọi màn hình, nên agent
  khoẻ trả lời từ node đầu tiên). Locator *vắng* thì sai: nó chờ hết timeout root-node của
  server và làm agent khoẻ trông như đã chết.
* Timeout HTTP **120 s → 30 s**. Truy vấn element chậm nhất từng đo là 10,2–10,5 s (S8+
  dưới feed đang phát), `/source` 3,4 s, và không có gì chậm đi qua client này — push/
  install APK là adb. 120 s không phải giới hạn, nó là treo: `locate` (4 round trip) đứng
  tám phút, đủ giết một lần chạy probe ở mức 600 s.
* `ensure_agent` không còn tin `/status`: session mới **phải tự chứng minh**, không thì
  force-stop cả hai nửa instrumentation rồi khởi động lại (đo bằng tay: `open_session` sau
  đó trả lời trong 4040 ms). Vẫn bị blind sau khi restart thì **báo lỗi nêu tên nguyên
  nhân** thay vì thử lại vô hạn — có thứ khác trên máy đang giữ `UiAutomation`.

Và một chỗ thứ hai của lỗi cửa sổ 5 s ở §9.19: `ensure_tiktok_foreground` trong
`nurture/hierarchy.rs` chờ 10 × 800 ms = **8 s**, nên gate G2 với TikTok đã tắt từ chối
bằng `đã gọi mở … nhưng nó không lên foreground` trong khi app đang mở bình thường. Đưa
lên 40 s, cùng số đo. Sau khi sửa: `đã đưa TikTok lên foreground` ở **16,3 s**.

### 9.22 Toàn quyền: `human_limits` mặc định TẮT (12/08/2026)

Quyết định của người vận hành, sau khi phát hiện tỉ lệ đặt trong panel không phải tỉ lệ
nhận được. `HumanSessionPolicy` giữ **sáu** thứ đè lên số đã đặt:

| Thứ | Giá trị | Hệ quả lên "Thích 100%" |
|---|---|---|
| `can_interact_with_post` | tối đa 2 trong 5 thẻ gần nhất | **Không bao giờ vượt 40% số bài**, bất kể đặt gì |
| `like_cap` | 8–16 / giờ | Dừng ở 8 sau ~15 phút |
| `comment_cap` / `follow_cap` | 1–3 và 1–2 / giờ | |
| `min_action_gap` | 12–35 s sau mỗi hành động | ~2–5 hành động/phút |
| `rest_after_video` | 15–90 s mỗi 7–13 bài | `nghỉ tự nhiên 85s` trong log |
| `should_take_block_break` | nghỉ 20–45 phút | |

Sửa ở **một chỗ**: `HumanSessionPolicy` có field `limits`, và cả sáu cửa trên trả lời nó.
Hơn 20 call site trong hai vòng lặp không đổi một dòng. `NurtureSettings::human_limits`
mặc định `false` (`#[serde(default)]`, nên row cũ đọc ra cũng là tắt), live-tunable, có
công tắc riêng trong panel kèm `!` liệt kê đúng những con số ở trên.

**Cái duy nhất giữ lại khi tắt là một `UNPACED_ACTION_GAP = 800 ms`** — và nó không phải
nhịp, nó là *settle*. Mọi hành động ở đây tự chứng minh bằng cách đọc lại màn hình, nên
bắn cử chỉ kế tiếp vào một màn hình còn đang animate là cách để tap rơi vào thứ trượt vào
dưới nó.

**Nghiệm thu qua `run_hierarchy_session`, Thích 100%, 12 video:**

```
done — 10/12 video, tim 10/10, follow 0/0, 131s
```

**10/10**: mọi bài được thả tim, mọi lượt xác nhận được bằng nhãn đổi trạng thái. Không
một dòng `bỏ qua tim: nhịp phiên hiện tại đã đủ`, không một `nghỉ tự nhiên` nào trong
131 s. Cùng cấu hình đó khi `human_limits` bật cho tối đa ~4/10 bài.

**Giá phải trả, ghi để không ai tưởng đây là nâng cấp thuần:** chính nhịp đó là thứ làm
phiên trông giống người. Tắt đi thì nhanh hơn, dày hơn, và **dễ bị nhận ra hơn**. Người
vận hành đã chọn đánh đổi đó một cách rõ ràng và có thể bật lại bằng một công tắc.

#### Hai chuyện đo được trong lúc nghiệm thu

**`on_feed()` không phân biệt được feed thật với trang bài mở từ link.** §9.16 đã ghi rằng
trang bài deep-link vẫn hiện `Đề xuất`; hệ quả chưa ai ghi là vòng lặp bị **kẹt** ở đó:
`video đã tim từ trước — bỏ qua` bốn lần trên **cùng một thẻ**, rồi
`feed không đổi thẻ sau nhiều lượt vuốt — dừng`. Dưới một bài mở từ link không có thẻ nào
để vuốt tới. Máy bị các phép đo `--measure-carousel` của tôi để lại ở trạng thái đó; một
lần chạy từ TikTok khởi động lạnh thì không hề kẹt. **Chẩn đoán: cùng một thẻ lặp lại +
vuốt không đổi = đang ở trang bài, không phải feed.**

**Dấu vân tay phải lấy lại sau khi lật ảnh.** `before` lấy ở đầu bài, rồi carousel lật cả
chục ảnh, rồi cú vuốt dọc lại bị xử theo dấu vân tay cũ đó. Trước khi sửa: một lần chạy
lật 4 bài ảnh báo `vuốt chưa chứng minh được đổi thẻ` 4 lần và dừng ở `1/5 video` — tính
năng báo feed bị kẹt chính là traversal vừa chạy xong.

**Màn hình khoá cho ra đúng triệu chứng "app không lên foreground".** `monkey` báo thành
công, `dumpsys window` cho `isStatusBarKeyguard=true`, TikTok chạy suốt. Thông báo từ chối
giờ nói thẳng câu đó.

### 9.23 Cử chỉ: đường vuốt thật thay vì một đoạn thẳng (12/08/2026)

Người vận hành nói thao tác chưa giống người, và **điểm chạm** thì đã tốt sẵn:
`TouchPointPlanner` jitter trong đúng hình chữ nhật control, không lặp toạ độ, giữ khoảng
cách với vết chạm gần đây. Chỗ lộ nằm ở ba tính chất khác, cả ba đều **hằng số**:

| Thứ | Trước | Sau |
|---|---|---|
| Đường vuốt | **một** `pointerMove` = đoạn thẳng tuyệt đối | Bézier bậc hai, 12 chân, bow 1,2–4,5% chiều dài, chiều bow ngẫu nhiên từng cử chỉ |
| Vận tốc | không đổi từ pixel đầu tới pixel cuối | smoothstep — tăng, chạy, dịu; thời lượng chia theo profile đó |
| Điểm đầu/cuối | phân số cố định, **cùng hai pixel mọi lần** | jitter ±18 px, clamp trong màn |
| Nhấc ngón | ngay khi hết di chuyển | còn tiếp xúc 12–45 ms |
| Tap | `pause 60` cố định, **không dịch** khi đang chạm | tiếp xúc 45–130 ms + trôi ±2 px dưới tiếp xúc |

Transport không phải rào cản: agent nói **W3C pointer actions**, nhận số `pointerMove` tuỳ
ý với thời lượng riêng từng bước — nên cả đường cong đi trong **một** round trip, y như cũ.
Hình dạng cũ đơn giản là thứ đơn giản nhất mà chạy được.

`SwipePath` là type mới trong `types.rs`; `UiSession::swipe_path` có **default** thu path về
hai đầu, nên iOS và mọi fake session không phải hiện thực gì. Android override nó.
`SwipeGesture` giữ nguyên vì nó được persist trong flow script và mang trong evidence.

`total_ms` vẫn là số của caller: chỉ hình dạng đổi, thời lượng không — nên không có hằng số
nào đã tinh chỉnh phải tinh chỉnh lại.

Test kiểm **tính chất** chứ không kiểm ảnh: đường có rời khỏi dây cung (36/40 lần), chân dài
nhất ≥ 2× chân ngắn nhất, tổng thời lượng đúng bằng yêu cầu, hai lần xin cùng một cử chỉ ra
hai cử chỉ khác nhau, và không điểm nào ra khỏi màn (kiểm cả ba dạng vuốt sát mép).
Nghiệm thu trên máy là "vẫn lái được TikTok": `done — 10/12 video, 89s`.

**Không thể chứng minh "TikTok thấy giống người"** — không ai chứng minh được điều đó từ
đây. Chứng minh được là: những tính chất trước đây **hằng số** thì giờ không còn hằng số.

#### Lớp thứ ba của "toàn quyền": mood multipliers

`human_limits = false` ở §9.22 chưa đủ. Đo được ngay sau đó: một lần chạy 12 video với
`like_prob = 100` ra **`tim 0/0`**, mọi post ghi `(lướt)`. `Mood::Skimming.like_mult()` là
**`0.0`** — không phải giảm, là tắt — và Skimming chiếm ~60% số video. Nên dỡ hết trần vẫn
còn một lớp đè.

Thêm `Mood::Neutral`: mọi multiplier bằng 1.0, kể cả `watch_mult`, nên `xem min/max` cũng là
cửa sổ thật. `MoodCycle::neutral()` được chọn khi `human_limits` tắt, và `MoodCycle::retune`
đổi được giữa phiên theo cả hai chiều vì công tắc là live-tunable. Nhãn của nó trong log là
`theo đúng tỉ lệ đặt`, để đọc log biết ngay đang ở chế độ nào.

Các multiplier theo mood **vẫn đúng cho việc của chúng** — chúng bình quân về 1.0 qua một
phiên dài, giữ cho *cài đặt* trung thực theo giờ. Chúng chỉ không phải thứ một người đặt
"100% nghĩa là mọi bài" đang yêu cầu.

Nghiệm thu: `tim 6/6`, mọi post ghi `theo đúng tỉ lệ đặt`.

### 9.24 Vị trí chạm: cụm có lệch, không phải random đều (12/08/2026)

Người vận hành nói "cảm giác lướt, bấm, vị trí nó random á" — và đúng. §9.23 sửa **đường**
vuốt; chỗ này là **phân bố**, và nó sai theo hướng ngược với trực giác: cái cũ *quá* ngẫu
nhiên.

Ba luật cũ, cộng lại cho một phân bố **đều hơn cả tình cờ**:

* `gen_range` **đều** trên toàn hình chữ nhật của control — góc cực biên được chạm nhiều
  bằng giữa nút. Không ngón tay nào làm vậy.
* `used: HashSet` — **không bao giờ trả lại một toạ độ đã dùng**. Lấy mẫu không hoàn lại
  trên một ô nhỏ tự nó là một dấu hiệu nhận ra được.
* `RECENT_MIN_DISTANCE = 3.0` — mỗi tap phải cách 96 tap gần nhất ≥3 px. Đẩy về blue noise.

Ngón tay thật cho một **cụm**: gần chuẩn tắc, tâm lệch vài pixel theo hướng bàn tay đang
nghiêng, **và có lặp lại**. Nên giờ là:

* chuẩn tắc hai chiều quanh `tâm + bias`, σ = bán kính / 3 (Box–Muller, sáu dòng, không
  thêm dependency);
* `HAND_BIAS`: lệch ±7 px **rút một lần cho mỗi máy** rồi giữ nguyên cả phiên — đó mới là
  "bàn tay", chứ rút lại mỗi tap thì chỉ là thêm nhiễu. Đơn vị pixel, không phải phân số
  của nút: ngón tay không biết nút to bao nhiêu;
* **bỏ hẳn** luật chống lặp. Chạm đúng một pixel hai lần trong năm mươi lần chạm là chuyện
  thật;
* cùng `bias` đó áp cho hai đầu đường vuốt, và jitter ở đó cũng đổi từ đều sang chuẩn tắc.

Luật duy nhất giữ lại là luật chịu lực: **tap không bao giờ ra khỏi hình chữ nhật của
control** — nút tim và nút bình luận cách nhau một khoảng rail, chạm lệch là mở drawer.

**Một lỗi tôi tự tạo khi làm việc này, đáng ghi vì rất dễ lặp:** clamp vào biên `.5` rồi mới
`round()` thì làm điểm nhảy **ra ngoài** biên. Với phép rút đều thì đó là sự kiện xác suất
0; với chuẩn tắc bị clamp thì nó bão hoà đúng vào biên liên tục nên nổ ngay. Thứ tự đúng là
quantize rồi clamp trên biên đã làm tròn.

**Và ba lần test chập chờn, cả ba do tôi viết sai chứ không do code:**

1. Đo độ tụm quanh **tâm hình học** trong khi bàn tay có lệch cố định — phải đo quanh tâm
   của chính mẫu. Fail ~1/6.
2. Khẳng định **hai** số ngẫu nhiên trong [-7,7] cách nhau >0,5 px — tự tạo ~7% fail. Thay
   bằng: đo độ tản của lệch trên **12** planner.
3. Dung sai ±2 px cho trung bình của **400** mẫu với σ = 20 px — sai số chuẩn 1 px, tức chỉ
   2 SE, fail ~1/10. Nâng mẫu lên 4000 (SE 0,32 px) thì cùng dung sai thành 5 SE.

Sau đó: **0 fail / 25 lần chạy**. Một test chập chờn còn tệ hơn không có test, và cả ba lần
lỗi đều là "đo may mắn thay vì đo tính chất".

Nghiệm thu trên máy: `done — 10/12 video, tim 11/11, 120s`. Mọi tap vẫn trúng nút tim và
đều xác nhận được bằng nhãn đổi trạng thái — cụm có lệch không làm giảm độ chính xác.

### 9.25 Nhịp thời gian: bỏ luật chống lặp và một cái lỗ trong histogram (12/08/2026)

Sau §9.24, hai chỗ còn lại mắc **đúng** hai lỗi đó:

**`watch_seconds` có luật chống lặp.** `min_delta` = 15 % cửa sổ, nên hai bài liền nhau
không được xem gần bằng nhau. Thấy nguyên trong log thật, cửa sổ 3–5 s:
`2,5 · 3,6 · 2,8 · 3,2 · 2,7 · 2,3 · 2,9 · 2,0` — so le, không bao giờ gần hai lần. Người
xem hai clip xấp xỉ bằng nhau là chuyện bình thường. Và phép rút là **ba dải đều rời nhau**
(20 % thấp, 10 % cao, 70 % giữa) nên hình dạng có cạnh cứng ở ranh dải, phẳng bên trong.

Giờ là **một** phép rút liên tục lệch về phía ngắn — đúng hình dạng thời gian xem thật: phần
lớn bài chỉ được nhìn qua, ít bài giữ được chú ý. `persona` chuyển từ "chọn dải nào" sang
"độ lệch bao nhiêu": một núm thay ba khoảng cứng.

**`swipe_duration_ms` để lại một cái lỗ.** Ba dải rời: 190–280, 300–520, 520–820 — nên
**không cú vuốt nào dài 281–299 ms**. Một histogram có lỗ là dấu hiệu mạnh hơn bất kỳ giá
trị đơn lẻ nào. Giờ một dải liên tục 190–820 lệch ngắn, giữ nguyên ý của hình cũ mà không có
đường may.

**Biên của vuốt nhanh giữ nguyên 150–240.** Tôi đã nới thành 140–260 rồi test cũ bắt được —
và test đúng: không có phép đo nào nói biên đó sai, lỗi cần sửa là cái lỗ chứ không phải
cạnh. Đổi một con số đã đo mà không đo lại là chính điều repo này cấm.

### 9.26 Interaction: thả tim, và bình luận thủ công (12/08/2026)

**Đã có sẵn, không phải làm:** `ThreadMode::{Threaded, Standalone}` — "acc sau trả lời acc
trước" / "mỗi acc một bình luận gốc" — cùng dropdown trong UI.

**Thêm `manual_comments: Vec<String>`** trên `ThreadCampaignRequest`. Không rỗng thì dùng
thay AI; `instruction`/`max_words` vẫn nằm đó và là thứ chiến dịch quay về. `#[serde(default)]`
nên mọi chiến dịch đã lưu đọc ra là pool rỗng, tức đúng chế độ AI nó được tạo ra với.

Chia theo `(target, ordinal)` chứ không lấy lại từ đầu mỗi link — mười link không mở đầu
bằng cùng một câu — và **tất định**, nên chạy lại một chiến dịch gửi đúng chữ đó, điều làm
cho bằng chứng đã lưu kiểm được. Từ chối khi pool ít câu hơn số message: Threaded nghĩa là
message N trả lời N-1, nên pool hai câu trên chuỗi ba câu sẽ có một acc trả lời một bình luận
giống nguyên văn của chính nó.

**Thêm `like_target: bool`.** `TargetDriver` có method thứ tư `like_target`, **default là từ
chối** chứ không phải im lặng — "người vận hành yêu cầu thả tim mà không có gì xảy ra" phải
nhìn thấy được. Đường hierarchy override nó; đường pixel không, vì thả tim trên trang bài ở
đó cần toạ độ chưa ai đo, và bịa ra thì đúng là điều `screen.rs` từ chối làm với màn hình
chưa calibrate.

Phần thả tim **tách** vào `crates/core/src/tiktok_like.rs` chứ không copy — cùng lý do
`tiktok_drawer` đã tách: hợp đồng này là **đo được**, và hai bản của "nhãn liked xuất hiện
là bằng chứng" sẽ lệch. Lệch ở đây nghĩa là báo tim không có, hoặc từ chối tim đã có.
`nurture::hierarchy::HierarchyRun::like` giờ là một lời gọi tới nó; chỗ đặt điểm chạm vẫn ở
lại vòng lặp, vì lịch sử chạm và bàn tay của máy đó thuộc về phiên, không thuộc về cái tim.

Gọi **sau** khi chứng minh đã tới bài và **trước** khi gõ gì: thanh rail đang ở đúng chỗ
arrival check vừa tìm thấy, và một lần thả tim thất bại **không** làm mất bình luận — nó
được ghi log rồi message đi tiếp. Không fatal có chủ ý: từ chối ở đây là "backend không làm
được" hoặc "nhãn không đổi", không cái nào là lý do bỏ một bình luận đã xếp hàng.

**Chưa chạy live.** Gate H4/H5 (probe, máy thật, reply lồng đúng cha kiểm bằng mắt) là của
đường cũ. Đường **qua app** — `interaction_start_thread` từ UI, DB states, artifact — vẫn
chưa chạy lần nào, và hai tính năng này chưa có lần chạy máy nào cả.

### 9.27 Đảo quyết định: minicap vào bộ cài, không để ngoài repo (12/08/2026)

**Đảo cái gì.** Câu bị gạch ở §9.18 nói đặt APK ngoài repo tại
`~/.riviu/minicap/noarch/minicap.apk` và trỏ bằng biến User `RIVIU_MINICAP_APK`. Từ đây
`minicap.apk` là **resource trong bộ cài**: `sidecars/android/noarch/minicap.apk`, khai báo
ở `bundle.resources` của `tauri.conf.json`.

**Vì sao.** Quyết định cũ không sai lúc nó được viết — nó viết khi minicap là prerequisite
của **người phát triển**, và một biến User trên máy dev là hợp lý. Hợp đồng của **bản phát
hành** thì khác: máy Windows sạch cài xong phải chạy được ngay. Với hợp đồng đó, "tải APK
ngoài repo rồi đặt một biến User" có ba khuyết điểm không sửa được bằng tài liệu:

1. Nó là **một bước tay CI không kiểm được**. Không gate nào fail được khi APK thiếu, vì nó
   không nằm trong cây nguồn.
2. Nó **vô hình cho tới lần stream đầu tiên**. Máy vẫn hiện trong fleet, tile vẫn vẽ, rồi mới
   đổ ở `ensure_stream`. Đo được hôm nay: launch qua `driver.ps1` (script này **không** đặt
   biến đó) cho **cả hai** máy tile `● Error` với
   `startBackgroundStream failed for device …: no minicap apk configured`, fleet `0/2`, dù APK
   nằm sẵn ở `~/.riviu`. Launch lại với `$env:RIVIU_MINICAP_APK` đặt trong **cùng** shell:
   cả hai `● Live`, fleet `2/2`, không đổi một dòng code nào. Cùng một máy, cùng một APK, khác
   nhau đúng một biến môi trường — đó là hình dạng của lỗi này khi nó xảy ra với người vận hành.
3. Nó **không chỉ chết stream**. `open_ui_context` đòi stream, nên nurture và interaction chết
   theo, và người vận hành thấy một app không làm được gì.

**`RIVIU_MINICAP_APK` vẫn override.** Thứ tự là `config → env → bundled`, bản đóng gói ưu tiên
**thấp nhất**. Nó là lưới an toàn cho máy sạch, không tước quyền của ai đang trỏ vào APK khác.

**Giá phải trả, ghi rõ:** +4,2 MB cho **mọi** bộ cài, kể cả macOS nơi minicap hoàn toàn vô
dụng. Chấp nhận có ý thức — 4,2 MB đổi lấy việc bỏ một bước tay không kiểm được. Xuất xứ
(`@devicefarmer/minicap-prebuilt` v2.7.3) và bảng FPS ở §9.18 vẫn nguyên giá trị: cùng đúng
file byte-for-byte, chỉ đổi chỗ nó nằm.

**Giấy phép:** minicap là Apache-2.0 — phân phối lại được, kèm điều kiện truyền NOTICE. Xem
file `NOTICE` ở gốc repo, cũng là nơi ghi mức phơi nhiễm của `adb.exe` đóng gói cùng đợt
(platform-tools theo *Android SDK License Agreement*, **chưa ai review** — quyết định của
người vận hành, ghi lại chứ không giả vờ đã thẩm định).

### 9.28 Đóng gói xong: số đo, và bốn thứ suýt hỏng im lặng (12/08/2026)

**Nghiệm thu nhánh bundled minicap.** Launch qua `driver.ps1` (script **không** đặt
`RIVIU_MINICAP_APK`), shell xác nhận biến không tồn tại: cả hai máy `● Live`, `2 sẵn sàng`,
`Thiết bị 2/2`, đều đang stream, và log **không còn** dòng `no minicap apk configured` mà cùng
lệnh đó đã in ở đầu phiên. `tauri-build` copy đủ 7 file vào `target/debug/sidecars/android/`
với kích thước khớp manifest, nên nhánh packaged được đi vào **ngay trong dev** — không cần
build bộ cài mới kiểm được.

**Bundled adb chạy được từ layout của nó.** `sidecars/android/win-x86_64/adb.exe version` →
`1.0.41 / 37.0.1-15733141`, `Installed as <đường dẫn trong repo>`, exit 0, và `adb devices`
thấy cả hai máy. Đây là bằng chứng cho việc hai DLL nằm cạnh exe là đủ. Chạy được an toàn vì
bản đóng gói **cùng revision 37.0.1** với platform-tools đang giữ server ở 5037 — khác revision
là đúng cái sẽ in `adb server version doesn't match this client; killing...`.

**Vẫn chưa kiểm được, và không kiểm được từ máy này:** nhánh candidate bundled **chưa bao giờ
được chạy** — theo thiết kế, `PATH` thắng trên mọi máy dev. Cần máy Windows sạch. Ghi ở mục
2.7 của plan.

**Bốn thứ suýt hỏng im lặng, tìm ra trong lúc làm:**

1. **`core.autocrlf = true` sẽ phá digest ngay lần clone đầu.** Ship `adb.exe` mà bỏ
   `NOTICE.txt` của Google là đúng lỗ attribution đang bịt, nên mang nó theo — nhưng file đó
   **thuần LF** (21.893 bare LF, 0 CRLF). Không có `-text`, checkout sẽ viết lại thành CRLF,
   file cộng thêm 21.893 byte, và **cả** `bytes` **lẫn** `sha256` ghim cho nó đều sai trên mọi
   máy khác máy đã sinh ra chúng — xanh cho tác giả, đỏ cho tất cả người khác. Chặn bằng
   `sidecars/android/** -text`. Kiểm bằng `git checkout-index` qua một `GIT_INDEX_FILE` tạm
   (không chạm index thật): cả hai digest tái tạo đúng.
2. **`bundle.resources` có 6 resource mà không ai kiểm.** Danh sách viết tay trong
   `verify_packaged_resources` đã lệch khỏi config và không gate nào biết:
   `signer/requirements.txt`, `wda/candidate-manifest.json`, `wda/text-manifest.json`,
   `wda/interaction-capabilities.json`, `wda/interaction-capabilities.schema.json`,
   `wda/interaction_vision_ocr.swift`. Sáu cái này **được ship và không được verify**. Tìm ra
   bởi chính `assert_every_sidecar_resource_is_verified` mới thêm, không phải bởi đọc code —
   đó là lý lẽ cho việc có nó.
3. **Đặt thẳng `minicap_apk = Some(bundled)` sẽ phá cả hai override**, vì config được ưu tiên
   **trước** env. Nên có hai field riêng `bundled_*` ở ưu tiên thấp nhất. Field mà người vận
   hành không thể vượt lên không phải lưới an toàn, nó là chiếm quyền.
4. **`detect_driver` cũ có một lỗi thật.** Nó resolve **một** đường rồi chỉ probe đường đó, mà
   `resolve` lấy candidate đầu tiên **tồn tại**. Một `ANDROID_HOME` cũ trỏ vào SDK đã xoá là đủ
   để nó tuyên bố Android không khả dụng trên máy có adb tốt ở vị trí sau. Giờ là vòng thử từng
   candidate, và refusal kể tên **cả sáu** kèm nguồn (`AdbOrigin::label`) — người vận hành cần
   thấy `RIVIU_ADB_PATH` của họ *đã được đọc và bị từ chối*, không phải đoán xem có được đọc.

**Bẫy test đã tránh:** bản đầu tôi viết `resolve_never_picks_the_bare_name_over_a_real_file`
kỳ vọng bundled thắng. Máy này `ANDROID_HOME` rỗng nên nó xanh — nhưng **image Windows của
GitHub Actions có đặt `ANDROID_HOME`** trỏ vào SDK thật, nên test đó sẽ đỏ trên CI. Viết lại
thành claim không phụ thuộc môi trường (qua `configured`). Cùng loại lỗi với ba test flaky ở
§9.24–9.25: test đo cái mà môi trường quyết định.

**Gate sau đợt này:** 846 test Rust / 0 fail (trước 831), 78 test Python, 95 test frontend / 18
file, `cargo fmt` + `clippy -D warnings` sạch, `npm run build`, e2e 6/6,
`collect_desktop_ci_artifacts.py verify-android-tools` ok. Ba nhánh phủ định đã tự kiểm: file
không ghim bị từ chối, hỏng cùng kích thước bị từ chối theo SHA-256, resource khai báo mà không
ai kiểm bị từ chối.

### 9.29 Sidecar iOS hỏng mà app báo khoẻ: sự im lặng là HAI lỗi (12/08/2026)

**Việc tha exit code 2 không sai — nó có mục đích thật.** Nó làm `verifiedProcessControl` *fail
closed*: giữ driver, bỏ hợp đồng (mục 968-973), và ba test ở
`verified_process_control_requires_a_versioned_ready_ping_handshake` ghim đúng điều đó. Không
đụng vào. Lỗi là **lý do bị mất**, không phải việc tha.

**Điểm mù tệ hơn exit 2, và nó exit 0.** `riviu_pmd.py` phát
`{"ok": true, "pymobiledevice3": false, …}` với **exit 0** khi import thất bại. Nên nhánh cũ
không có gì sai về exit status — payload mới là chỗ sai — và
`crates/ios-driver/src/lib.rs:80` trả `degraded_reason: None` **vô điều kiện** cho mọi `Ok`.
Kết quả: app báo một bản cài hoàn toàn khoẻ trong khi mọi lời gọi thiết bị đều chết. Banner đỏ
**đã có sẵn**; nó chỉ chưa bao giờ được cho một lý do để hiện.

Sửa: `classify_sidecar_ping` phán xét **payload**, không phán xét exit code — nên bắt luôn cả
hai ca. Nêu **mọi** lỗi tìm được, không chỉ cái đầu (một bản cài hỏng thường hỏng nhiều thứ, và
báo một cái là đưa người ta đi sửa triệu chứng). Payload không parse được thì kèm exit code +
400 ký tự cuối stderr, vì traceback Python là thứ duy nhất nói ra nguyên nhân.

**Chỗ im lặng thứ hai: `list_devices`.** Nó bỏ **cả** error của `run_json` **và** key `error`
trong payload. Sửa một chỗ thôi thì đường liệt kê fleet vẫn im. Giờ ghi vào
`last_list_error: Arc<Mutex<Option<String>>>`, set ở cả hai nguồn, **xoá khi liệt kê sạch** — đó
là cách "người vận hành vừa cài Apple Devices" hiện ra mà không cần khởi động lại app.

**Bác bỏ có lý do:** cho `list_devices` trả `Err`. `AppState::bootstrap` hard-fail lần scan đầu
**có chủ ý** vì Flow recovery cần snapshot đáng tin, nên `Err` ở đó biến "chưa cài Apple Devices"
thành app không mở được. Ghi vào doc comment tại chỗ để lần sau không ai "sửa" lại.

`boot_degraded_reason` đã nối vào banner đỏ đang có. `last_list_error()` **chưa có ai đọc** —
nó chờ panel chẩn đoán ở 2.3. Gate: 851 test Rust / 0 fail (ios-driver 137 → 142).

### 9.30 Interaction chạy thật lần đầu qua app — và cửa arrival FAIL OPEN (13/08/2026)

Campaign `08f3c2cc`: Riêng lẻ, 2 target (1 `/video/` + 1 `/photo/`), 2 actor Android,
`messageCount=2`, pool 4 câu tay, không thả tim.

**Đạt:** một bình luận đã đăng **thật qua UI** (`succeeded`, `effect_intent=post_comment`).
Luật chia pool và xoay actor đúng **y bảng dự đoán** trên cả 4 dòng —
`actor_index = (target_index + ordinal) % 2`, `pool_index = (target_index*2 + ordinal) % 4`.
Bằng chứng gửi là thật: `armedFrameSha256 ≠ clearedFrameSha256`, và app **đọc lại được chính
chữ nó vừa gõ** (`postedIdentity.text`).

**`/photo/` là target hạng nhất, đã xác nhận.** `parse_one` nhận cả `video` và `photo`
(`interaction.rs:129-133`), `target_key = content:<id>` nên 9 link không thể trùng,
`kind` chỉ được đọc **một chỗ**: chuỗi cho câu INSERT (`db.rs:1146-1148`). Không driver, cửa
arrival hay khay bình luận nào rẽ nhánh theo `kind`. Link `@.lt.iu.ngh` cũng qua vì điều kiện
chỉ là bắt đầu bằng `@` và dài ≥ 2.

#### Cửa arrival FAIL OPEN — lỗi nặng nhất tìm được trong phiên

```rust
let before = read_author_label(session, labels).await.unwrap_or_default();   // :443
if on_post && !author.is_empty() && author != before {                       // :481 → Structural → gõ và gửi
```

`before` rỗng ⇒ `author != before` **luôn đúng** ⇒ cửa tụt xuống còn "TikTok foreground + có
khay bình luận", mà **chính cái feed cũng thoả**. `ScreenNeverChanged` trở thành không thể xảy
ra. Và `Structural` chỉ được `log::warn!` rồi **vẫn gửi**. Nghĩa là: **bình luận vào bài người
lạ và vẫn ghi `Succeeded`.**

`before` rỗng không hề hiếm: `read_author_label` (`:1102`) dùng `.ok().flatten()?`, nên **lỗi
locate** (agent trục trặc, timeout, cây accessibility chết §9.21) biến thành `None` y như "không
có node Follow".

**Sửa:** thêm `ArrivalRefusal::NoBaseline` (`target_open_no_baseline`), `before` giữ `Option`,
thử lại **một lần** sau `ARRIVAL_POLL` rồi từ chối — **trước** `open_url_in_app`, nên máy không
baseline được thì không tốn side effect nào. Ghim bởi
`an_arrival_check_that_cannot_read_the_baseline_refuses_before_opening_the_link` (khẳng định
`opened` rỗng) và `an_unreadable_baseline_is_retried_once_before_it_is_refused`.

**Hai fixture cũ phải sửa, và đó là dấu hiệu tốt:** chúng không có baseline nào, nên giờ bị
`NoBaseline` trước khi kịp tới `WrongApp`/`NoPostPage`. Cho chúng baseline thật chứ **không** nới
cửa — `a_post_that_never_changes_is_refused_as_an_unresolved_link` vẫn xanh, tức
`ScreenNeverChanged` vẫn tới được.

#### Ordinal 0 bị từ chối TẤT ĐỊNH — app vấp vào dấu chân của chính nó

Redmi bị `target_open_screen_unchanged` trên **đúng cái link** mà Note 8 đăng thành công vài phút
sau. Không phải máy hỏng, không phải link chết. Pha thu frame bằng chứng mở target trên
`target_root_actor` — **chính là máy của ordinal 0** — và không gì đưa máy đi đâu giữa hai
context (`clean_ticket` chỉ dừng stream + invalidate session; phiên sau chỉ *resume* app bằng
`monkey -c LAUNCHER`). Nên tới pha send, `before` **đã là nhãn tác giả của bài đích**, và
`author != before` không bao giờ đúng.

Trong chế độ **thủ công** pha đó còn vô ích: pool phủ mọi `(target, ordinal)` nên `frames` chỉ
được đọc ở nhánh AI. **Sửa:** `needs_ai_evidence_frames()`; manual thì không mở gì cả.

**Bản sửa ngây thơ đã bị bác bỏ:** đổi `?` thành `continue` là chưa đủ — nó sẽ cho một lần chạy
mà **mọi** ordinal 0 của **mọi** target đều bị từ chối sai, mỗi dòng lại nói "bài đã bị
xoá/riêng tư/chặn vùng", đẩy người vận hành đi kiểm link trong khi code đang từ chối dấu chân
của nó.

#### Một target lỗi không được giết cả chiến dịch

Dấu `?` ở khối evidence nằm trong thân trần của `execute_thread_campaign`, nên Err chạy thẳng ra
ngoài: target 2 chưa từng được đặt `Preparing`, `prepared_json` vẫn `NULL`, hai dòng của nó nằm
`queued` **không có lỗi riêng**, và campaign mang lỗi của target khác. Tách
`collect_target_evidence_frames` + `fail_whole_target`; lỗi giờ đánh `Failed` cho đúng các
assignment của target đó rồi `continue`. `queued` im lặng còn nguy hiểm ở chỗ khác: đó **là**
state mà retry coi là retryable.

#### Retry sẽ đăng trùng, qua hai bước

`retryable_assignments` loại `Succeeded` đúng, kèm comment "tapping Send is not idempotent".
Nhưng vòng chuẩn bị ghi `Preparing` cho **mọi** assignment của target, **không** lọc
`only_assignments` (bộ lọc đó chỉ có ở vòng gửi). Nên retry lần một không gửi lại nhưng **xoá
mất** dấu vết thành công; retry lần hai đọc `Preparing`, thấy retryable, và **gửi lại một bình
luận đã công khai**. Sửa bằng một guard `continue` trong vòng chuẩn bị.

#### Câu warn chỉ sai nguyên nhân

"OCR không khả dụng trên nền tảng này" sai kép: điều kiện chỉ xét **mức proof**, không biết gì
về platform hay driver; đường hierarchy không gọi OCR bao giờ; và đường pixel cũng rơi vào
`Structural` khi OCR chạy tốt mà chỉ không thấy handle trong grace. Đổi thành câu nêu đúng cái
*không* đọc được, kèm `reader=`. Và **không** gate việc gửi vào `Identified`: nickname folds lên
handle khoảng **1 trên 3** account (§9.5 đã đo), gate vào đó là từ chối gần hết bài mở tốt.

Thêm `TargetProof::as_str()` và ghi `"arrival"` vào evidence — trước đó mức proof bị bỏ
(`let _proof`) và chỉ tồn tại trong log.

#### Còn hở, chưa sửa

- **Không có byte artifact nào trên đĩa.** `publish_evidence_frame` trả `None` vì
  `close_ui_context` đã **xoá cache frame của UDID đó ở dòng ngay trước**
  (`clean_ticket → teardown_stream → clear_and_advance → state.latest.remove`). Hàng
  `comment-root-evidence` có `sha256` (lấy nhầm từ `postedIdentity.frameSha256`) nhưng
  `relative_path=NULL`, và `artifacts/interactions/` chỉ có `.staging`/`.quarantine` rỗng. Chỉ
  hoist lên trước teardown là **chưa đủ** — `FrameSource::latest` không có hợp đồng liveness nào
  và farm này đã đo hub trả bytes của producer đã chết (`last_frame_age_ms=11373`,
  `baseline_sequence == latest_sequence`). Phải qualify theo **generation + sequence >
  watermark** (`GenerationFrameSource` đã có sẵn, pha interaction chưa dùng).
- `interaction_events` rỗng 0 dòng; `interaction_targets.state` và `interaction_dispatch` không
  bao giờ tiến.
- **API key AI là bắt buộc kể cả chế độ thủ công** (`:591-593` bail *sau khi* campaign đã
  `running`), và lý do vô hình trên UI vì `InteractionCampaignSummary` không có `error_code`.
- H6-e (trộn iPhone+Android ở Threaded) **không quan sát được**: gate phân hoạch theo
  `reports_element_bounds`, hai máy Android thì nhóm pixel rỗng nên gate luôn cho qua. Cần một
  iPhone, không có cách lách.

#### Bẫy của harness, không phải của app

**EVKey64** (bộ gõ tiếng Việt bên thứ ba, hook bàn phím toàn cục, **không** hiện trong
`Get-WinUserLanguageList`) ăn keystroke của SendKeys: `www`→`ww`, `@user`→`@ùe`, `photo`→`phồt`
(`f`=huyền, `s`=sắc, `r`=hỏi, `oo`=ô). Cả hai link đầu tiên bị `unsupportedHost`. Cách đúng:
`Set-Clipboard` rồi `fill x y "^a{DEL}^v"` — Ctrl+V là tổ hợp phím, EVKey không biến đổi.

**`uiautomator dump` không phải dụng cụ đo được cây khi TikTok đang phát video**:
`ERROR: could not get idle state.` Agent uiautomator2 mà app dùng đọc `AccessibilityNodeInfo`
trực tiếp, **không** chờ idle — nên dump thất bại **không** chứng minh app đọc không được.

**`mCurrentFocus=…SplashActivity` không có nghĩa là đang ở màn splash** — đó là tên activity
chính của TikTok. Đừng kết luận "máy đang treo ở splash" từ dòng đó.

Gate: **855 test Rust / 0 fail** (851 → 855), `cargo fmt` + `clippy -D warnings` sạch.

### 9.31 H6-a ĐẠT sau ba lần chạy — và link chết trông giống hệt lỗi code (13/08/2026)

Sau khi sửa xong §9.30 cộng A3 (artifact), chạy lại **hai** lần nữa. Cả hai lần đều đáng ghi vì
chúng nói hai chuyện khác nhau.

**Lần 2 (`3e617811`): cả 4 assignment `target_open_screen_unchanged`, 0 bình luận.** Nhưng A2
đã hoạt động — chiến dịch **không** còn dừng ở lỗi đầu, nó chạy hết cả 4 và target 2 được chạm
tới (lần 1 nằm `queued`). Nguyên nhân không phải code: **hai link photo tôi chọn không resolve
được**. Chứng minh bằng ảnh màn hình thật: bắn intent rồi `adb exec-out screencap` cho thấy máy
vẫn ở tab `Đề xuất` với video của `KietFei` — TikTok nhận intent rồi để nguyên feed. Cửa từ chối
đúng, và câu "thường là bài đã bị xoá/riêng tư/chặn vùng" đúng nguyên nhân.

**Phân biệt được "link photo" với "bài này chết":** cùng máy, `@tuyt.hoa7225/photo/…` **mở
được** (thấy badge `Ảnh`, 7 dấu carousel, caption đúng) và `@user497553423635/video/…` cũng mở
được. Nên deep link `/photo/` chạy tốt; chỉ hai bài kia là chết. **Bài học đo lường:** một link
chết và một lỗi code cho ra **cùng một** `error_code`. Cách duy nhất tách được là chụp màn hình
máy. Đừng sửa code trước khi làm việc đó.

Cũng xác nhận A1 là bug thật: bài mà link `/video/` mở ra có tác giả **`Xuân`** — đúng
`Follow Xuân` trong thông báo lỗi của **lần 1**. Tức là pha evidence đã để Redmi đứng sẵn trên
bài đích, y như suy luận.

**Lần 3 (`44edce27`), hai link đã chứng minh mở được — `state=partial`:**

| line | kind | ord | actor | text dự đoán | kết quả |
|---|---|---|---|---|---|
| 1 | photo | 0 | Redmi | nhìn cuốn thật đấy | `failed: target_open_no_baseline` |
| 1 | photo | 1 | Note 8 | màu lên đẹp quá | **succeeded**, `arrival=structural` |
| 2 | video | 0 | Note 8 | xem đi xem lại mấy lần | **succeeded**, `arrival=structural` |
| 2 | video | 1 | Redmi | cái này hợp gu mình | `failed: target_open_no_baseline` |

**Cả 4 dòng khớp y bảng dự đoán** về actor và text, và **cả hai target đều chạy** — nên chiều
*target* của luật chia pool (chỉ số 2, 3) giờ đã chứng minh được, không chỉ chiều ordinal. Cả
hai lần thành công đều đọc lại được **đúng** chữ đã gõ khỏi màn hình (`postedIdentity.text ==
prepared.text`).

**Đăng được lên bài `/photo/`** — đó là điều lần 1 và lần 2 chưa chứng minh được.

**`NoBaseline` nổ hai lần trong thực tế, và nổ đúng.** Cả hai lần trên Redmi, vì máy đang ở một
thẻ **LIVE** (không có node `Follow `) nên không baseline được ⇒ từ chối **trước khi gửi
intent**. Đây đúng là rủi ro mà kế hoạch đã cảnh báo, và nó fail-closed: thà mất một assignment
hơn là bình luận mù. **Hệ quả vận hành:** máy đang ở thẻ LIVE thì assignment đó bị bỏ. Nếu tỉ lệ
này cao, việc cần làm là đưa máy về thẻ bài thường trước khi chạy, **không** phải nới cửa.

**Artifact có thật trên đĩa, và mở ra xem được.** 4 file JPEG (`ffd8ff`), 38–71 KB, sha256 khác
nhau, cả hai loại `comment-root-evidence` và `comment-failure-evidence` — đường lỗi cũng lưu
ảnh. Mở file lớn nhất ra xem: khay bình luận đang mở trên đúng bài photo mục tiêu, hiện
`màu lên đẹp quá` của `Hoàng Hồng Nam`, nhãn `Bình luận đầu tiên`, `1 giây` trước, header
`1 bình luận`. Đúng thứ dùng để phân xử về sau.

`arrival=structural` giờ nằm trong `evidence_json` (A4.3). `state=partial` là báo cáo trung
thực: không phải `failed`, không phải `succeeded`.

**H6-a: ĐẠT.** Còn nợ: `interaction_events` vẫn rỗng, `interaction_targets.state` và
`interaction_dispatch` vẫn không tiến, `error_code` của campaign vẫn không hiện trên UI.

### 9.32 H6-b và H6-c ĐẠT trong một lần chạy (13/08/2026)

Chiến dịch `e851ce3f`: **Qua lại** (threaded) + **thả tim bật** + comment thủ công, 2 target đã
chứng minh mở được, 2 actor Android. `state=partial`.

| line | kind | ord | actor | text | kết quả |
|---|---|---|---|---|---|
| 1 | photo | 0 | Redmi | công nhận nhìn thích mắt | `uncertain` — đã bấm Gửi, không xác nhận được |
| 1 | photo | 1 | Note 8 | đúng ý mình luôn | `skipped_parent` at ordinal 0 |
| 2 | video | 0 | Note 8 | chỗ này trông yên tĩnh | **succeeded**, `post_comment` |
| 2 | video | 1 | Redmi | save lại để dành | **succeeded**, `reply_comment` |

Cả 4 text và cả 4 actor khớp y bảng dự đoán (`actor_index = (target_index + ordinal) % 2`).

**H6-b — reply lồng đúng cha, kiểm bằng mắt.** `2v/1` có `parent_assignment_id` trỏ đúng `2v/0`,
và evidence ghi `parent.author='Hoàng Hồng Nam'`, `parent.text='chỗ này trông yên tĩnh'` — tức
nó khớp cha theo **cả tác giả và nguyên văn**, không phải theo vị trí. Mở
`comment-reply-evidence` (45.400 byte) ra xem: `save lại để dành` của **`Mítt zới còiii`** thụt
vào **dưới** `chỗ này trông yên tĩnh` của **`Hoàng Hồng Nam`** — hai tài khoản khác nhau, đúng
cấu trúc chuỗi. Ảnh đó cũng cho thấy comment của lần chạy 1 (`1 giờ`) và lần 3 (`8 phút`) vẫn
còn, khớp mốc thời gian.

**Chuỗi đứt được xử lý đúng.** Root `1p/0` thành `uncertain` ("đã bấm Gửi nhưng không xác nhận
được; không retry vì trạng thái giao nhận mơ hồ") ⇒ `1p/1` thành `skipped_parent` kèm
`parent_identity_not_confirmed_at_ordinal_0`. Nó **không** trả lời một cha không xác định, và
`Uncertain` không retry được — đúng thiết kế. Đây cũng là ca `comment-failure-evidence` có ảnh
trên đĩa, tức đúng cái state cần người mở ra xem thì có thứ để xem.

**H6-c — thả tim.** Ba dòng `đã thả tim (nhãn đổi trạng thái)` đúng nguyên văn chuỗi mong đợi:
`content:7668985481056587029` bởi Redmi, `content:7669277385455340807` bởi Note 8 rồi bởi Redmi.
Ba chứ không bốn, vì `1p/1` bị `skipped_parent` trước khi tới bước tim — đúng.

**Và điều quan trọng nhất của H6-c: thả tim không bao giờ làm mất bình luận.** `1p/0` thả tim
thành công *và* vẫn gửi comment (rồi mới `uncertain` ở bước xác nhận); `2v/0` và `2v/1` thả tim
thành công *và* comment `succeeded`. Không có ca nào tim làm chết comment.

**H6-e** (trộn iPhone+Android ở Threaded ⇒ `MixedPlatformThread`) **gác lại theo yêu cầu người
vận hành** — gate phân hoạch theo `reports_element_bounds` nên hai máy Android không bao giờ trip
được nó.

### 9.34 H6-d ĐẠT sau khi sửa ba chỗ ở 9.33 (13/08/2026)

Chiến dịch `9b1ddc61`: AI viết, Riêng lẻ, hai link đã chứng minh. `state=partial`, và lần này
**cả bốn assignment đều chạy**.

**Chữ AI viết đăng được, đọc lại đúng nguyên văn:**

| line | ord | kết quả |
|---|---|---|
| 1 photo | 1 | `'Nghe ổn áp thật, gom đồ vô là đi thôi!'` — read back khớp |
| 2 video | 1 | `'Ảnh sóng ảo chất quá, muốn ghé quá!'` — read back khớp |

**Ba bản sửa đều nghiệm thu được ngay trong lần chạy này:** lỗi AI ở `1p/0` **không** còn giết
chiến dịch (trước đó `1p/0` chết là hết); assignment đó có `error_code` **riêng**; và lỗi đó
**nêu nguyên nhân**.

**Và nguyên nhân hoá ra không phải API chết.** Nó là một **cửa chất lượng bên trong app**:

```
1p/0: comment_context_rejected: context=0  overall=0  instruction=100 genericity=0
2v/0: comment_context_rejected: context=60 overall=60 instruction=90  genericity=40
```

Câu AI viết bị từ chối vì chưa neo đủ vào nội dung bài — `2v/0` đạt 60 mà vẫn bị loại, nên
ngưỡng nằm trên 60. Suy đoán cũ của tôi (`model deepseek-v4-flash` không tồn tại) **sai**: model
gọi được, key dùng được, và cái chặn là gate của chính chúng ta.

**Hai cột là schema chết, đã ghi tại chỗ thay vì lấp.** `interaction_targets.state` và
`interaction_dispatch` **không có writer nào** (chỉ INSERT giá trị mặc định) **và không có reader
nào**. Đo được: mọi dòng target đứng `queued` kể cả target có assignment `succeeded`. Cố ý **không
lấp**: `interaction_assignments` mới là bản ghi thật, và một state ở cấp target duy trì song song
là nguồn sự thật thứ hai có thể lệch với cái thứ nhất. `interaction_dispatch` thì kèm một hiểm
hoạ đã ghi vào schema: nó có hình dạng của một lease một-chủ (`owner`, `claimed_at`) nhưng **không
ai claim**, nên đừng ai coi dòng đó là bằng chứng có chủ. Nếu sau này hai instance trên cùng một
data dir là chuyện có thể xảy ra thì đây là chỗ đặt guard — hiện chưa có.

**`interaction_events` vẫn rỗng** và vẫn chưa quyết: hoặc ghi ở các transition đã có, hoặc xoá
bảng. Chưa làm cái nào.

**Điểm cần người vận hành quyết:** ordinal 0 bị loại trên **cả hai** target, ordinal 1 đậu trên
cả hai. Khác biệt duy nhất giữa chúng là `direction`: ordinal 1 được thêm câu "trả lời tự nhiên
câu trước ..." vì `previous` đã có. Đáng nói là **ở chế độ Riêng lẻ thì câu đó vô lý** — bình
luận độc lập không trả lời ai — nhưng chính nó lại làm chữ neo tốt hơn và đậu gate. Tức tỉ lệ
loại ~50% ở lần đầu mỗi target là **có thể sửa được**, và chỗ sửa là prompt, không phải gate.

### 9.36 Đăng bài: phần không cần máy đã làm, và phép đo quyết định (13/08/2026)

Kế hoạch xếp **code trước, đo sau** cho Phase 3, và ba phần dưới đây không chạm máy nào.

**Nhãn khai báo mà từ chối.** Sáu control mới — `ProfileTab`, `ComposerNext`, `PostButton`,
`PostDeleteMenu`, `PostDelete`, `PostDeleteConfirm` — đều `None` ở **cả hai** label set vì
chưa ai đo. `None` nghĩa là từ chối. Ba nhãn xoá từ chối **lúc chọn driver**, không phải giữa
chừng: bài đã đăng mà không gỡ được là lời hứa phiên chạy không giữ được, nên phải từ chối
trước khi đăng.

Hai điều ghi trước cả khi đo, vì chúng là bẫy đã biết: `ProfileTab` **phải** `Exact` —
`Hồ sơ <tên>` có trên action rail nên `Contains` sẽ mở hồ sơ tác giả, đúng bẫy từng làm
`Contains("Follow")` khớp tab `Đã follow`, và so khớp description **không phân biệt hoa
thường**. `PostDelete` **phải** dùng `locate_all` chứ không `locate` — rail đã có
`Thêm hoặc xóa video này khỏi mục Yêu thích` (mục 2641) nên hơn một match là chuyện thật và
phải từ chối.

**Test đầy đủ giờ không trôi được nữa.** `no_entry_carries_an_empty_label` trước đây lặp một
mảng **viết tay** chỉ có 15 trong 17 control, nên nhãn nào thêm sau cùng đơn giản là không
được kiểm — im lặng. Giờ nó lặp `TikTokControl::ALL`, và `ordinal()` (`#[cfg(test)]`) match
**exhaustive** nên compiler từ chối một variant mới cho tới khi nó có trong `ordinal`, rồi
`every_control_appears_in_all` từ chối tới khi nó có trong `ALL`.

**`publish_driver.rs`: "chứng minh rồi mới xoá" là sự thật ở tầng type.** `PostProof` chỉ
dựng được bởi một hiện thực `prove_own_post` (field private, constructor nhận *quan sát* chứ
không nhận một chữ "đúng"), nên `delete_proved_post` không thể gọi mà chưa chứng minh gì. Thứ
tự do compiler kiểm, không do ai nhớ. Ba mức không đồng nhất, theo số đo: `Follow ` trên rail
là **từ chối dứt khoát** (bài người khác, caption khớp mấy cũng không bù được); caption không
khớp thì từ chối, bị cắt thì **hạ** xuống `captionProof="prefix"`; counter ảnh đọc không được
thì **hạ** xuống `"unread"` (mục 9.20 đã đo counter là overlay tạm) nhưng counter **đọc được
mà lệch** thì từ chối — vì lúc đó bài trên màn không phải bundle.

`DeleteFailure` copy hai variant của `SendFailure` **và lý lẽ đảo chiều**: với comment,
`AfterEffect` chặn retry để không đăng hai lần; với xoá, nó chặn retry vì lần thử thứ hai sẽ
rơi vào **bài mới nhất hiện tại**. Cùng variant, lý lẽ ngược nhau — và chính chỗ đảo đó là lý
lẽ mạnh nhất cho cả chuỗi bằng chứng.

#### Phép đo quyết định, chạy được ngay

```
cargo run -p riviu-android-driver --example probe -- <serial> --measure-own-post "<caption>"
```

**Chỉ đọc, không tap gì.** Mở sẵn một bài của mình trên máy rồi chạy — probe **không** tự
điều hướng tới đó, cố ý, để phép đo không dựa vào một grid hồ sơ chưa ai đo. Nó dump cây, rồi
trả lời đúng một câu: caption của campaign có nằm **nguyên văn** trên màn không. Không nguyên
văn thì nó đo prefix dài nhất **theo ký tự** (caption tiếng Việt, đi theo byte sẽ cắt giữa
code point) và kết luận:

| prefix đọc được | kết luận |
|---|---|
| nguyên văn | luật của người vận hành hiện thực được như đã viết |
| ≥ 24 ký tự | đủ để định danh một bài; ghi `captionProof="prefix"` |
| 1–23 ký tự | **quá yếu** — giữ xoá bằng tay |
| 0 | **không chứng minh được** — không xoá tự động |

Nó cũng báo có `Follow ` trên trang không, vì đó là từ chối dứt khoát duy nhất trong chuỗi.

**Chưa chạy.** Đây là số đo M4/M5 và nó cần một bài thật của người vận hành.

### 9.48 Điều khiển từ máy tính không được park stream (13/08/2026)

Bấm ô máy trên lưới **mở overlay giữa màn** (preview lớn + phím Back/Home/Recents/âm lượng/Power/thông báo/chụp), không gửi gesture từ thumbnail. Gesture chỉ chạy trên preview lớn.

`device_tap` / `device_swipe` / `device_type_text` / `device_home` / `device_key` / `group_input` đi qua `DeviceControlPlane::open_manual_session`: exclusive **không** `submit_park`, **không** `start_interaction_session`, **không** foreground TikTok, **không** tạo MJPEG mới, và `close_manual_session` **không** `invalidate_ui_session`. iOS tái dùng session WDA đang cache khi stream còn sống; `POST /session` lúc MJPEG đang chạy vẫn bị cấm. Android `open_session` độc lập với minicap. Nurture đang giữ exclusive thì tap trả `DeviceBusy`.

Đừng quay lại `open_ui_context` cho thao tác tay: path đó park tile, `monkey` TikTok, chờ 40 s, rồi teardown stream.

### 9.47 v0.1.1 đã phát hành, và chuỗi updater nghiệm thu từ ngoài (13/08/2026)

Lần tag thứ ba xanh cả năm job. Release `v0.1.1` là **Latest**, 23 asset.

**Nghiệm thu từ ngoài bản build, không phải từ log CI.** Gọi đúng cái URL đã nướng vào mọi
binary — `/releases/latest/download/latest.json` — rồi đi tiếp từng URL bên trong nó:

| khoá | HTTP | magic 4 byte | là gì |
|---|---|---|---|
| `darwin-aarch64` | 206 | `1f8b0800` | gzip ⇒ `.app.tar.gz` |
| `darwin-x86_64` | 206 | `1f8b0800` | gzip |
| `windows-x86_64` | 206 | `4d5a9000` | `MZ` ⇒ NSIS setup.exe |
| `windows-x86_64-msi` | 206 | `d0cf11e0` | OLE compound ⇒ **MSI thật** |
| `windows-x86_64-nsis` | 206 | `4d5a9000` | cùng file, cùng 58.206.452 byte với khoá trơn |

Dòng `-msi` là bằng chứng cái sửa ở 9.41 chạy thật, không chỉ chạy trong test: khoá đó trả một
tệp OLE, tức MSI, chứ không phải PE của NSIS.

**Chữ ký khớp byte.** Chuỗi inline trong `latest.json` bằng đúng nội dung asset `.sig` đã
publish, và giải base64 ra `untrusted comment: signature from tauri secret key`.

**Tên asset không bị GitHub đổi** — đúng như thiết kế ở 9.41: cái ghi ra đĩa đã là cái GitHub
phục vụ, nên URL trong manifest đoán được.

**Ba lần tag, ba lỗi thật khác nhau, không lần nào do thay đổi cùng ngày:**

1. `packaging` thiếu ở job release (9.45) — job đó **chưa bao giờ chạy** trong lịch sử repo.
2. Ngân sách cleanup 10ms trong test (9.46) — flake thời gian thực.
3. Xanh.

Cả ba lần đều fail **trước** `gh release create`, nên không release nào bị tạo dở và gate bất
biến không bị chạm — tag chưa có release đằng sau thì dời được, không phải đốt version. Đó là
lý do vẫn ra `v0.1.1` chứ không phải `v0.1.3`.

**Ghi chú về HEAD:** `curl -I` lên asset của GitHub Release trả `HTTP 000` (CDN từ chối), nên
muốn kiểm URL phải dùng **GET một phần** (`Range: bytes=0-63` → `206`). Nếu tin HEAD thì sẽ
kết luận sai là cả năm URL chết.

### 9.46 Ngân sách 10 mili-giây trong test, và lần thứ hai cùng một loại lỗi (13/08/2026)

Lần tag thứ hai: **quality fail**, `TimeoutError: app process-control deadline expired` ở
`test_verified_terminate_accepts_an_already_absent_process`. Nó vừa xanh ở run ngay trước, và
tôi không sửa dòng nào trong `riviu_pmd.py`.

**Đọc traceback kỹ mới thấy chỗ đúng**: lỗi đến từ khối `finally` — bản thân thao tác **thành
công**, rồi một bước *cleanup* vượt ngân sách. Và ngân sách đó, trong test, là
`TERMINATE_CLEANUP_TIMEOUT_SECONDS = **0.01**` — **10 mili-giây đồng hồ thực cho toàn bộ
cleanup cộng lại**, trong khi mọi thứ trong test đều là fake.

10ms là **cùng bậc độ lớn** với một lần GC pause hay một khựng của Defender trên runner dùng
chung. Chứng cứ ngay tại máy này: chạy đúng bộ Python của CI bốn lần liên tiếp cho 16,4s rồi
5,7s / 5,3s / 5,4s — **biên độ 3 lần** chỉ do trạng thái máy.

Vì sao nó bị ép xuống 0.01: fixture làm một await chậm bằng `sleep(1)`, nên ngân sách phải nhỏ
hơn 1s để test "boundary có bị chặn" chứng minh được điều gì. Nhưng ngân sách cũng là đồng hồ
thực mà các test đường-thành-công phải **chạy xong bên trong** nó. Cửa sổ hợp lệ rất rộng, và
0.01 nằm sát mép dưới.

Sửa: đặt tên cho ba con số chỉ có nghĩa khi đi cùng nhau — `FIXTURE_STALL_SECONDS = 1.0`,
`TEST_DEADLINE_SECONDS = 0.25`, `BOUNDED_WITHIN_SECONDS = 0.9` — và nêu ràng buộc ngay tại chỗ.
0.25 vẫn kém 1s bốn lần (nên boundary chậm vẫn trip đúng) mà nâng sàn lên **25 lần**. Hai
assertion thời lượng đổi từ `0.25` sang `BOUNDED_WITHIN_SECONDS`; ý nghĩa giữ nguyên: *chạy xong
mà không chờ hết cú stall*. Giá phải trả là module đó chạy ~3,2s thay vì ~0,3s — không đáng kể so
với một job quality 30 phút. Chạy lại 6 lần: ổn định.

**Đây là lần thứ hai trong cùng một ngày cùng một loại lỗi** (lần đầu ở mục 9.42, ngưỡng
classification). Bài học chung: **một ngưỡng thời gian thực đặt sát chi phí thật là một đồng xu
tung, không phải một cái gate.** Và cả hai lần, dấu hiệu đều là "test này vừa xanh ở commit
trước mà tôi không sửa gì liên quan".

### 9.45 Tag đầu tiên: job release chưa bao giờ chạy, và nó hỏng (13/08/2026)

Push tag `v0.1.1`. Quality xanh, cả ba build xanh, **job release fail** —
`ModuleNotFoundError: No module named 'packaging'` ở bước `verify-release`.

**Không phải do thay đổi nào của tôi.** Job release chạy cùng script collector như hai job kia,
mà script `import packaging` ở mức module. Hai job kia có nó **như tác dụng phụ** của việc cài
`requirements-build.txt`; job release **không cài gì cả**. Nó chỉ chạy khi push tag, nên đây là
lần đầu tiên trong lịch sử repo có gì đó thực thi nó.

**May ở chỗ nó fail đúng chỗ**: trước `gh release create`, nên **không release nào được tạo** và
gate bất biến không bị chạm. Tag tồn tại mà không có release là trạng thái sửa được — dời tag
sang commit đã fix, chứ không phải bump lên `v0.1.2`.

Sửa: cài **đúng một** dependency, và **đọc pin từ lock** chứ không viết lại số vào workflow —
một bản sao thứ hai của pin là một chỗ thứ hai để quên.

**Và đóng cả lớp lỗi, không chỉ ca này.** Cái nguy hiểm ở đây là *một job không bao giờ được
tổng duyệt*: nó chỉ chạy trên tag, nên mọi lần push đều không chứng minh gì về nó. Thêm
`every_job_running_the_collector_installs_what_it_imports` — đọc workflow, tách theo job, và
đòi job nào chạy collector thì job đó phải cài dependency. Đã kiểm ngược: bỏ fix ra thì nó chỉ
đúng `['release']`. Đây là thứ chạy trên **mọi** push, tức thứ duy nhất có thể canh một job
không ai diễn thử.

### 9.44 Đường Đăng bài cho máy Android đi qua, và bốn version chỉ kiểm ba (13/08/2026)

Hai lỗi tìm ra khi hiện thực quyết định "chưa đăng gì từ Android", không phải khi thiết kế.

**1. Không có gate nền tảng nào cho Đăng bài.** Gate duy nhất là `supports_push_media`, và
driver Android trả **`true`** — đúng, vì đẩy ảnh vào gallery là phần nó **có** làm thật. Thứ
thiếu là composer, và không capability nào nói điều đó. Nên một máy Android map được vào
campaign, bấm Transfer rồi Post, và `publish_commands` tap **toạ độ logic iOS** với **bundle
iOS** lên nó — đúng loại "bịa toạ độ cho cú tap không lùi được" mà mục 10 cấm, mà lần này là
đăng bài.

Nặng hơn: doc comment trong chính file ghi *"the Publish page refuses an Android target before
dispatch"*. **Không có gate nào như vậy** — không ở UI (`FarmPages.tsx` chỉ gọi thẳng), không ở
backend. Một câu doc tự nhận có bảo vệ là tệ hơn không có câu nào, vì nó làm người đọc sau
không đi kiểm.

Sửa: `refuse_devices_this_path_cannot_drive` gate theo **`reports_element_bounds`** — cùng tín
hiệu đường tương tác dùng để phân hoạch pixel/cây — gọi ở **cả hai** cửa vào trước mọi thay đổi
trạng thái. Từ chối ở `publish_transfer` nữa, không chỉ `publish_post`: transfer trước sẽ đẩy
ảnh lên một máy không bao giờ đăng được rồi để đó. Nhận predicate thay vì control plane để test
được mà không cần fleet. Thông báo **nêu tên đúng máy vi phạm**, vì fleet trộn và "một máy nào
đó" bắt người ta đi mò 16 cái.

**2. Bốn trường version, `verify-version` chỉ kiểm ba.** Nó kiểm `tauri.conf.json`,
`package.json`, `Cargo.toml` — và bỏ **`tauri.full.conf.json`**, chính overlay mà bản release
build bằng, tức chính file quyết định version binary **tự báo lúc chạy**.

Hậu quả chỉ xuất hiện **sau khi phát hành**: `latest.json` quảng cáo `0.1.1`, binary tự nhận
`0.1.0`, nên mọi bản đã cài được mời đúng bản cập nhật đó **mãi mãi** và không bao giờ thoả —
một vòng lặp không có gì ở phía này nhận ra. Đã thêm vào hợp đồng version.

### 9.43 Hai lối xoá còn lại: đã đo, đều đóng — và một lối khác mở ra (13/08/2026)

Mục 9.37 kết luận xoá tự động không dựng được từ trang bài, và ghi lại **hai lối chưa thử**.
Đo cả hai, chỉ-đọc, trên bài thật của tài khoản Note 8.

**Lối 1 — long-press trong grid hồ sơ: đóng.** `input swipe x y x y 900` trên một ô grid chỉ
**mở bài**, y như một cú tap. Không menu ngữ cảnh, không gì khác xuất hiện.

**Lối 2 — `Cài đặt quyền riêng tư`: đóng cho việc xoá.** Sheet không cuộn, nên inventory dưới
đây là **toàn bộ** nó:

| nhãn | loại | clickable |
|---|---|---|
| `Cài đặt quyền riêng tư` | TextView (tiêu đề) | false |
| `Đóng` | Button | **true** |
| `Ai có thể xem bài đăng này` | TextView (đầu mục) | false |
| `Mọi người` / `Bạn bè` / `Chỉ bạn` | TextView | **false** cả ba |
| `Cho phép bình luận` | Switch (desc dài) | true |
| `Cho phép sử dụng lại nội dung` | Switch | true |

**Không có mục xoá.** Vậy bốn bề mặt đã đo — `...` trên trang bài (ra share sheet), thân trang
bài, long-press trong grid, và sheet quyền riêng tư — đều không có control xoá nào có nhãn.
Kết luận "xoá tự động không dựng được bằng nhãn trên trill 46.3.3" giờ là **đã quét hết**, không
còn là "chưa thử nốt".

**Nhưng lối 2 mở ra một thứ khác: `Chỉ bạn`.** Đặt bài về chỉ-mình-xem đạt được *mục đích*
"bài không còn công khai" mà **không** cần xoá, và nó **có nhãn** — khác hẳn nút xoá. Hai điều
kiện kèm theo, đo được và phải nói rõ:

1. Node `Chỉ bạn` là `clickable=false`. Đích tap là hàng cha không nhãn, nên cách chạy là
   locate theo nhãn rồi tap **tâm bounds của nhãn đó** — đúng cơ chế `interaction_hierarchy`
   đang dùng (`element.centre()`), không phải toạ độ bịa.
2. Trên đúng bài tôi đo, `Chỉ bạn` **xám** trên ảnh (bài này đang bật Ủy quyền quảng cáo).
   Cây báo `enabled=true` nhưng đó là thuộc tính của TextView, không phải trạng thái của hàng.
   Nên "`Chỉ bạn` có chọn được không" **chưa chốt**; trên bài mình vừa đăng thì rất có thể có.

Đây là **lựa chọn của người vận hành, không phải thứ tôi tự đổi**: "gỡ bài" và "để bài ở chế độ
chỉ mình xem" là hai việc khác nhau — bài vẫn còn trên tài khoản. Ghi ra để chọn, không tự thay.

### 9.42 Ngưỡng thời gian của `classification_stays_fast_enough_for_the_watcher` (13/08/2026)

Test này fail trong gate của tôi. **Không phải do thay đổi nào của tôi** — nó nằm ở
`crates/core`, chỗ tôi không sửa dòng nào. Ba số đo trên **cùng code, cùng ảnh vào**:

| chạy thế nào | fastest-of-5 |
|---|---|
| chỉ test đó | 166 ms |
| cả binary `real_frames` | 377 ms |
| `cargo test --workspace` | **424 ms** — vượt ngưỡng 400 |

Tức trạng thái máy một mình đổi số đo **2,5 lần**.

**Đáng nói là doc comment của chính test đã ghi một lần fail y hệt ở 416 ms.** Lần đó cách
đo được sửa (lấy pass nhanh nhất thay vì trung bình) nhưng **ngưỡng để nguyên**. Lấy
fastest-of-5 làm hẹp nhiễu chứ không bỏ được nhiễu, nên cùng kiểu fail quay lại. Bài học:
sửa estimator mà không sửa bound thì mới sửa một nửa.

Nâng lên **1200 ms**, có căn cứ: quan sát tệ nhất thật là 424 ms, và đây là **bản debug**
với vai trò chống hồi quy thuật toán, không phải ngân sách thời gian thực — 24 FPS là
41 ms/frame, nên 1200 ms đã là ~29× cái frame nó bảo vệ. Thứ nó vẫn bắt được là loại thay
đổi đáng bắt: mất pyramid, hoặc quét ở full resolution — cả hai đắt theo **lần**, không
theo phần trăm. Số in ra vẫn giữ, vì assert chỉ nổ khi đã quá muộn còn dòng in cho thấy
trôi dần.

### 9.41 Updater xong đường phát hành: `latest.json`, và thứ tự lúc cài (13/08/2026)

**Lỗ hổng tìm ra khi nối, không phải khi thiết kế:** `find_installers` lọc theo đuôi
`.msi/.exe/.dmg`, nên **`.sig` chưa bao giờ được thu**, và trên macOS `.app.tar.gz` cũng
không. CI đã in `Finished 2 updater signatures at:` nên chữ ký *có sinh* — chỉ là không ai
mang nó ra khỏi máy build. Chữ ký không tới được release thì `latest.json` không có gì để
ghi, và updater bắt buộc phải có nó.

**Trap thứ hai, tìm ra bằng cách đọc code plugin chứ không đoán: bản cài MSI.** Plugin tra
khoá theo thứ tự `{os}-{arch}-{installer}` **rồi mới lùi về** `{os}-{arch}`
(`updater.rs:568-598`), trong đó `installer` là loại bộ cài mà **bản đang chạy** được cài từ.
Nên một `latest.json` chỉ có khoá trơn `windows-x86_64` **không** làm bản MSI "không cập nhật
được" như tôi viết trong README lúc đầu — nó làm bản MSI **âm thầm cài bản NSIS đè lên**: một
app, hai mục gỡ cài đặt, hai danh tính registry. Sửa: mỗi loại bộ cài một khoá
(`windows-x86_64-nsis`, `windows-x86_64-msi`, và khoá trơn trỏ NSIS làm câu trả lời cho loại
bundle plugin không nhận ra). Chữ ký MSI vốn đã sinh sẵn — CI in "Finished 2 updater
signatures" — chỉ là chưa ai mang ra.

Đây là loại lỗi chỉ sửa được **trước** lần phát hành đầu: asset của một release là bất biến.

**Trap về tên asset.** GitHub **đổi tên** asset có ký tự nó không thích — dấu cách thành
dấu chấm. Bản Windows tên `Riviumanagersphone Full_0.1.0_x64-setup.exe`, nên URL phục vụ
sẽ khác tên đã upload, và `latest.json` phải chứa URL **đúng từng ký tự**. Chọn cách
**đổi tên từ đầu** trong collector (`release_asset_name`) thay vì đoán cách GitHub
sanitize: cái ghi ra đĩa đã là cái GitHub sẽ phục vụ, checksum phủ đúng tên đó, hai bên
không lệch được. Còn 0 tag nên đổi tên lúc này không phá gì. `verify_updater_record` từ
chối luôn mọi tên mà GitHub *sẽ* viết lại — fail closed trước khi nó thành URL.

**Chữ ký kiểm hai lớp**: `.sig` của Tauri là base64 bọc minisign, và `latest.json` lấy
base64 đó nguyên văn. Kiểm cả base64 **và** chuỗi giải ra có mở đầu `untrusted comment:` —
base64 hợp lệ mà không phải minisign sẽ được collector nhận và bị **mọi client** từ chối,
tức chỗ tệ nhất để phát hiện.

**Thiếu một platform là fail, không phải suy giảm nhẹ.** Public key đã nướng vào mọi binary
đã ship và gate bất biến cấm sửa release đã phát hành, nên một `latest.json` thiếu entry
là platform đó **không bao giờ** update được, chỉ chữa bằng cách phát hành version cao hơn.
Fail job release tốn một lần re-tag; ship lỗ hổng tốn mọi bản đã cài.

**Thứ tự lúc cài là chỗ chịu lực nhất, và test ghim nó.** `install` của plugin kết bằng
cách tự gọi `process::exit`, nên `RunEvent::Exit` **không chạy**. Thứ tự: hỏi busy → **tải
xong** → `graceful_shutdown` → mới `install`. Tải trước vì tải fail phải để fleet y nguyên;
nhả máy trước khi cài vì sau đó không còn đường nhả. Đổi chỗ bất kỳ cặp nào **vẫn compile
và vẫn chạy đúng trên máy không cắm điện thoại nào** — đúng lý do phải ghim bằng test
(`the_updater_releases_the_fleet_between_downloading_and_installing`).

**Không dùng `on_before_exit` của plugin, có lý do:** callback đó chạy trong async runtime,
còn `graceful_shutdown` gọi `block_on` — sẽ panic ngay trong lúc tắt. Vì vậy gọi thẳng, từ
một OS thread thường (không `spawn_blocking`, cùng lý do runtime).

**Và không giữ admission gate, không phải vì nó read-only:** giữ `CommandAdmission` sẽ
**deadlock**, vì `graceful_shutdown` chờ các lệnh mutating drain, mà lệnh này chính là một
trong số đó — nó sẽ chờ chính mình.

**`busy_reason` sửa hai chỗ.** Nó ghép chuỗi bị lỗi xuống dòng thành một dãy 18 dấu cách
giữa câu — chuỗi này hiện cho người vận hành đọc. Và nó chỉ đếm nurture; giờ đếm thêm hàng
đợi việc, **và hàng đợi không đọc được thì tính là đang chạy**: giá của một câu "rảnh" sai
là cắt phiên đang chạy để đổi binary, giá của một câu "đang chạy" sai là người ta hỏi lại.
Flow run vẫn chưa đếm — `FlowRuntime` không có API hỏi liveness, và bịa một cái ở đây là
tạo nguồn sự thật thứ hai. Ghi ra chứ không ngụ ý là đã đủ.

### 9.40 M4 ĐẠT — caption đọc được nguyên văn, và ngưỡng cắt ở đâu (13/08/2026)

**Đo được mà không đăng gì.** Lần trước tôi kết luận "chưa đo được M4" vì hai bài của tài khoản
trên Redmi không có caption — nhưng tôi **chỉ kiểm Redmi**. Tài khoản trên Note 8
(`Hoàng Hồng Nam`, `@user19257731814158`) có **nhiều bài carousel ảnh và đều có caption**. Bài học
nhỏ: "chưa đo được" phải kiểm hết máy đang có trước khi nói ra.

Tap `ProfileTab` (nhãn vừa đo) trên Note 8 ở (972, 2029) — **lần đo thứ hai của nhãn đó, trên máy
thứ hai**, cùng chuỗi `Hồ sơ`. Rồi mở một bài và dump cây qua agent.

| caption | trạng thái trên trang bài |
|---|---|
| 39 ký tự | **nguyên văn** — `probe --measure-own-post` tự báo `VERBATIM` |
| 49 ký tự (`Mới đi Đà Lạt về, chi phí thực tế đây ạ 👇`) | **nguyên văn**, không dấu cắt |
| 116 ký tự (`Nếu chỉ có 3 ngày ở Đà Lạt, …`) | **bị cắt**, kết thúc bằng `…` |

**Kết luận: luật của người vận hành hiện thực được.** Thiết kế cần prefix **≥ 24 ký tự**; đo được
là **nguyên văn tới ít nhất 49**, và prefix đọc được **~115** ký tự cả khi bị cắt. Dư rất nhiều.
Ngưỡng cắt nằm giữa 49 và ~116, **chưa ghim chính xác** — không cần, vì cả hai đầu đều trên 24.

**Chi tiết cho lúc hiện thực `captionProof`:** dấu cắt là **một ký tự `…`** (U+2026) ở cuối, không
phải link `thêm`. Nên so khớp nên bỏ `…` cuối rồi so như prefix, và hạ `captionProof="prefix"`.

`Follow control on this page: absent` — dấu hiệu bài-của-mình xác nhận lại lần nữa.

**Điều này làm kết luận về xoá sắc hơn, không đổi nó.** Mục 9.37 nói xoá tự động không dựng được
vì **nút xoá không có nhãn**. Giờ nửa còn lại cũng có bằng chứng dương: **caption hoàn toàn đủ làm
bằng chứng**. Tức chuỗi P0–P5 dựng được; chỉ P6 (mở sheet, tap Xoá) là không. Nếu sau này tìm ra
lối xoá khác (long-press grid, hoặc sau `Cài đặt quyền riêng tư`) thì phần chứng minh đã sẵn sàng.

### 9.39 Auto-update: khoá, và hai chỗ cố ý KHÔNG tự động (13/08/2026)

**Khoá.** Sinh bằng `tauri signer generate` **ngoài repo**, có mật khẩu:

| thứ | ở đâu |
|---|---|
| private key + mật khẩu | `C:\Users\cattfan\Documents\riviu-updater-key\` — **ngoài** repo, `~/.riviu`, scratchpad |
| private key (dùng cho CI) | GitHub secret `TAURI_SIGNING_PRIVATE_KEY` |
| mật khẩu (dùng cho CI) | GitHub secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |
| **public** key | commit trong `tauri.conf.json` — không mật, và **phải** commit |

Có mật khẩu chứ không để trống: một secret GitHub bị lộ khi đó vẫn chưa phải một khoá dùng
được. Đổi lại thư mục backup chứa cả hai, tức nó tự đủ để phục hồi — **và tự đủ để mất**. Đó là
đánh đổi có ý thức, chọn hướng "phục hồi được" vì hậu quả bên kia nặng hơn nhiều:

> **Mất private key = mọi bản đã cài không bao giờ update được nữa.** Pubkey nướng vào từng
> binary đã ship, nên không có đường phát hành lại bằng khoá khác. Backup thư mục đó offline.

**Hai chỗ cố ý không tự động.**

1. **Không kiểm lúc mở app.** Máy farm thường offline và không ai yêu cầu nó gọi mạng. Kiểm là
   hành động của người vận hành, qua command `update_check`.
2. **Không bao giờ tự cài.** `update_check` **báo cáo**, không cài. Và nó trả về **hai** câu trả
   lời trong một lần gọi: có bản mới không, **và** giờ có phải lúc an toàn không. Một updater
   nói "có bản mới" mà không nói "anh đang có 16 máy giữa phiên" là mời người ta cập nhật vào
   đúng lúc tệ nhất — cài đặt thay thế binary đang chạy, mà tiến trình đó đang giữ WDA relay,
   XCTest runner và lease của các máy, những thứ chỉ chính nó khi shutdown mới nhả.

`busy_reason` được hỏi **trước** lời gọi mạng, để một fleet đang chạy vẫn được báo dù GitHub
không tới được — "đừng cập nhật lúc này" là nửa gấp hơn của câu trả lời. Nó trả về **một câu**
chứ không phải bool, để người vận hành biết mình sẽ cắt cái gì.

**Một khoảng trống đã nêu tên chứ không giả vờ đầy đủ:** `busy_reason` hiện chỉ hỏi phiên nurture
— đó là việc dài hạn duy nhất kiểm được liveness mà không chạm máy. Flow run và job run cũng
gây gián đoạn y như thế. Nên "rảnh" ở đây nghĩa là "không có phiên nurture", **không** phải
"không có gì cả".

**Ràng buộc bất biến của release vẫn nguyên.** `createUpdaterArtifacts: true` làm bundler ký và
ghi `.sig` cạnh archive; thiếu secret thì **build đổ** chứ không ship artifact không ký — đúng
hướng, vì bản đã cài chỉ nhận update có chữ ký khớp pubkey nướng trong nó, nên một release
không ký đơn giản là không ai lấy được. Endpoint là
`/releases/latest/download/latest.json`, được GitHub phân giải **lúc request**, nên phát hành
bản mới tự trỏ lại mà **không chạm byte nào** của release cũ — gate "không ghi đè release đã
có" ở workflow giữ nguyên.

**Chưa làm:** sinh và upload `latest.json` trong cùng `gh release create`, và UI để bấm. Nên
hiện tại chuỗi này **chưa chạy được đầu-cuối**; phần đã có là khoá, ký lúc build, và command
báo cáo.

### 9.38 CI đỏ bốn lần vì đúng cái bẫy tôi đã tự cảnh báo (13/08/2026)

`cargo test` local 872/0, CI đỏ **bốn lần liên tiếp** ở `Test Rust workspace`. Một test:
`resolve_falls_back_to_the_bare_name_when_nothing_on_disk_matches`.

**Runner Windows của GitHub có `ANDROID_HOME`** trỏ vào một SDK thật, nên
`AdbProgram::resolve` tìm thấy `<ANDROID_HOME>/platform-tools/adb.exe` và trả về nó, chứ không
trả về bare name như test đòi. Máy này `ANDROID_HOME` rỗng nên test xanh.

Điều đáng ghi không phải cái bug, mà là: **tôi đã viết cảnh báo này ra rồi và chỉ áp cho một
trong hai test cùng phụ thuộc.** Test bên cạnh có nguyên comment "GitHub's windows images set
`ANDROID_HOME`", còn test này thì không — cùng một lượt sửa, bỏ sót một nửa.

**Sửa:** phát biểu lại tính chất sao cho môi trường không quyết được nó. Điều thật sự cần là
"một đường dẫn không tồn tại thì **không bao giờ** được trả về, vì không spawn được", nên:
`assert_ne!(resolved, missing)` cộng "kết quả phải là file có thật **hoặc** bare name". Đúng
với mọi giá trị của `ANDROID_HOME`.

**Cách kiểm từ giờ:** chạy gate với biến đó đặt sẵn, tức tái tạo môi trường CI ở local:

```powershell
$env:ANDROID_HOME = "C:\Users\cattfan\AppData\Local\Android"
cargo test --workspace --locked -- --test-threads=1
```

Xanh cả khi có và khi không có biến đó thì mới là env-independent. Bài học chung: một test đọc
`std::env` là một test mà **môi trường** quyết định kết quả, và máy dev không phải môi trường
CI. `gh run view <id> --log-failed` là cách nhanh nhất để thấy — tôi đã push bốn lần trước khi
nghĩ tới việc xem CI.

### 9.37 Trang bài của mình: hai dấu hiệu dương, và KHÔNG có nút xoá nào có nhãn (13/08/2026)

Đo được **mà không đăng gì**: tài khoản trên Redmi (`@cattfan239`, `Mítt zới còiii`) đã có hai
bài sẵn, nên chỉ cần tap `ProfileTab` vừa đo rồi mở một bài. Dump qua **agent** (không dùng
`uiautomator dump` — nó đòi idle và grid hồ sơ có thumbnail động, lại giết agent theo §9.21).

**Hai dấu hiệu *dương* của bài-của-mình**, tốt hơn hẳn việc chỉ dựa vào sự vắng mặt của
`Follow`:

| dấu hiệu | bài của mình | bài người khác |
|---|---|---|
| `Cài đặt quyền riêng tư` (text) | **có** | không |
| `… lượt xem` (text) | **có** | không |
| `Follow ` (desc) | **không** | có |

Kế hoạch chỉ có "không có `Follow ` trên rail" — một sự vắng mặt. Hai cái trên là sự *hiện
diện*, và một chuỗi bằng chứng dựa vào hiện diện thì mạnh hơn.

**Và đây là kết quả dứt khoát cho đường xoá.** Toàn bộ inventory của trang bài đó:

- `content-desc`: `Âm gốc của …`, `Chia sẻ video. …`, `Đọc hoặc viết bình luận. …`,
  `Hồ sơ Mítt zới còiii`, `Phát`, `Quay lại`,
  **`Thêm hoặc xóa video này khỏi mục Yêu thích.`**, `Thích`, `Thích video. 20 lượt thích`,
  `Thịnh hành`, `Tìm kiếm`, `Video`
- `text`: `· 05-31`, `0`, `20`, `410 lượt xem`, `Bóc tem`, `Cài đặt quyền riêng tư`,
  `Dùng thử mẫu trên TikTok`, `Mítt zới còiii`, `Tìm kiếm`, `Tìm nội dung liên quan`

Hai kết luận:

1. **Không có control xoá nào có nhãn.** Cụm `...` thấy rõ trong ảnh chụp **không có
   `content-desc` và không có `text`**. Nên `PostDeleteMenu` không định vị được bằng cơ chế
   của catalogue này; muốn tới nó phải dùng **toạ độ**, mà toạ độ cho một cú tap không lùi
   được trên màn chưa calibrate là đúng thứ project này từ chối bịa (mục 10).
2. **Mồi bẫy đã được xác nhận, không còn là dự đoán.** Chuỗi duy nhất chứa `xóa` trên cả
   trang, ở cả hai thuộc tính, là `Thêm hoặc xóa video này khỏi mục Yêu thích.` — nút **Yêu
   thích**. `Contains("xóa")` sẽ tap vào đó.

Và bẫy `Contains("Hồ sơ")` xuất hiện **lần thứ hai**: trang này có `Hồ sơ Mítt zới còiii`.

**Hệ quả cho quyết định của người vận hành.** Kế hoạch đã nêu khả năng "từ chối xoá tự động,
giữ tay" nếu caption không đọc được. Số đo này đưa tới cùng kết luận **bằng một đường khác và
sớm hơn**: chưa cần biết caption có bị cắt không, vì **nút xoá còn chưa định vị được bằng
nhãn**. Xoá tự động trên build này chỉ làm được nếu chấp nhận toạ độ — tôi không đề nghị điều đó.

**Đã đóng nốt câu hỏi "sheet mở ra sau khi tap `...` có nhãn không".** Tap thẳng vào cụm ba
chấm (996, 1754) — nó mở sheet **`Gửi đến`**, tức là **chia sẻ**, không phải tuỳ chọn bài. Dump
qua agent, inventory đầy đủ (hàng thứ hai cuộn ngang nên mắt không thấy hết, cây thì thấy):

`Chiếu`, `Facebook`, `Ghim`, `Gửi đến`, `Instagram Direct`, `Messenger`,
`Mời bạn bè trò chuyện`, `Phân tích`, `Sao chép Liên kết`, `SMS`, `Tải về`,
`Tăng lượt xem`, `Tạo nhóm`, `Zalo`, cùng vài tên liên hệ.

**Không có mục xoá nào.** Nên trên build này, **hành động xoá không tới được từ trang bài của
mình** — không có trigger có nhãn, và cái sheet mà `...` mở ra cũng không chứa nó. Xoá nằm ở
chỗ khác: có thể long-press trong grid hồ sơ, hoặc sau `Cài đặt quyền riêng tư`. **Chưa đo.**

**Kết luận cho quyết định của người vận hành, bằng số đo:** xoá tự động **không dựng được từ
trang bài** trên `com.ss.android.ugc.trill` 46.3.3. Câu trả lời trung thực là **từ chối xoá tự
động, giữ tay** — đúng phương án dự phòng kế hoạch đã nêu, nhưng tới bằng một đường ngắn hơn:
không phải vì caption bị cắt, mà vì **nút xoá không có ở đó**.

Vẫn còn một điều ngỏ: bản dump trang bài lấy lúc share sheet che phần trên, và chỉ trên **một**
build. Và hai lối xoá chưa thử (long-press grid, `Cài đặt quyền riêng tư`) là việc đo tiếp nếu
người vận hành muốn theo đuổi xoá tự động.

**Chưa đo được M4** (caption nguyên văn) vì hai bài sẵn có của tài khoản **không có caption**.
Muốn trả lời M4 phải có một bài có caption.

#### `ProfileTab` đã đo, và cái bẫy hoá ra nằm ngay trên cùng một màn

Redmi Note 12, `com.ss.android.ugc.trill`, UI tiếng Việt, 13/08/2026 — thanh tab dưới ở
y=2135: `Trang chủ`, `Cửa hàng`, `Quay`, `Hộp thư`, **`Hồ sơ`**. (`Quay` khớp lại số đo
`ComposerOpen` đã có.) Nên `ProfileTab = Exact("Hồ sơ")`, một trong sáu nhãn hết còn từ chối.

**Và bẫy `Contains` không còn là dự đoán.** Cùng **một** bản dump đó chứa cả hai:

```
content-desc="Hồ sơ"            <- tab của mình, thanh dưới
content-desc="Hồ sơ Ánh đây"    <- link hồ sơ TÁC GIẢ, trên action rail
```

Chuỗi nguy hiểm là **tiền tố** của chuỗi an toàn, nên chỉ tính exact mới tách được. Đúng hình
dạng của `Follow ` / `Đã follow` (mục 9.5): trên build này, khoảng trắng hoặc tính exact là
toàn bộ khác biệt giữa "nút của tác giả" và "thứ ta muốn bấm". Ghim bởi
`the_profile_tab_cannot_match_the_author_profile_link`.

**Cách đo:** `probe -- <serial> --measure-tab-bar` (chỉ đọc) rồi đọc `target/tab-bar.xml`.

#### Probe KHÔNG treo — lần đó là cargo đang build

Tôi đã báo sai một lần: `cargo run -q --example probe` "treo" 600s. Nó không treo, nó đang
**compile** example đó lần đầu. Build riêng trước rồi chạy binary thì probe chạy trọn vòng:
`screenshot_png` 1622 ms, minicap **29,2 FPS** trên Redmi, `inspect_app_process` ok.

```powershell
cargo build -q -p riviu-android-driver --example probe
.\target\debug\examples\probe.exe <serial> --measure-tab-bar
```

#### Hai biến môi trường probe cần, và lỗi nó báo khi thiếu

Đã mất một lượt vì cái này, ghi lại. `probe` **không** dùng đường resolve adb của app và
**không** tự tìm package TikTok theo máy:

```powershell
$env:RIVIU_ADB_PATH      = "<...>\platform-tools\adb.exe"   # thiếu -> "program not found"
$env:RIVIU_TIKTOK_PACKAGE = "com.ss.android.ugc.trill"      # thiếu -> nhắm musically
cargo run -q -p riviu-android-driver --example probe -- <serial> --measure-tab-bar
```

Thiếu cái đầu thì lỗi là `program not found` **sau** dòng
`launch_app(tiktok) FAILED: run adb -s … monkey …`, đọc như lỗi máy chứ không như lỗi PATH.
Thiếu cái thứ hai thì nó in `target package: com.zhiliaoapp.musically` rồi fail launch trên
máy SEA — và dòng `target package` là chỗ duy nhất nói ra điều đó, nên phải đọc nó.

`AdbProgram::resolve` mới (mục 9.28) nhận `bundled` làm ứng viên cuối, nhưng probe gọi
`resolve(None, None)` nên không dùng adb đóng gói. Chưa sửa: probe là công cụ của người phát
triển, và chỗ cần đúng thứ tự ứng viên là app.

### 9.35 Tách `graceful_shutdown`, và một lo ngại bị nói quá (13/08/2026)

Thân của handler `RunEvent::Exit` tách thành `graceful_shutdown(handle)` và được gọi từ **cả**
`RunEvent::Exit` **lẫn** `RunEvent::ExitRequested`. Lý do: updater thoát app để trao cho
installer, và một lần thoát bằng `process::exit` **không** phát `Exit` — nên toàn bộ chuỗi
(`reject_new_work` → drain command → `shutdown_cleanup`) sẽ không chạy. Gọi hai lần ở một lần
thoát bình thường là vô hại vì mọi bước đều idempotent, và dọn hai lần tốt hơn bỏ qua một lần.

Test `exit_order_...` **chuyển** sang nhắm vào hàm đã tách (nó scan vùng literal, nên tách hàm là
phá nó). Thêm `every_exit_path_runs_the_graceful_shutdown` khẳng định có call site **và** không
có `process::exit(` / `process::abort(` nào trong thân production — vì đó là lỗi rò im lặng quay
lại. Chi tiết đáng ghi: bản đầu của test tự bắt chính mình, vì **doc comment giải thích vì sao
cấm `process::exit`** cũng chứa chuỗi đó. Phải tìm dạng có dấu mở ngoặc; văn xuôi không viết thế.

**Lo ngại "rò adb forward" trong kế hoạch là nói quá.** Đo sau khi WM_CLOSE: không còn tiến trình
`riviu-pmd` nào (WDA relay và XCTest runner sạch — đó là rò thật mà việc tách này ngăn), nhưng
`adb forward --list` **vẫn còn hai forward**. Đó là **cố ý**, không phải rò: §9.18 đã ghi forward
tồn tại trong adb server chính là thứ làm lần mở app kế tiếp lên `● Live` ngay, và `agent_ready`
dựa vào `HashSet` trong process chứ không dựa vào adb server. Đừng "sửa".

### 9.33 Ba lỗi làm chế độ AI không debug được (13/08/2026)

Chiến dịch `a6abbe41`: chế độ **AI viết**, Riêng lẻ, cùng hai link đã chứng minh mở được. Chết
**ngay lập tức**: `1p/0` đứng ở `preparing`, ba assignment còn lại `queued`, campaign `failed`.
Không assignment nào có `error_code` riêng.

**Ba việc phải sửa, tất cả cùng một họ với §9.30.**

1. **Còn một dấu `?` nữa của đúng loại đã sửa.** Bản sửa A2 chỉ bọc khối thu frame; lời gọi AI
   trong vòng chuẩn bị vẫn `?` trên thân hàm trả `anyhow::Result<()>`, nên **một** lần AI lỗi ở
   assignment đầu tiên giết cả chiến dịch và để ba assignment kia `queued` không lý do. Đây là
   cái `?` mà kế hoạch patch đã cảnh báo là "có `?` thứ hai cùng loại".
2. **Chuỗi nguyên nhân bị bỏ.** `error_code` chỉ có `AI chuẩn bị assignment 0` — đó là lớp
   `.with_context()` ngoài cùng. Handler dùng `error.to_string()` chứ không `{:#}`, nên nguyên
   nhân thật (HTTP status, body, timeout) **mất hẳn**. Log cũng không có dòng nào: đường AI không
   log lỗi ở đâu cả. Kết quả là một lần fail live mà **không thể chẩn đoán từ bằng chứng nó để
   lại** — đúng loại lỗi mục 9 tồn tại để tránh.
3. **Và `error_code` của campaign vẫn không hiện trên UI**, nên người vận hành thấy "Lỗi" và
   không có gì khác. Ba thứ này cộng lại làm chế độ AI không debug được từ ghế người vận hành.

**Đã sửa cả ba** và nghiệm thu ở §9.34: lỗi AI giờ đánh `Failed` cho đúng assignment rồi
`continue`; `error_code` giữ cả chuỗi bằng `{:#}` và có `log::error!`; `error_code` của campaign
**và** của assignment đều được select và render (`InteractionCampaignSummary.error_code` mới, và
`assignment.errorCode` thì đã có trong type từ trước mà chưa ai render).

Một lỗi tôi tự tạo trong lúc sửa, và test bắt được: chèn `c.error_code` vào giữa hai câu SELECT
làm **mọi index của ba subquery đếm dịch một chỗ**, nên `target_count` đọc vào cột lý do. Ghim
bởi `a_campaign_summary_carries_the_error_code_that_ended_it`, và test đó kiểm **cả hai** đường
đọc (list và detail) vì chúng là hai câu SELECT riêng.

Suy đoán ban đầu của tôi — `model: "deepseek-v4-flash"` không phải id hợp lệ — **sai**. Xem
§9.34: API gọi được, và cái chặn là gate chất lượng của chính chúng ta.

**Ghi thêm, không liên quan tới H6-d:** `apiKey` của AI nằm **plaintext** trong bảng `settings`
(`nurture.settings` là một chuỗi JSON). Không phải lỗi do đợt này gây ra, và không nằm trong
phạm vi đã thống nhất — ghi lại để đừng ai phải phát hiện lại.
